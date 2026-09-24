use std::{collections::BTreeMap, fs, path::Path};

use kuru_core::{
    AuthorityClaimCategory, Config, ConfigSnapshot, InvocationOverrides, McpConfig, Mode,
    ModelPreference, ProjectPreferences, SelectionOverrides, load_instructions,
};
use tempfile::TempDir;

fn write(path: impl AsRef<Path>, text: impl AsRef<[u8]>) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn load_text(text: &str) -> anyhow::Result<Config> {
    let dir = TempDir::new().unwrap();
    write(dir.path().join(".kuru/config.toml"), text);
    Config::load(None, dir.path(), None)
}

fn escaped_source(path: &Path) -> String {
    path.canonicalize()
        .unwrap()
        .as_os_str()
        .to_string_lossy()
        .chars()
        .flat_map(char::escape_default)
        .take(160)
        .collect()
}

#[test]
fn defaults_are_usable_and_preserve_explicit_permission_boundaries() {
    let dir = TempDir::new().unwrap();
    let config = Config::load(None, dir.path(), None).unwrap();
    assert_eq!(config, Config::default());
    config.validate().unwrap();
    assert_eq!(config.provider, "codex");
    assert_eq!(config.model, "auto");
    assert!(!config.allow_shell);
    assert!(!config.allow_write);
    assert_eq!(config.max_rounds, 3);
    assert_eq!(config.max_parallel, 4);
    assert_eq!(config.max_tool_calls, 12);
    assert_eq!(config.dream_every, 8);
    assert!(config.dream_on_exit);
    assert!(config.effort.is_none());
}

#[test]
fn managed_defaults_and_exact_locks_cover_budget_rules_and_alias_tables() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    let managed = dir.path().join("managed.toml");
    write(
        &managed,
        "[defaults]\nmodel='managed-default'\n[constraints]\nmax_tool_calls=4\npermissions=[]\n[constraints.mcp.service]\ncommand='fixed-runner'",
    );
    write(project.join(".kuru/config.toml"), "model='project-model'");
    let snapshot = ConfigSnapshot::parse_with_layers(
        None,
        &project,
        None,
        None,
        Some(&managed),
        InvocationOverrides {
            typed_config: vec![
                "max_tool_calls=4".into(),
                "mcp.service={command='fixed-runner'}".into(),
            ],
            ..InvocationOverrides::default()
        },
    )
    .unwrap();
    let config = snapshot.finalize(&ProjectPreferences::default()).unwrap();
    assert_eq!(config.model, "project-model");
    assert_eq!(config.max_tool_calls, 4);
    assert_eq!(
        config.mcp["service"].command.as_deref(),
        Some("fixed-runner")
    );
    for assignment in [
        "max_tool_calls=5",
        "mcp.service={command='different-runner'}",
        "permissions=[{action='deny',selector={kind='native',name='shell'}}]",
    ] {
        let failure = ConfigSnapshot::parse_with_layers(
            None,
            &project,
            None,
            None,
            Some(&managed),
            InvocationOverrides {
                typed_config: vec![assignment.into()],
                ..InvocationOverrides::default()
            },
        );
        assert!(failure.is_err(), "{assignment}");
        assert!(
            !failure
                .unwrap_err()
                .to_string()
                .contains("different-runner")
        );
    }
}

#[test]
fn managed_locks_reject_project_local_and_saved_conflicts() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    let managed = dir.path().join("managed.toml");
    write(
        &managed,
        "[defaults.mcp.named_service]\ncommand='fixed-runner'\n[constraints]\nallow_write=false\nmode='ifs'\n[constraints.mcp.named_service]\ncommand='fixed-runner'",
    );
    write(project.join(".kuru/config.toml"), "allow_write=true");
    assert!(
        ConfigSnapshot::parse_with_layers(
            None,
            &project,
            None,
            None,
            Some(&managed),
            InvocationOverrides::default(),
        )
        .is_err()
    );
    write(
        project.join(".kuru/config.toml"),
        "allow_write=false\n[mcp.named_service]\nargs=['new']",
    );
    assert!(
        ConfigSnapshot::parse_with_layers(
            None,
            &project,
            None,
            None,
            Some(&managed),
            InvocationOverrides::default(),
        )
        .is_err()
    );
    write(project.join(".kuru/config.toml"), "allow_write=false");
    let local = project.join(".kuru/config.local.toml");
    let local_text = "allow_write=true";
    write(&local, local_text);
    assert!(
        ConfigSnapshot::parse_with_layers(
            None,
            &project,
            Some((&local, local_text)),
            None,
            Some(&managed),
            InvocationOverrides::default(),
        )
        .is_err()
    );
    let snapshot = ConfigSnapshot::parse_with_layers(
        None,
        &project,
        None,
        None,
        Some(&managed),
        InvocationOverrides::default(),
    )
    .unwrap();
    assert!(
        snapshot
            .finalize(&ProjectPreferences {
                mode: Some(Mode::Freudian),
                ..ProjectPreferences::default()
            })
            .is_err()
    );
    snapshot.finalize(&ProjectPreferences::default()).unwrap();

    write(&managed, "[constraints.mcp]");
    write(
        project.join(".kuru/config.toml"),
        "[mcp.worker]\ncommand='repo-runner'",
    );
    assert!(
        ConfigSnapshot::parse_with_layers(
            None,
            &project,
            None,
            None,
            Some(&managed),
            InvocationOverrides::default(),
        )
        .is_err()
    );

    write(
        &managed,
        "[constraints.external_agents]\nprimary='https://example.test/primary'",
    );
    write(
        project.join(".kuru/config.toml"),
        "[external_agents]\nprimary='https://example.test/primary'\nsecondary='https://example.test/secondary'",
    );
    assert!(
        ConfigSnapshot::parse_with_layers(
            None,
            &project,
            None,
            None,
            Some(&managed),
            InvocationOverrides::default(),
        )
        .is_ok()
    );
    write(
        project.join(".kuru/config.toml"),
        "[external_agents]\nprimary='https://example.test/changed'",
    );
    assert!(
        ConfigSnapshot::parse_with_layers(
            None,
            &project,
            None,
            None,
            Some(&managed),
            InvocationOverrides::default(),
        )
        .is_err()
    );
}

#[test]
fn local_and_typed_overrides_keep_remaining_automatic_claims() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    write(
        project.join(".kuru/config.toml"),
        "allow_shell=true\n[mcp.worker]\ncommand='repo-runner'",
    );
    let local = project.join(".kuru/config.local.toml");
    write(&local, "allow_shell=false\nmode='freudian'");
    let snapshot = ConfigSnapshot::parse_with_layers(
        None,
        &project,
        Some((&local, "allow_shell=false\nmode='freudian'")),
        None,
        None,
        InvocationOverrides {
            typed_config: vec!["max_rounds=4".into()],
            ..InvocationOverrides::default()
        },
    )
    .unwrap();
    let claims = snapshot.manifest().claims();
    assert!(
        claims
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::McpStdio)
    );
    assert!(
        !claims
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::Shell)
    );
    let config = snapshot.finalize(&ProjectPreferences::default()).unwrap();
    assert_eq!(config.mode, Mode::Freudian);
    assert_eq!(config.max_rounds, 4);
    assert!(!config.allow_shell);

    let dedicated = ConfigSnapshot::parse_with_layers(
        None,
        &project,
        Some((&local, "allow_shell=false\nmode='freudian'")),
        None,
        None,
        InvocationOverrides {
            typed_config: vec!["allow_shell=false".into()],
            allow_shell: true,
            ..InvocationOverrides::default()
        },
    )
    .unwrap();
    assert!(
        dedicated
            .finalize(&ProjectPreferences::default())
            .unwrap()
            .allow_shell
    );
    assert!(
        !dedicated
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::Shell)
    );
}

