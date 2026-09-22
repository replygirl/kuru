use std::{fs, path::Path};

use kuru_core::{
    AuthorityClaimCategory, Config, ConfigSnapshot, InvocationOverrides, McpConfig, NativeTool,
    PermissionAction, PermissionRule, PermissionSelector, ProjectPreferences,
    ProjectRelativeTarget,
};
use tempfile::TempDir;

fn target(path: &str) -> ProjectRelativeTarget {
    ProjectRelativeTarget::parse(path).unwrap()
}

fn rule(
    action: PermissionAction,
    selector: PermissionSelector,
    path: Option<&str>,
) -> PermissionRule {
    PermissionRule {
        action,
        selector,
        path: path.map(str::to_owned),
    }
}

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn overlapping_rules_ignore_array_order_and_legacy_flags_are_only_fallbacks() {
    let file = PermissionSelector::native(NativeTool::FileWrite);
    let path = target("src/nested/main.rs");
    let rules = [
        rule(PermissionAction::Allow, file.clone(), None),
        rule(PermissionAction::Deny, file.clone(), Some("src/**")),
        rule(PermissionAction::Ask, file.clone(), Some("src/nested/*")),
    ];
    for order in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
        let mut config = Config {
            allow_write: true,
            ..Config::default()
        };
        config.permissions = order.map(|index| rules[index].clone()).into();
        assert_eq!(
            config.permission_decision(&file, Some(&path)),
            PermissionAction::Deny
        );
        assert_eq!(
            config.permission_decision(&file, Some(&target("other.txt"))),
            PermissionAction::Allow
        );
    }

    let mut config = Config::default();
    config.mcp.insert(
        "files".into(),
        McpConfig {
            url: Some("https://example.test/mcp".into()),
            ..McpConfig::default()
        },
    );
    config
        .external_agents
        .insert("research".into(), "https://example.test/a2a".into());
    for name in [NativeTool::FileWrite, NativeTool::FileDelete] {
        assert_eq!(
            config.permission_decision(&PermissionSelector::native(name), Some(&path)),
            PermissionAction::Ask
        );
    }
    for name in [NativeTool::Shell, NativeTool::WebFetch] {
        assert_eq!(
            config.permission_decision(&PermissionSelector::native(name), None),
            PermissionAction::Ask
        );
    }
    config.allow_shell = true;
    assert_eq!(
        config.permission_decision(&PermissionSelector::native(NativeTool::Shell), None),
        PermissionAction::Allow
    );
    assert_eq!(
        config.permission_decision(&PermissionSelector::native(NativeTool::WebFetch), None),
        PermissionAction::Ask
    );
    for name in [NativeTool::FileRead, NativeTool::FileList] {
        assert_eq!(
            config.permission_decision(&PermissionSelector::native(name), Some(&path)),
            PermissionAction::Allow
        );
    }
    assert_eq!(
        config.permission_decision(
            &PermissionSelector::mcp("files", "write_document").unwrap(),
            None
        ),
        PermissionAction::Allow
    );
    assert_eq!(
        config.permission_decision(&PermissionSelector::a2a("research").unwrap(), None),
        PermissionAction::Allow
    );
    assert_eq!(
        config.permission_decision(&PermissionSelector::a2a("unconfigured").unwrap(), None),
        PermissionAction::Deny
    );
    config.permissions = vec![rule(PermissionAction::Ask, file.clone(), None)];
    config.allow_write = true;
    assert_eq!(
        config.permission_decision(&file, Some(&path)),
        PermissionAction::Ask
    );
    config.permissions = vec![rule(PermissionAction::Allow, file.clone(), None)];
    config.allow_write = false;
    assert_eq!(
        config.permission_decision(&file, Some(&path)),
        PermissionAction::Allow
    );
}

