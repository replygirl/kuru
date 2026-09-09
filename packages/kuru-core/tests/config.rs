use std::{collections::BTreeMap, fs, path::Path};

use kuru_core::{
    Config, McpConfig, Mode, ModelPreference, ProjectPreferences, SelectionOverrides,
    load_instructions,
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
    assert!(format!("{:#}", Config::load(None, dir.path(), None).unwrap_err()).contains("UTF-8"));
    write(&config, "#".repeat(256 * 1024 + 1));
    assert!(format!("{:#}", Config::load(None, dir.path(), None).unwrap_err()).contains("256 KiB"));
    fs::remove_file(&config).unwrap();
    fs::create_dir(&config).unwrap();
    assert!(
        format!("{:#}", Config::load(None, dir.path(), None).unwrap_err()).contains("regular file")
    );
    write(dir.path().join("AGENTS.md"), [0xff]);
    assert!(load_instructions(dir.path()).is_err());
    write(dir.path().join("AGENTS.md"), "x".repeat(256 * 1024 + 1));
    assert!(load_instructions(dir.path()).is_err());
}

#[test]
fn combined_limits_apply_across_many_individually_valid_files() {
    let dir = TempDir::new().unwrap();
    let mut path = dir.path().to_owned();
    for _ in 0..5 {
        path.push("child");
        write(path.join("AGENTS.md"), "x".repeat(220_000));
        write(
            path.join(".kuru/config.toml"),
            format!("#{}\n", "x".repeat(220_000)),
        );
    }
    assert!(
        load_instructions(&path)
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
    assert!(
        Config::load(None, &path, None)
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
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
    assert!(instructions.contains("most local applicable AGENTS.md takes precedence"));
    assert!(instructions.contains("higher-priority conversation instructions"));
    assert!(instructions.contains("日本語の名前を使う。 🪶"));
    assert!(
        instructions.contains(
            &project
                .canonicalize()
                .unwrap()
                .join("AGENTS.md")
                .display()
                .to_string()
        )
    );
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
        assert!(
            format!("{:#}", load_text(text).unwrap_err()).contains(name),
            "{text}"
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