#[test]
fn snapshot_keeps_only_effective_ancestor_authority_and_redacts_inspection() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    write(
        dir.path().join(".kuru/config.toml"),
        "provider='responses'\napi_base='https://api.example.test/v1'\napi_key_env='PENDING_SECRET'\nallow_shell=true\n[mcp.build]\ncommand='runner'\nargs=['--public']\n[mcp.build.env]\nTOKEN='mcp-env-secret'",
    );
    let snapshot =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(
        snapshot
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::Shell)
    );
    assert!(
        snapshot
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::McpStdio)
    );
    assert!(
        snapshot
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::ResponsesRoute)
    );
    let preview = snapshot.snapshot_toml().unwrap();
    assert!(preview.contains("[redacted]"));
    assert!(!preview.contains("mcp-env-secret"));
    assert_eq!(
        snapshot.responses_route().unwrap().unwrap().api_key_env(),
        "PENDING_SECRET"
    );

    let overridden = ConfigSnapshot::parse(
        None,
        &project,
        None,
        InvocationOverrides {
            provider: Some("demo".into()),
            ..InvocationOverrides::default()
        },
    )
    .unwrap();
    assert!(
        !overridden
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::ResponsesRoute)
    );
}

#[test]
fn snapshot_sanitizes_parser_text_but_keeps_position_and_defers_framework_validation() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join(".kuru/config.toml");
    write(&config, "# secret=do-not-print\n[\n");
    let error = ConfigSnapshot::parse(None, dir.path(), None, InvocationOverrides::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("parse error"));
    assert!(error.contains("line 2"));
    assert!(!error.contains("do-not-print"));
    fs::write(&config, "max_parts=3").unwrap();
    let snapshot =
        ConfigSnapshot::parse(None, dir.path(), None, InvocationOverrides::default()).unwrap();
    assert!(snapshot.snapshot_toml().unwrap().contains("max_parts = 3"));
    assert!(snapshot.finalize(&ProjectPreferences::default()).is_err());
    assert!(
        snapshot
            .finalize(&ProjectPreferences {
                mode: Some(Mode::Freudian),
                ..ProjectPreferences::default()
            })
            .is_ok()
    );
}

#[test]
fn snapshot_freezes_files_and_keeps_cli_and_remaining_ancestor_claims_distinct() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let config = dir.path().join(".kuru/config.toml");
    write(
        &config,
        "allow_write=true\nallow_shell=true\n[mcp.build]\ncommand='ancestor-runner'\nargs=['--inherited']\n[mcp.build.env]\nTOKEN='inherited-secret'",
    );
    let overrides = InvocationOverrides {
        allow_write: true,
        allow_shell: true,
        ..InvocationOverrides::default()
    };
    let snapshot = ConfigSnapshot::parse(None, &project, None, overrides).unwrap();
    let categories = snapshot
        .manifest()
        .claims()
        .iter()
        .map(|claim| claim.category())
        .collect::<Vec<_>>();
    assert!(!categories.contains(&AuthorityClaimCategory::WorkspaceWrite));
    assert!(!categories.contains(&AuthorityClaimCategory::Shell));
    assert!(categories.contains(&AuthorityClaimCategory::McpStdio));
    let digest = snapshot.manifest().full_digest();
    write(
        &config,
        "allow_write=true\nallow_shell=true\n[mcp.build]\ncommand='changed-runner'\nargs=['--changed']\n[mcp.build.env]\nTOKEN='changed-secret'",
    );
    assert_eq!(snapshot.manifest().full_digest(), digest);
    assert_eq!(
        snapshot
            .finalize(&ProjectPreferences::default())
            .unwrap()
            .mcp["build"]
            .command
            .as_deref(),
        Some("ancestor-runner")
    );
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(changed.manifest().full_digest(), digest);
    write(
        &config,
        "allow_write=true\nallow_shell=true\n[memory]\nstartup_timeout_secs=99\n[mcp.build]\ncommand='changed-runner'\nargs=['--changed']\n[mcp.build.env]\nTOKEN='changed-secret'",
    );
    let ordinary_change =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_eq!(
        ordinary_change.manifest().full_digest(),
        changed.manifest().full_digest()
    );
}

#[test]
fn manifest_binds_active_transports_and_safely_reports_every_automatic_source() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("parent");
    let project = parent.join("project");
    write(
        parent.join(".kuru/config.toml"),
        "[mcp.shared]\ncommand='outer-runner'",
    );
    write(
        project.join(".kuru/config.toml"),
        "[mcp.shared.env]\nTOKEN='inherited'\n[mcp.\"bad\\u001bname\"]\nurl='https://first.example.test/mcp'",
    );
    let first =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_eq!(first.manifest().sources().len(), 2);
    assert_eq!(
        first
            .manifest()
            .claims()
            .iter()
            .find(|claim| claim.category() == AuthorityClaimCategory::McpStdio)
            .unwrap()
            .sources()
            .len(),
        2
    );
    assert!(
        first
            .manifest()
            .sources()
            .iter()
            .all(|source| !source.as_str().contains('\u{1b}'))
    );
    assert!(
        first
            .manifest()
            .claims()
            .iter()
            .all(|claim| !claim.display().as_str().contains('\u{1b}'))
    );
    let first_http = first
        .manifest()
        .claims()
        .iter()
        .find(|claim| claim.category() == AuthorityClaimCategory::McpHttp)
        .unwrap()
        .digest();
    write(
        project.join(".kuru/config.toml"),
        "[mcp.shared.env]\nTOKEN='inherited'\n[mcp.\"bad\\u001bname\"]\nurl='https://second.example.test/mcp'",
    );
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let changed_http = changed
        .manifest()
        .claims()
        .iter()
        .find(|claim| claim.category() == AuthorityClaimCategory::McpHttp)
        .unwrap()
        .digest();
    assert_ne!(first_http, changed_http);

    let ordinary = TempDir::new().unwrap();
    write(ordinary.path().join(".kuru/config.toml"), "max_rounds=4");
    assert!(
        ConfigSnapshot::parse(None, ordinary.path(), None, InvocationOverrides::default())
            .unwrap()
            .manifest()
            .sources()
            .is_empty()
    );

    let provenance = TempDir::new().unwrap();
    let provenance_project = provenance.path().join("project");
    fs::create_dir_all(&provenance_project).unwrap();
    write(
        provenance.path().join(".kuru/config.toml"),
        "allow_shell=true",
    );
    let parent_claim = ConfigSnapshot::parse(
        None,
        &provenance_project,
        None,
        InvocationOverrides::default(),
    )
    .unwrap()
    .manifest()
    .full_digest();
    fs::remove_file(provenance.path().join(".kuru/config.toml")).unwrap();
    write(
        provenance_project.join(".kuru/config.toml"),
        "allow_shell=true",
    );
    let child_claim = ConfigSnapshot::parse(
        None,
        &provenance_project,
        None,
        InvocationOverrides::default(),
    )
    .unwrap()
    .manifest()
    .full_digest();
    assert_ne!(parent_claim, child_claim);
}

