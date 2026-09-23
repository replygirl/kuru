use std::{fs, path::Path};

use kuru_core::{AuthorityClaimCategory, ConfigSnapshot, InvocationOverrides};

fn write(path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn capture(project: &Path, user: Option<&Path>) -> ConfigSnapshot {
    ConfigSnapshot::parse_with_sources(
        None,
        user,
        project,
        None,
        None,
        None,
        &["/help", "/status"],
        InvocationOverrides::default(),
    )
    .unwrap()
}

#[test]
fn startup_captures_only_skill_metadata_and_selection_binds_body_and_reference() {
    let project = tempfile::tempdir().unwrap();
    let skill = project.path().join(".agents/skills/review/SKILL.md");
    write(
        &skill,
        "---\nname: review\ndescription: Review changes\nallowed-tools: shell\n---\nBODY-SENTINEL\n",
    );
    write(
        project
            .path()
            .join(".agents/skills/review/references/check.md"),
        "REFERENCE-SENTINEL\n",
    );
    let base = capture(project.path(), None);
    assert_eq!(base.prompt_catalog().skills().count(), 1);
    assert!(base.instructions().contains("Review changes"));
    assert!(!base.instructions().contains("BODY-SENTINEL"));
    assert!(!base.instructions().contains("allowed-tools"));
    assert!(
        base.manifest()
            .claims()
            .iter()
            .any(|claim| { claim.category() == AuthorityClaimCategory::ProjectSkillMetadata })
    );
    assert!(
        !base
            .manifest()
            .claims()
            .iter()
            .any(|claim| { claim.category() == AuthorityClaimCategory::ProjectSkillMaterial })
    );

    let (selected, changed) = base
        .with_selected_skill("review", Some("references/check.md"))
        .unwrap();
    assert!(changed);
    assert!(selected.instructions().contains("BODY-SENTINEL"));
    assert!(selected.instructions().contains("REFERENCE-SENTINEL"));
    assert!(!selected.instructions().contains("allowed-tools"));
    assert_eq!(selected.supplemental_prompt_source_paths().len(), 2);
    assert!(
        selected
            .manifest()
            .claims()
            .iter()
            .any(|claim| { claim.category() == AuthorityClaimCategory::ProjectSkillMaterial })
    );
    let (again, changed) = selected
        .with_selected_skill("review", Some("references/check.md"))
        .unwrap();
    assert!(!changed);
    assert_eq!(again.instructions(), selected.instructions());
}

#[test]
fn effective_command_order_and_builtin_shadow_do_not_claim_inert_sources() {
    let project = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    write(
        user.path().join("commands/plan.md"),
        "---\nname: plan\ndescription: User plan\n---\nUSER-PLAN\n",
    );
    write(
        project.path().join(".kuru/commands/plan.md"),
        "---\nname: plan\ndescription: Project plan\n---\nPROJECT-PLAN\n",
    );
    write(
        project.path().join(".kuru/commands/help.md"),
        "---\nname: help\ndescription: Fake help\n---\nFAKE-HELP\n",
    );
    let captured = capture(project.path(), Some(user.path()));
    let commands = captured.prompt_catalog().commands().collect::<Vec<_>>();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "plan");
    assert_eq!(commands[0].body.trim(), "PROJECT-PLAN");
    assert!(
        captured
            .prompt_catalog()
            .notices()
            .iter()
            .any(|notice| notice.contains("shadowed"))
    );
    assert!(
        captured
            .manifest()
            .claims()
            .iter()
            .any(|claim| { claim.category() == AuthorityClaimCategory::ProjectCommands })
    );
    let first = captured.manifest().full_digest();
    write(
        project.path().join(".kuru/commands/help.md"),
        "---\nname: help\ndescription: Changed fake help\n---\nCHANGED-FAKE\n",
    );
    assert_eq!(
        capture(project.path(), Some(user.path()))
            .manifest()
            .full_digest(),
        first
    );
}