#[test]
fn anchored_globs_and_checked_targets_do_not_widen_scope() {
    let file = PermissionSelector::native(NativeTool::FileRead);
    let mut config = Config {
        permissions: vec![rule(
            PermissionAction::Deny,
            file.clone(),
            Some("src/**/report?.txt"),
        )],
        ..Config::default()
    };
    for path in ["src/report1.txt", "src/a/b/report2.txt"] {
        assert_eq!(
            config.permission_decision(&file, Some(&target(path))),
            PermissionAction::Deny
        );
    }
    for path in ["other/report1.txt", "src/report[1].txt", "src/report12.txt"] {
        assert_eq!(
            config.permission_decision(&file, Some(&target(path))),
            PermissionAction::Allow
        );
    }
    for path in [
        "",
        "/root",
        "C:/root",
        "src/../secret",
        "src/./file",
        "src//file",
        "src\\file",
    ] {
        assert!(ProjectRelativeTarget::parse(path).is_err(), "{path}");
    }
    assert_eq!(target("1:report").as_str(), "1:report");
    for control in ['\u{7f}', '\u{85}'] {
        assert!(ProjectRelativeTarget::parse(format!("src/{control}")).is_err());
        assert!(PermissionSelector::mcp("files", format!("write{control}")).is_err());
    }
    assert_eq!(target(".").as_str(), ".");
    config.permissions = vec![rule(PermissionAction::Deny, file.clone(), Some("**"))];
    assert_eq!(
        config.permission_decision(&file, Some(&target("."))),
        PermissionAction::Deny
    );
    assert_eq!(
        config.permission_decision(&file, None),
        PermissionAction::Deny
    );
    assert_eq!(
        config.permission_decision(
            &PermissionSelector::native(NativeTool::Shell),
            Some(&target("."))
        ),
        PermissionAction::Deny
    );
}

#[test]
fn strict_selector_pattern_and_route_validation_reject_invalid_authority() {
    for source in [
        "[[permissions]]\naction='permit'\nselector={kind='native',name='file_write'}",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='missing'}",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write',description='anything'}",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write'}\nextra=true",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write'}\npath='/absolute'",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write'}\npath='src/../secret'",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write'}\npath='src/a**b'",
        "[[permissions]]\naction='allow'\nselector={kind='native',name='shell'}\npath='src/**'",
        "[[permissions]]\naction='allow'\nselector={kind='mcp',alias='absent',tool='write_document'}",
        "[[permissions]]\naction='allow'\nselector={kind='a2a',alias='absent'}",
    ] {
        let dir = TempDir::new().unwrap();
        write(dir.path().join(".kuru/config.toml"), source);
        assert!(Config::load(None, dir.path(), None).is_err(), "{source}");
    }
}

#[test]
fn layered_rule_arrays_replace_and_effective_ancestor_rules_bind_trust() {
    let dir = TempDir::new().unwrap();
    let parent = dir.path().join("parent");
    let project = parent.join("project");
    fs::create_dir_all(&project).unwrap();
    let ancestor = parent.join(".kuru/config.toml");
    write(
        &ancestor,
        "[[permissions]]\naction='ask'\nselector={kind='native',name='file_write'}",
    );
    let first =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let claim = first
        .manifest()
        .claims()
        .iter()
        .find(|claim| claim.category() == AuthorityClaimCategory::ToolPermissions)
        .unwrap();
    assert_eq!(claim.sources().len(), 1);
    assert!(claim.display().as_str().contains("1 ordered"));
    let old_digest = first.manifest().full_digest();
    write(
        &ancestor,
        "[[permissions]]\naction='deny'\nselector={kind='native',name='file_write'}",
    );
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert_ne!(changed.manifest().full_digest(), old_digest);
    assert_eq!(
        first
            .finalize(&ProjectPreferences::default())
            .unwrap()
            .permission_decision(
                &PermissionSelector::native(NativeTool::FileWrite),
                Some(&target("src/a.rs"))
            ),
        PermissionAction::Ask
    );

    let local = dir.path().join("explicit.toml");
    write(
        &local,
        "[[permissions]]\naction='allow'\nselector={kind='native',name='file_write'}",
    );
    let overridden =
        ConfigSnapshot::parse(None, &project, Some(&local), InvocationOverrides::default())
            .unwrap();
    assert!(
        !overridden
            .manifest()
            .claims()
            .iter()
            .any(|claim| claim.category() == AuthorityClaimCategory::ToolPermissions)
    );
    let config = overridden.finalize(&ProjectPreferences::default()).unwrap();
    assert_eq!(config.permissions.len(), 1);
    assert_eq!(config.permissions[0].action, PermissionAction::Allow);

    write(
        project.join(".kuru/config.toml"),
        "[[permissions]]\naction='deny'\nselector={kind='native',name='shell'}",
    );
    let nested = Config::load(None, &project, None).unwrap();
    assert_eq!(nested.permissions.len(), 1);
    assert_eq!(
        nested.permissions[0].selector,
        PermissionSelector::native(NativeTool::Shell)
    );
}