#[test]
fn manifest_binds_active_mcp_catalog_policy_and_omits_disabled_authority() {
    let directory = TempDir::new().unwrap();
    let project = directory.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let config = directory.path().join(".kuru/config.toml");
    write(
        &config,
        "[mcp.remote]\nurl='https://example.test/mcp'\nallow_tools=['read_*']\nheader_env={Authorization='MCP_AUTH'}",
    );
    let first =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let first_digest = first.manifest().full_digest();
    assert!(
        first
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::McpHttp)
    );

    write(
        &config,
        "[mcp.remote]\nurl='https://example.test/mcp'\nallow_tools=['write_*']\nheader_env={Authorization='OTHER_AUTH'}",
    );
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(changed.manifest().full_digest(), first_digest);

    write(
        &config,
        concat!(
            "[mcp.remote]\n",
            "enabled=false\n",
            "url='https://resource.example.test/mcp'\n",
            "[mcp.remote.oauth]\n",
            "enabled=true\n",
            "client_id='kuru-test'\n",
            "client_secret_env='MCP_CLIENT_SECRET'\n",
        ),
    );
    let disabled =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(
        disabled
            .manifest()
            .claims()
            .iter()
            .all(|claim| claim.category() != AuthorityClaimCategory::McpHttp)
    );

    write(
        &config,
        "[mcp.remote]\nenabled=false\nurl='https://example.test/mcp'\nheader_env={Authorization='OTHER_AUTH'}",
    );
    let disabled =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(
        disabled
            .manifest()
            .claims()
            .iter()
            .all(|claim| claim.category() != AuthorityClaimCategory::McpHttp)
    );
}

#[test]
fn external_agent_claims_follow_effective_automatic_values_and_provenance() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("parent");
    let project = parent.join("project");
    let local = dir.path().join("local.toml");
    fs::create_dir_all(&project).unwrap();
    write(
        parent.join(".kuru/config.toml"),
        "[external_agents]\nreview='https://review.example.test/a2a'\nretained='https://retained.example.test/a2a'",
    );
    write(
        &local,
        "[external_agents]\nreview='https://local.example.test/a2a'",
    );

    let first = ConfigSnapshot::parse(None, &project, Some(&local), InvocationOverrides::default())
        .unwrap();
    let claims = first
        .manifest()
        .claims()
        .iter()
        .filter(|claim| claim.category() == AuthorityClaimCategory::ExternalAgent)
        .collect::<Vec<_>>();
    assert_eq!(claims.len(), 1, "the local review URL is explicit");
    assert!(claims[0].display().as_str().contains("retained"));
    assert_eq!(claims[0].sources().len(), 1);
    assert_eq!(
        claims[0].sources()[0].as_str(),
        escaped_source(&parent.join(".kuru/config.toml")),
    );
    let first_digest = first.manifest().full_digest();

    write(
        project.join(".kuru/config.toml"),
        "[external_agents]\nretained='https://retained.example.test/a2a'",
    );
    let moved = ConfigSnapshot::parse(None, &project, Some(&local), InvocationOverrides::default())
        .unwrap();
    assert_ne!(moved.manifest().full_digest(), first_digest);

    write(
        project.join(".kuru/config.toml"),
        "[external_agents]\nretained='https://changed.example.test/a2a'",
    );
    let changed =
        ConfigSnapshot::parse(None, &project, Some(&local), InvocationOverrides::default())
            .unwrap();
    assert_ne!(
        changed.manifest().full_digest(),
        moved.manifest().full_digest()
    );
}

#[test]
fn layers_respect_user_ancestor_project_local_precedence_and_merge_nested_maps() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("organization");
    let project = parent.join("project");
    let user = dir.path().join("user.toml");
    let local = dir.path().join("local.toml");
    write(
        &user,
        "mode='polyvagal'\nmax_tool_calls=33\n[mcp.files]\ncommand='files'\nargs=['one','two']\n[mcp.files.env]\nFIRST='one'\nSECOND='two'\n[external_agents]\nreviewer='http://localhost:9000/a2a'",
    );
    write(
        parent.join(".kuru/config.toml"),
        "mode='freudian'\nmax_rounds=5\n[mcp.files.env]\nSECOND='override'",
    );
    write(
        project.join(".kuru/config.toml"),
        "mode='jungian'\nmax_parallel=2\n[mcp.files]\nargs=['replacement']\n[mcp.extra]\nurl='https://example.test/mcp'",
    );
    write(
        &local,
        "mode='ifs'\nallow_write=true\neffort='future-effort'\nmodel='模型-新'\ndream_every=0",
    );
    let config = Config::load(Some(&user), &project, Some(&local)).unwrap();
    assert_eq!(config.mode, Mode::Ifs);
    assert_eq!(config.max_tool_calls, 33);
    assert_eq!(config.max_rounds, 5);
    assert_eq!(config.max_parallel, 2);
    assert_eq!(config.model, "模型-新");
    assert_eq!(config.effort.as_deref(), Some("future-effort"));
    assert_eq!(config.dream_every, 0);
    assert!(config.allow_write);
    assert!(!config.allow_shell);
    assert_eq!(config.mcp["files"].args, ["replacement"]);
    assert_eq!(config.mcp["files"].env["FIRST"], "one");
    assert_eq!(config.mcp["files"].env["SECOND"], "override");
    assert_eq!(config.mcp.len(), 2);
    assert_eq!(
        config.external_agents["reviewer"],
        "http://localhost:9000/a2a"
    );
}

#[test]
fn invalid_types_or_unknown_keys_cannot_be_hidden_by_a_later_layer() {
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("user.toml");
    let local = dir.path().join("local.toml");
    write(&user, "max_rounds='wrong'");
    write(&local, "max_rounds=4");
    let error = Config::load(Some(&user), dir.path(), Some(&local)).unwrap_err();
    assert!(format!("{error:#}").contains("user.toml"));
    write(&user, "unknown_setting=1");
    assert!(Config::load(Some(&user), dir.path(), Some(&local)).is_err());
    // Semantic bounds apply to the effective configuration after all overrides.
    write(&user, "max_rounds=0");
    assert_eq!(
        Config::load(Some(&user), dir.path(), Some(&local))
            .unwrap()
            .max_rounds,
        4
    );
}