#[test]
fn selected_material_rejects_changed_metadata_linked_references_and_large_bodies() {
    let project = tempfile::tempdir().unwrap();
    let skill = project.path().join(".agents/skills/review/SKILL.md");
    write(&skill, "---\nname: review\ndescription: First\n---\nBODY\n");
    let captured = capture(project.path(), None);
    write(
        &skill,
        "---\nname: review\ndescription: Second\n---\nBODY\n",
    );
    assert!(captured.with_selected_skill("review", None).is_err());

    write(&skill, "---\nname: review\ndescription: First\n---\nBODY\n");
    let fresh = capture(project.path(), None);
    let reference = project
        .path()
        .join(".agents/skills/review/references/check.md");
    write(&reference, "REFERENCE");
    fs::hard_link(
        &reference,
        project
            .path()
            .join(".agents/skills/review/references/other.md"),
    )
    .unwrap();
    assert!(
        fresh
            .with_selected_skill("review", Some("references/check.md"))
            .is_err()
    );
    assert!(
        fresh
            .with_selected_skill("review", Some("references/../check.md"))
            .is_err()
    );

    let mut oversized = b"---\nname: review\ndescription: First\n---\n".to_vec();
    oversized.extend(vec![b'x'; 256 * 1024 + 1]);
    write(&skill, oversized);
    let metadata_only = capture(project.path(), None);
    assert_eq!(metadata_only.prompt_catalog().skills().count(), 1);
    assert!(metadata_only.with_selected_skill("review", None).is_err());
}

#[test]
fn project_commands_keep_priority_when_lower_precedence_files_fill_the_byte_budget() {
    let project = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let project_body = "P".repeat(8 * 1024);
    write(
        project.path().join(".kuru/commands/plan.md"),
        format!("---\nname: plan\ndescription: Project plan\n---\n{project_body}"),
    );
    for index in 0..4 {
        let name = format!("user-{index}");
        write(
            user.path().join(format!("commands/{name}.md")),
            format!(
                "---\nname: {name}\ndescription: User command\n---\n{}",
                "U".repeat(63 * 1024)
            ),
        );
    }
    let captured = capture(project.path(), Some(user.path()));
    assert_eq!(
        captured
            .prompt_catalog()
            .commands()
            .find(|entry| entry.name == "plan")
            .unwrap()
            .body,
        project_body
    );
    assert!(
        captured
            .prompt_catalog()
            .notices()
            .iter()
            .any(|notice| notice.contains("limit"))
    );
}

#[test]
fn control_characters_in_metadata_do_not_enter_help_or_actor_catalog() {
    let project = tempfile::tempdir().unwrap();
    write(
        project.path().join(".agents/skills/review/SKILL.md"),
        "---\nname: review\ndescription: \"Bad\\u001b[31m label\"\n---\nBODY\n",
    );
    write(
        project.path().join(".kuru/commands/report.md"),
        "---\nname: report\ndescription: |\n  Split\n  label\n---\nPROMPT\n",
    );
    let captured = capture(project.path(), None);
    assert_eq!(captured.prompt_catalog().skills().count(), 0);
    assert_eq!(captured.prompt_catalog().commands().count(), 0);
    assert!(!captured.instructions().contains("Bad"));
    assert!(captured.prompt_catalog().notices().len() >= 2);
}

#[test]
fn project_skill_metadata_precedes_user_and_oversized_entries_are_omitted() {
    let project = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    write(
        project.path().join(".agents/skills/review/SKILL.md"),
        "---\nname: review\ndescription: Project review\n---\nPROJECT-BODY\n",
    );
    write(
        user.path().join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: User review\n---\nUSER-BODY\n",
    );
    write(
        project.path().join(".agents/skills/oversized/SKILL.md"),
        format!(
            "---\nname: oversized\ndescription: {}\n---\nBODY\n",
            "x".repeat(8 * 1024)
        ),
    );
    let captured = capture(project.path(), Some(user.path()));
    let skills = captured.prompt_catalog().skills().collect::<Vec<_>>();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "review");
    assert_eq!(skills[0].description, "Project review");
    assert!(!captured.instructions().contains("USER-BODY"));
    assert!(!captured.instructions().contains("PROJECT-BODY"));
    assert!(
        captured
            .prompt_catalog()
            .notices()
            .iter()
            .any(|notice| notice.contains("omitted"))
    );
}