#[test]
fn invalid_local_types_cannot_be_hidden_by_command_line_overrides() {
    let dir = TempDir::new().unwrap();
    let local = dir.path().join("local.toml");
    for (text, overrides, private_value) in [
        (
            "allow_shell='secret-shell-value'",
            InvocationOverrides {
                allow_shell: true,
                ..InvocationOverrides::default()
            },
            "secret-shell-value",
        ),
        (
            "model=3",
            InvocationOverrides {
                model: Some("command-line-model".into()),
                ..InvocationOverrides::default()
            },
            "command-line-model",
        ),
    ] {
        write(&local, text);
        let error = ConfigSnapshot::parse(None, dir.path(), Some(&local), overrides).unwrap_err();
        assert!(
            error.to_string().contains("configuration type error"),
            "{error:#}"
        );
        assert!(error.to_string().contains("local.toml"), "{error:#}");
        assert!(!error.to_string().contains(private_value), "{error:#}");
    }
}

#[test]
fn malformed_toml_unknown_keys_and_type_errors_have_actionable_context() {
    for text in [
        "[",
        "model=1",
        "max_rounds=-1",
        "mode='unknown'",
        "max_paralell=4",
        "mcp='wrong'",
        "[mcp.example]\ncommand='test'\nargz=[]",
    ] {
        let error = format!("{:#}", load_text(text).unwrap_err());
        assert!(error.contains("config.toml"), "{error}");
    }
}

#[test]
fn explicit_missing_paths_and_non_directory_projects_fail() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.toml");
    assert!(Config::load(Some(&missing), dir.path(), None).is_err());
    assert!(Config::load(None, dir.path(), Some(&missing)).is_err());
    assert!(Config::load(None, &missing, None).is_err());
    let file = dir.path().join("file");
    write(&file, "");
    assert!(Config::load(None, &file, None).is_err());
    assert!(load_instructions(&file).is_err());
    assert!(load_instructions(&missing).is_err());
}

#[test]
fn bounded_regular_utf8_files_prevent_unlimited_or_malformed_instruction_loading() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join(".kuru/config.toml");
    write(&config, [0xff, 0xfe]);
    let error = Config::load(None, dir.path(), None).unwrap_err();
    assert!(
        error.to_string().contains("configuration read error"),
        "{error:#}"
    );
    write(&config, "#".repeat(256 * 1024 + 1));
    let error = Config::load(None, dir.path(), None).unwrap_err();
    assert!(
        error.to_string().contains("configuration read error"),
        "{error:#}"
    );
    fs::remove_file(&config).unwrap();
    fs::create_dir(&config).unwrap();
    let error = Config::load(None, dir.path(), None).unwrap_err();
    assert!(
        error.to_string().contains("configuration read error"),
        "{error:#}"
    );
    fs::remove_dir(&config).unwrap();
    write(dir.path().join("AGENTS.md"), [0xff]);
    assert!(load_instructions(dir.path()).is_err());
    write(dir.path().join("AGENTS.md"), "x".repeat(256 * 1024 + 1));
    let captured =
        ConfigSnapshot::parse(None, dir.path(), None, InvocationOverrides::default()).unwrap();
    assert!(captured.instructions().contains("256 KiB file limit"));
    assert_eq!(captured.instruction_notices().len(), 1);
    assert!(!captured.instructions().contains(&"x".repeat(256)));
    let instructions = dir.path().join("AGENTS.md");
    fs::remove_file(&instructions).unwrap();
    fs::create_dir(&instructions).unwrap();
    let error = load_instructions(dir.path()).unwrap_err();
    assert!(
        format!("{error:#}").contains("instruction read error"),
        "{error:#}"
    );
}

#[test]
fn combined_limits_apply_across_many_individually_valid_files() {
    let dir = TempDir::new().unwrap();
    let mut path = dir.path().to_owned();
    for index in 0..5 {
        path.push("child");
        write(
            path.join("AGENTS.md"),
            format!("SOURCE-{index}\n{}", "x".repeat(220_000)),
        );
        write(
            path.join(".kuru/config.toml"),
            format!("#{}\n", "x".repeat(220_000)),
        );
    }
    let instructions = load_instructions(&path).unwrap();
    assert!(instructions.contains("1 MiB combined instruction limit"));
    assert!(instructions.contains("SOURCE-3"));
    assert!(!instructions.contains("SOURCE-4"));
    assert!(
        Config::load(None, &path, None)
            .unwrap_err()
            .to_string()
            .contains("configuration read error")
    );
}

#[test]
fn snapshot_claims_ordered_automatic_instructions_and_freezes_exact_bytes() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("parent");
    let project = parent.join("project");
    write(parent.join("AGENTS.md"), "OUTER-APPROVED");
    write(project.join("AGENTS.md"), "ROOT-APPROVED");

    let snapshot =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let claim = snapshot
        .manifest()
        .claims()
        .iter()
        .find(|claim| claim.category() == AuthorityClaimCategory::ProjectInstructions)
        .unwrap();
    assert_eq!(
        claim
            .sources()
            .iter()
            .map(|source| source.as_str())
            .collect::<Vec<_>>(),
        [
            escaped_source(&parent.join("AGENTS.md")),
            escaped_source(&project.join("AGENTS.md")),
        ]
    );
    assert_eq!(claim.display().as_str(), "2 ordered automatic sources");
    let instructions = snapshot.instructions();
    assert!(
        instructions.find("OUTER-APPROVED").unwrap() < instructions.find("ROOT-APPROVED").unwrap()
    );

    write(parent.join("AGENTS.md"), "OUTER-CHANGED");
    write(project.join("AGENTS.md"), "ROOT-CHANGED");
    assert!(snapshot.instructions().contains("OUTER-APPROVED"));
    assert!(snapshot.instructions().contains("ROOT-APPROVED"));
    assert!(!snapshot.instructions().contains("CHANGED"));
}

#[test]
fn claude_wrapper_and_imports_render_once_in_order_and_bind_exact_bytes() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(project.join("AGENTS.md"), "AGENT-ONLY\n");
    write(project.join("docs/rules.md"), "IMPORTED-ONLY\n");
    write(
        project.join("CLAUDE.md"),
        "CLAUDE-BEFORE\n@AGENTS.md\n@docs/../AGENTS.md\n@docs/rules.md\nCLAUDE-AFTER\n",
    );
    let captured =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let rendered = captured.instructions();
    assert_eq!(rendered.matches("AGENT-ONLY").count(), 1);
    assert_eq!(rendered.matches("IMPORTED-ONLY").count(), 1);
    assert!(!rendered.contains("@AGENTS.md"));
    assert!(!rendered.contains("@docs/../AGENTS.md"));
    assert!(rendered.find("AGENT-ONLY").unwrap() < rendered.find("CLAUDE-BEFORE").unwrap());
    assert!(rendered.find("CLAUDE-BEFORE").unwrap() < rendered.find("IMPORTED-ONLY").unwrap());
    assert!(rendered.find("IMPORTED-ONLY").unwrap() < rendered.find("CLAUDE-AFTER").unwrap());
    let claim = captured
        .manifest()
        .claims()
        .iter()
        .find(|claim| claim.category() == AuthorityClaimCategory::ProjectInstructions)
        .unwrap();
    assert_eq!(claim.sources().len(), 3);
    let reviewed = captured.manifest().full_digest();
    write(project.join("docs/rules.md"), "CHANGED-IMPORT\n");
    assert!(captured.instructions().contains("IMPORTED-ONLY"));
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(reviewed, changed.manifest().full_digest());
    assert!(changed.instructions().contains("CHANGED-IMPORT"));
    fs::rename(project.join("docs"), project.join("parked-docs")).unwrap();
    write(project.join("docs/rules.md"), "CHANGED-IMPORT\n");
    let replaced_directory =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(
        changed.manifest().full_digest(),
        replaced_directory.manifest().full_digest(),
        "containing directory identity is part of reviewed authority"
    );
}

#[test]
fn nested_instruction_union_is_path_scoped_ordered_and_keeps_captured_bytes() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(project.join("AGENTS.md"), "ROOT-ORIGINAL\n");
    write(project.join("src/AGENTS.md"), "SRC-ORIGINAL\n");
    write(project.join("tests/CLAUDE.md"), "TESTS-ORIGINAL\n");
    let base = ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(!base.instructions().contains("SRC-ORIGINAL"));
    assert!(!base.instructions().contains("TESTS-ORIGINAL"));

    let (src, changed) = base.with_nested_directories(&["src".into()]).unwrap();
    assert!(changed);
    assert!(src.instructions().contains("SRC-ORIGINAL"));
    assert!(!src.instructions().contains("TESTS-ORIGINAL"));
    assert_eq!(src.nested_instruction_source_paths().len(), 1);

    let (both, changed) = src.with_nested_directories(&["tests".into()]).unwrap();
    assert!(changed);
    let (reverse, _) = base
        .with_nested_directories(&["tests".into(), "src".into()])
        .unwrap();
    assert_eq!(both.instructions(), reverse.instructions());
    assert_eq!(
        both.manifest().full_digest(),
        reverse.manifest().full_digest()
    );
    assert_eq!(
        both.nested_instruction_source_paths(),
        reverse.nested_instruction_source_paths()
    );

    write(project.join("AGENTS.md"), "ROOT-CHANGED\n");
    write(project.join("src/AGENTS.md"), "SRC-CHANGED\n");
    write(project.join("src/CLAUDE.md"), "NEW-CLAUDE\n");
    write(project.join("src/sub/AGENTS.md"), "SUB-NEW\n");
    let (still_captured, changed) = both.with_nested_directories(&["src/sub".into()]).unwrap();
    assert!(
        changed,
        "newly appearing nested sources require a new review"
    );
    assert!(still_captured.instructions().contains("ROOT-ORIGINAL"));
    assert!(still_captured.instructions().contains("SRC-ORIGINAL"));
    assert!(still_captured.instructions().contains("SUB-NEW"));
    assert!(!still_captured.instructions().contains("ROOT-CHANGED"));
    assert!(!still_captured.instructions().contains("SRC-CHANGED"));
    assert!(still_captured.instructions().contains("NEW-CLAUDE"));
    let fresh =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let (fresh, _) = fresh.with_nested_directories(&["src/sub".into()]).unwrap();
    assert!(fresh.instructions().contains("ROOT-CHANGED"));
    assert!(fresh.instructions().contains("SRC-CHANGED"));
    assert!(fresh.instructions().contains("NEW-CLAUDE"));
    assert_ne!(
        still_captured.manifest().full_digest(),
        fresh.manifest().full_digest()
    );
}

#[test]
fn nested_sources_share_the_existing_whole_source_cap() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(
        project.join("AGENTS.md"),
        (0..4)
            .map(|index| format!("@large/{index}.md\n"))
            .collect::<String>(),
    );
    for index in 0..4 {
        write(
            project.join(format!("large/{index}.md")),
            "L".repeat(240_000),
        );
    }
    write(project.join("a/AGENTS.md"), "A".repeat(100_000));
    write(project.join("b/AGENTS.md"), "B".repeat(32_000));
    let base = ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let (nested, _) = base
        .with_nested_directories(&["b".into(), "a".into()])
        .unwrap();
    assert!(!nested.instructions().contains(&"A".repeat(1_000)));
    assert!(nested.instructions().contains(&"B".repeat(1_000)));
    assert!(
        nested
            .instruction_notices()
            .iter()
            .any(|notice| notice.contains("1 MiB combined instruction limit"))
    );
    assert_eq!(nested.nested_instruction_source_paths().len(), 1);
}

#[test]
fn replaced_active_nested_directory_cannot_reuse_reviewed_bytes() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(project.join("src/AGENTS.md"), "OLD-SOURCE\n");
    write(project.join("tests/file.txt"), "two");
    let base = ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let (used, changed) = base.with_nested_directories(&["src".into()]).unwrap();
    assert!(changed);
    fs::rename(project.join("src"), project.join("old-src")).unwrap();
    write(project.join("src/AGENTS.md"), "NEW-SOURCE\n");
    assert!(used.with_nested_directories(&["src".into()]).is_err());
    assert!(used.with_nested_directories(&["tests".into()]).is_err());
    let fresh =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let (fresh, changed) = fresh.with_nested_directories(&["src".into()]).unwrap();
    assert!(changed);
    assert!(fresh.instructions().contains("NEW-SOURCE"));
}

#[test]
fn nested_absence_does_not_accumulate_or_hide_a_later_source() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(project.join("src/file.txt"), "one");
    let base = ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let (used, changed) = base.with_nested_directories(&["src".into()]).unwrap();
    assert!(!changed);
    write(project.join("src/AGENTS.md"), "NEW-SOURCE\n");
    let (again, changed) = used.with_nested_directories(&["src".into()]).unwrap();
    assert!(changed);
    assert!(again.instructions().contains("NEW-SOURCE"));
}

#[cfg(unix)]
#[test]
fn nested_directory_links_and_traversal_are_rejected_before_source_read() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    let outside = dir.path().join("outside");
    write(outside.join("AGENTS.md"), "FAKE-OUTSIDE-NESTED-BYTES");
    fs::create_dir_all(&project).unwrap();
    symlink(&outside, project.join("linked")).unwrap();
    let base = ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let linked = base
        .with_nested_directories(&["linked".into()])
        .unwrap_err()
        .to_string();
    assert!(linked.contains("instruction path error"));
    assert!(!linked.contains("FAKE-OUTSIDE-NESTED-BYTES"));
    assert!(
        base.with_nested_directories(&["../outside".into()])
            .is_err()
    );
}

#[test]
fn import_cycle_and_fenced_example_report_without_recursive_or_accidental_import() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(project.join("AGENTS.md"), "ROOT\n@docs/one.md\n");
    write(project.join("docs/one.md"), "ONE\n@../AGENTS.md\n");
    write(project.join("after.md"), "AFTER-FENCE\n");
    write(
        project.join("CLAUDE.md"),
        "````md\n```\n@missing.md\n````\u{a0}\n@not-closed.md\n```\n````\n~~~~md\n@also-missing.md\n~~~~\n@after.md\nordinary @person.md text\n",
    );
    let captured =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_eq!(captured.instructions().matches("ROOT").count(), 1);
    assert_eq!(captured.instructions().matches("ONE").count(), 1);
    assert!(captured.instructions().contains("@missing.md"));
    assert!(captured.instructions().contains("@not-closed.md"));
    assert!(captured.instructions().contains("@also-missing.md"));
    assert!(captured.instructions().contains("AFTER-FENCE"));
    assert!(captured.instructions().contains("ordinary @person.md text"));
    assert_eq!(captured.instruction_notices().len(), 1);
    assert!(captured.instruction_notices()[0].contains("import cycle"));
}

#[test]
fn import_path_escape_and_missing_source_fail_before_activation() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(dir.path().join("private.md"), "FAKE-PRIVATE-BYTES");
    write(project.join("CLAUDE.md"), "@../private.md\n");
    let error = load_instructions(&project).unwrap_err().to_string();
    assert!(error.contains("instruction path error"));
    assert!(!error.contains("FAKE-PRIVATE-BYTES"));
    write(project.join("CLAUDE.md"), "@missing.md\n");
    assert!(
        load_instructions(&project)
            .unwrap_err()
            .to_string()
            .contains("instruction read error")
    );
}

#[cfg(unix)]
#[test]
fn imported_symlink_is_rejected_without_reading_its_target() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(dir.path().join("outside.md"), "FAKE-OUTSIDE-BYTES");
    write(project.join("CLAUDE.md"), "@linked.md\n");
    symlink(dir.path().join("outside.md"), project.join("linked.md")).unwrap();
    let error = load_instructions(&project).unwrap_err().to_string();
    assert!(error.contains("instruction read error"));
    assert!(!error.contains("FAKE-OUTSIDE-BYTES"));

    let outside = dir.path().join("outside");
    write(outside.join("file.md"), "FAKE-NESTED-OUTSIDE-BYTES");
    write(project.join("CLAUDE.md"), "@linked-dir/file.md\n");
    symlink(&outside, project.join("linked-dir")).unwrap();
    let error = load_instructions(&project).unwrap_err().to_string();
    assert!(error.contains("instruction path error"));
    assert!(!error.contains("FAKE-NESTED-OUTSIDE-BYTES"));
}

#[test]
fn duplicate_imports_do_not_consume_content_budget_twice() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    write(project.join("AGENTS.md"), "@large.md\n".repeat(8));
    write(project.join("large.md"), "L".repeat(240_000));
    let captured =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_eq!(
        captured.instructions().matches(&"L".repeat(1_000)).count(),
        240
    );
    assert_eq!(
        captured
            .instructions()
            .matches("--- resume AGENTS.md")
            .count(),
        1
    );
    assert!(captured.instruction_notices().is_empty());
}

#[test]
fn import_depth_and_graph_caps_omit_whole_branches_with_notices() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    for index in 0..9 {
        write(
            project.join(format!("depth/{index}.md")),
            format!("DEPTH-{index}\n@{}.md\n", index + 1),
        );
    }
    write(project.join("depth/9.md"), "TOO-DEEP-SENTINEL\n");
    write(project.join("AGENTS.md"), "@depth/0.md\n");
    let depth =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(!depth.instructions().contains("TOO-DEEP-SENTINEL"));
    assert!(
        depth
            .instructions()
            .contains("eight-edge import depth limit")
    );
    let imports = (0..150)
        .map(|index| format!("@graph/{index}.md\n"))
        .collect::<String>();
    write(project.join("AGENTS.md"), imports);
    for index in 0..150 {
        write(
            project.join(format!("graph/{index}.md")),
            format!("GRAPH-{index}\n"),
        );
    }
    let graph =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(graph.instructions().contains("128-source graph limit"));
    assert!(graph.instructions().contains("GRAPH-126"));
    assert!(!graph.instructions().contains("GRAPH-149"));
    assert_eq!(graph.instruction_notices().len(), 17);
    assert!(graph.instruction_notices()[16].contains("7 additional"));
}

#[test]
fn automatic_instruction_add_change_remove_and_replacement_change_manifest() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let empty =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let path = project.join("AGENTS.md");
    write(&path, "approved bytes");
    let added =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(
        empty.manifest().full_digest(),
        added.manifest().full_digest()
    );

    write(&path, "changed bytes");
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(
        added.manifest().full_digest(),
        changed.manifest().full_digest()
    );

    fs::remove_file(&path).unwrap();
    let removed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_eq!(
        empty.manifest().full_digest(),
        removed.manifest().full_digest()
    );

    write(&path, "same bytes");
    let original =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    fs::rename(&path, project.join("parked-agents")).unwrap();
    write(&path, "same bytes");
    let replaced =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(
        original.manifest().full_digest(),
        replaced.manifest().full_digest(),
        "native file replacement with identical bytes must stale approval"
    );
}

#[test]
fn instructions_preserve_scope_order_content_and_local_precedence() {
    let dir = TempDir::new().unwrap();
    assert!(load_instructions(dir.path()).unwrap().is_empty());
    let project = dir.path().join("nested");
    write(
        dir.path().join("AGENTS.md"),
        "OUTER: Use a broad naming scheme.",
    );
    write(project.join("AGENTS.md"), "INNER: 日本語の名前を使う。 🪶");
    let instructions = load_instructions(&project).unwrap();
    assert!(instructions.find("OUTER:").unwrap() < instructions.find("INNER:").unwrap());
    assert!(instructions.contains("most local applicable source takes precedence"));
    assert!(instructions.contains("higher-priority conversation instructions"));
    assert!(instructions.contains("日本語の名前を使う。 🪶"));
    // The displayed source path is bounded and escapes platform separators;
    // the source headings still identify both captured scopes.
    assert_eq!(instructions.matches("\n--- AGENTS.md: ").count(), 2);
}

#[cfg(unix)]
#[test]
fn canonical_project_ancestry_is_used_for_symlinked_checkouts() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("real/project");
    write(project.join("AGENTS.md"), "Real project instructions");
    write(project.join(".kuru/config.toml"), "mode='freudian'");
    let alias = dir.path().join("linked");
    symlink(&project, &alias).unwrap();
    assert_eq!(
        Config::load(None, &alias, None).unwrap().mode,
        Mode::Freudian
    );
    assert!(
        load_instructions(&alias)
            .unwrap()
            .contains("Real project instructions")
    );
}

#[test]
fn bounds_and_required_values_fail_without_restricting_future_model_efforts() {
    for (name, text) in [
        ("provider", "provider='unknown'"),
        ("model", "model='  '"),
        ("effort", "effort=''"),
        ("max_rounds", "max_rounds=0"),
        ("max_rounds", "max_rounds=65"),
        ("max_tool_calls", "max_tool_calls=0"),
        ("max_tool_calls", "max_tool_calls=1025"),
        ("max_parallel", "max_parallel=0"),
        ("max_parallel", "max_parallel=65"),
        ("max_parts", "max_parts=6"),
        ("max_parts", "max_parts=129"),
        ("codex_command", "codex_command=''"),
        ("api_key_env", "api_key_env='9KEY'"),
        ("api_key_env", "api_key_env='HAS-DASH'"),
        ("api_base", "api_base='ftp://example.test'"),
    ] {
        let error = load_text(text).unwrap_err();
        assert!(
            error.to_string().contains("configuration validation error"),
            "{name}: {error:#}"
        );
    }
    let valid = load_text("provider='responses'\neffort='adaptive-2028'\napi_key_env='_CUSTOM_KEY_2'\nmode='freudian'\nmax_parts=3\nmax_parallel=64\nmax_rounds=64\nmax_tool_calls=1024").unwrap();
    assert_eq!(valid.effort.as_deref(), Some("adaptive-2028"));
    assert_eq!(load_text("provider='demo'").unwrap().provider, "demo");
    for invalid in [String::new(), "x".repeat(257), "a\0b".into()] {
        assert!(
            Config {
                model: invalid,
                ..Config::default()
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn endpoints_require_http_hosts_and_do_not_expose_embedded_credentials_in_errors() {
    for value in [
        "relative",
        "file:///tmp/thing",
        "https://",
        "https://example.test/#fragment",
        "https://name:keep-this-private@example.test",
    ] {
        let config = Config {
            api_base: value.into(),
            ..Config::default()
        };
        let error = format!("{:#}", config.validate().unwrap_err());
        assert!(!error.contains("keep-this-private"));
    }
    for value in [
        "https://api.example.test/v1/",
        "http://127.0.0.1:4000/v1",
        "http://[::1]:4000/v1",
    ] {
        Config {
            api_base: value.into(),
            ..Config::default()
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn mcp_requires_one_transport_and_valid_process_arguments_and_environment() {
    for text in [
        "[mcp.a]",
        "[mcp.a]\ncommand='x'\nurl='https://example.test'",
        "[mcp.a]\ncommand=''",
        "[mcp.a]\nurl='file:///tmp/mcp'",
        "[mcp.a]\nurl='https://example.test'\nargs=['bad']",
        "[mcp.a]\nurl='https://example.test'\nenv={KEY='bad'}",
        "[mcp.'bad/name']\ncommand='x'",
        "[mcp.a]\ncommand='x'\nenv={'BAD-KEY'='value'}",
    ] {
        assert!(load_text(text).is_err(), "{text}");
    }

    let literal_secret = "KURU_LITERAL_SECRET_MUST_NOT_ECHO_8E178B";
    let error = load_text(&format!(
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\naccess_token='{literal_secret}'"
    ))
    .unwrap_err();
    assert!(!format!("{error:#}").contains(literal_secret));

    let too_many = (0..65)
        .map(|index| format!("'scope:{index}'"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(
        load_text(&format!(
            "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nscopes=[{too_many}]"
        ))
        .is_err()
    );
    let mcp = McpConfig {
        command: Some("server".into()),
        args: vec!["--safe".into()],
        env: BTreeMap::from([("_KEY2".into(), "".into())]),
        ..McpConfig::default()
    };
    let mut config = Config {
        mcp: BTreeMap::from([("server-1_2".into(), mcp)]),
        ..Config::default()
    };
    config.validate().unwrap();
    config.mcp.get_mut("server-1_2").unwrap().args = vec!["nul\0arg".into()];
    assert!(config.validate().is_err());
    config.mcp.get_mut("server-1_2").unwrap().args = vec!["x".into(); 257];
    assert!(config.validate().is_err());
    config.mcp.get_mut("server-1_2").unwrap().args.clear();
    config
        .mcp
        .get_mut("server-1_2")
        .unwrap()
        .env
        .insert("KEY".into(), "nul\0value".into());
    assert!(config.validate().is_err());
}

#[test]
fn mcp_catalog_controls_are_default_enabled_bounded_and_deny_first() {
    let parsed = load_text(
        "[mcp.remote]\nurl='https://example.test/mcp'\nenabled=true\nallow_tools=['read_*','shared']\ndeny_tools=['read_secret','shared']\nheader_env={Authorization='MCP_AUTH'}",
    )
    .unwrap();
    let remote = &parsed.mcp["remote"];
    assert!(remote.enabled);
    assert!(remote.admits_tool("read_public"));
    assert!(!remote.admits_tool("read_secret"));
    assert!(!remote.admits_tool("shared"));
    assert!(!remote.admits_tool("write_public"));

    let default = load_text("[mcp.local]\ncommand='runner'").unwrap();
    assert!(default.mcp["local"].enabled);
    assert!(default.mcp["local"].admits_tool("anything"));

    for text in [
        "[mcp.local]\ncommand='runner'\nheader_env={Authorization='MCP_AUTH'}",
        "[mcp.remote]\nurl='https://example.test/mcp'\nheader_env={Host='MCP_HOST'}",
        "[mcp.remote]\nurl='https://example.test/mcp'\nheader_env={'bad name'='MCP_AUTH'}",
        "[mcp.remote]\nurl='https://example.test/mcp'\nheader_env={Authorization='BAD-NAME'}",
        "[mcp.remote]\nurl='https://example.test/mcp'\nallow_tools=['']",
        "[mcp.remote]\nurl='https://example.test/mcp'\ndeny_tools=['bad[set]']",
    ] {
        assert!(load_text(text).is_err(), "{text}");
    }

    let mut oversized = McpConfig {
        command: Some("runner".into()),
        allow_tools: vec!["*".into(); 129],
        ..McpConfig::default()
    };
    let mut config = Config {
        mcp: BTreeMap::from([("local".into(), oversized.clone())]),
        ..Config::default()
    };
    assert!(config.validate().is_err());
    oversized.allow_tools.clear();
    oversized.deny_tools = vec!["?".into(); 129];
    config.mcp.insert("local".into(), oversized);
    assert!(config.validate().is_err());
}

#[test]
fn mcp_oauth_configuration_is_bounded_isolated_and_manifest_bound() {
    let directory = TempDir::new().unwrap();
    let project = directory.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let config = directory.path().join(".kuru/config.toml");
    let configured = concat!(
        "[mcp.remote]\n",
        "url='https://resource.example.test/mcp'\n",
        "header_env={X-Tenant='MCP_TENANT'}\n",
        "[mcp.remote.oauth]\n",
        "enabled=true\n",
        "client_id='kuru-test'\n",
        "client_secret_env='MCP_CLIENT_SECRET'\n",
        "scopes=['files:read','files:write']\n",
    );
    write(&config, configured);
    let first =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let parsed = load_text(configured).unwrap();
    let oauth = parsed.mcp["remote"].oauth.as_ref().unwrap();
    assert!(oauth.enabled);
    assert_eq!(oauth.client_id.as_deref(), Some("kuru-test"));
    assert_eq!(oauth.scopes, ["files:read", "files:write"]);
    let first_digest = first.manifest().full_digest();

    write(
        &config,
        concat!(
            "[mcp.remote]\n",
            "url='https://resource.example.test/mcp'\n",
            "header_env={X-Tenant='MCP_TENANT'}\n",
            "[mcp.remote.oauth]\n",
            "enabled=true\n",
            "client_id='kuru-test'\n",
            "client_secret_env='MCP_CLIENT_SECRET'\n",
            "scopes=['files:read']\n",
        ),
    );
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(changed.manifest().full_digest(), first_digest);

    for text in [
        "[mcp.local]\ncommand='runner'\n[mcp.local.oauth]\nenabled=true",
        "[mcp.remote]\nurl='http://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true",
        "[mcp.remote]\nurl='https://example.test/mcp'\nheader_env={Authorization='MCP_AUTH'}\n[mcp.remote.oauth]\nenabled=true",
        "[mcp.remote]\nurl='https://example.test/mcp'\nheader_env={'Proxy-Authorization'='MCP_AUTH'}\n[mcp.remote.oauth]\nenabled=true",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nclient_id='configured'\nclient_metadata_url='https://client.example.test/kuru.json'",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nclient_secret_env='MCP_SECRET'",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nclient_metadata_url='http://client.example.test/kuru.json'",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nscopes=['files:read','files:read']",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nscopes=['bad scope']",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\naccess_token='secret'",
    ] {
        assert!(load_text(text).is_err(), "{text}");
    }

    for text in [
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true",
        "[mcp.remote]\nurl='https://example.test/mcp'\n[mcp.remote.oauth]\nenabled=true\nclient_metadata_url='https://client.example.test/kuru.json'",
        "[mcp.remote]\nurl='https://example.test/mcp'\nheader_env={Authorization='MCP_AUTH'}\n[mcp.remote.oauth]\nenabled=false",
    ] {
        load_text(text).unwrap();
    }
}

#[test]
fn configured_peer_names_and_counts_are_bounded() {
    for text in [
        "[external_agents]\n'bad alias'='http://localhost'",
        "[external_agents]\nvalid='ws://localhost'",
    ] {
        assert!(load_text(text).is_err());
    }
    let mut config = Config {
        external_agents: (0..65)
            .map(|i| (format!("peer{i}"), "http://localhost".into()))
            .collect(),
        ..Config::default()
    };
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("64 external")
    );
    config.external_agents.clear();
    config.mcp = (0..65)
        .map(|i| {
            (
                format!("mcp{i}"),
                McpConfig {
                    command: Some("server".into()),
                    ..McpConfig::default()
                },
            )
        })
        .collect();
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("64 MCP")
    );
}

#[test]
fn remembered_selections_override_defaults_but_not_explicit_invocation_choices() {
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("user.toml");
    let local = dir.path().join("invocation.toml");
    write(
        &user,
        "mode='ifs'\nmodel='user-model'\neffort='low'\nmax_rounds=5",
    );
    write(
        dir.path().join(".kuru/config.toml"),
        "mode='polyvagal'\nmodel='project-model'",
    );
    let preferences = ProjectPreferences {
        mode: Some(Mode::Jungian),
        providers: BTreeMap::from([
            (
                "codex".into(),
                ModelPreference {
                    model: "remembered-codex".into(),
                    effort: Some("ultra".into()),
                },
            ),
            (
                "responses".into(),
                ModelPreference {
                    model: "remembered-api".into(),
                    effort: None,
                },
            ),
        ]),
    };
    let load = |local: Option<&Path>, provider, model| {
        Config::load_with_preferences(
            Some(&user),
            dir.path(),
            local,
            &preferences,
            SelectionOverrides {
                provider,
                model,
                ..SelectionOverrides::default()
            },
        )
        .unwrap()
    };
    let remembered = load(None, None, None);
    assert_eq!(remembered.mode, Mode::Jungian);
    assert_eq!(remembered.model, "remembered-codex");
    assert_eq!(remembered.effort.as_deref(), Some("ultra"));
    assert_eq!(remembered.max_rounds, 5);

    // A provider override selects that provider's saved pair, including an
    // explicit default effort which clears lower-level file defaults.
    let api = load(None, Some("responses"), None);
    assert_eq!(api.model, "remembered-api");
    assert_eq!(api.effort, None);
    let demo = load(None, Some("demo"), None);
    assert_eq!(demo.model, "project-model");
    assert_eq!(demo.effort.as_deref(), Some("low"));

    write(
        &local,
        "mode='freudian'\nmodel='local-model'\neffort='medium'",
    );
    let explicit = load(Some(&local), None, None);
    assert_eq!(explicit.mode, Mode::Freudian);
    assert_eq!(explicit.model, "local-model");
    assert_eq!(explicit.effort.as_deref(), Some("medium"));

    // Overriding only the model must not retain its predecessor's saved ultra.
    write(&local, "model='different-model'");
    assert_eq!(
        load(Some(&local), None, None).effort.as_deref(),
        Some("low")
    );
    assert_eq!(
        load(None, None, Some("cli-model")).effort.as_deref(),
        Some("low")
    );
    assert_eq!(
        load(None, None, Some("remembered-codex")).effort.as_deref(),
        Some("ultra")
    );
    write(&local, "provider='responses'");
    assert_eq!(load(Some(&local), None, None).model, "remembered-api");
    assert_eq!(
        load(Some(&local), Some("codex"), None).model,
        "remembered-codex"
    );
}

#[test]
fn stored_preferences_reject_malformed_choices_without_restricting_future_capabilities() {
    let dir = TempDir::new().unwrap();
    let mut preferences = ProjectPreferences {
        mode: None,
        providers: BTreeMap::from([(
            "future-provider".into(),
            ModelPreference {
                model: "future-model".into(),
                effort: Some("future-effort".into()),
            },
        )]),
    };
    preferences.validate().unwrap();
    for model in ["", " ", "with\0nul"] {
        preferences
            .providers
            .get_mut("future-provider")
            .unwrap()
            .model = model.into();
        assert!(
            Config::load_with_preferences(
                None,
                dir.path(),
                None,
                &preferences,
                SelectionOverrides::default()
            )
            .is_err()
        );
    }
    preferences
        .providers
        .get_mut("future-provider")
        .unwrap()
        .model = "valid".into();
    preferences
        .providers
        .get_mut("future-provider")
        .unwrap()
        .effort = Some("".into());
    assert!(preferences.validate().is_err());
    preferences.providers.clear();
    preferences.providers.insert(
        "bad/provider".into(),
        ModelPreference {
            model: "valid".into(),
            effort: None,
        },
    );
    assert!(preferences.validate().is_err());
    preferences.providers = (0..65)
        .map(|index| {
            (
                format!("provider-{index}"),
                ModelPreference {
                    model: "valid".into(),
                    effort: None,
                },
            )
        })
        .collect();
    assert!(preferences.validate().is_err());
    for value in [
        serde_json::json!({"mode":"unknown"}),
        serde_json::json!({"unexpected":true}),
        serde_json::json!({"providers":{"codex":{"model":42}}}),
        serde_json::json!({"providers":{"codex":{"model":"x","extra":true}}}),
    ] {
        assert!(serde_json::from_value::<ProjectPreferences>(value).is_err());
    }
}
