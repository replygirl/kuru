#![cfg(feature = "tooling")]

use kuru_delivery::command::BlockingCommand as Command;
use std::{fs, path::PathBuf};
#[path = "support/files.rs"]
mod files;

use kuru_delivery::repo;

struct Repository(tempfile::TempDir);

impl Repository {
    fn new() -> Self {
        let repo = Self(tempfile::tempdir().unwrap());
        repo.write("Cargo.toml", "[workspace]\nmembers = [\"apps/kuru-tui\", \"packages/kuru-core\"]\n[workspace.dependencies]\nserde = \"=1.0.229\"\nkuru-core = { path = \"packages/kuru-core\" }\n");
        repo.write("mise.toml", "monorepo_root = true\n[tools]\nrust = \"1.98.1\"\n[monorepo]\nconfig_roots = [\"apps/*\", \"packages/*\"]\n");
        repo.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.98.1\"\n");
        repo.write("AGENTS.md", "# Kuru\nCanonical instructions.\n");
        repo.write("CLAUDE.md", "@AGENTS.md\n");
        repo.write(
            "apps/kuru-tui/Cargo.toml",
            "[package]\nname = \"kuru\"\n[dependencies]\nserde.workspace = true\n",
        );
        repo.write(
            "apps/kuru-tui/mise.toml",
            "[tasks.test]\nrun = \"cargo test -p kuru\"\n",
        );
        repo.write(
            "packages/kuru-core/Cargo.toml",
            "[package]\nname = \"kuru-core\"\n",
        );
        repo.write(
            "packages/kuru-core/mise.toml",
            "[tasks.test]\nrun = \"cargo test -p kuru-core\"\n",
        );
        repo.write("apps/kuru-docs/package.json", "{\"private\":true}");
        repo.write(
            "apps/kuru-docs/mise.toml",
            "[tasks.build]\nrun = \"npm run docs:build\"\n",
        );
        repo
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.0.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn errors(&self) -> Vec<String> {
        repo::check(self.0.path()).unwrap()
    }

    fn replace(&self, relative: &str, old: &str, new: &str) {
        let path = self.0.path().join(relative);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(old));
        fs::write(path, content.replace(old, new)).unwrap();
    }
}

#[test]
fn owned_packages_exact_pins_and_canonical_instructions_pass_real_cli() {
    let repo = Repository::new();
    repo.write(
        "scripts/install.sh",
        "#!/usr/bin/env bash\nexec cargo run --bin kuru-delivery -- \"$@\"\n",
    );
    repo.write(
        "scripts/check-commit.sh",
        "#!/usr/bin/env bash\nexec cog verify --file \"$1\"\n",
    );
    repo.write(
        "scripts/install.ps1",
        "& $PSScriptRoot/../packages/kuru-delivery/support/install.ps1 @args\n",
    );
    assert!(repo.errors().is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"))
        .args(["repo", "--root"])
        .arg(repo.0.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo.write("CLAUDE.md", "Duplicated instructions.");
    let output = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"))
        .args(["repo", "--root"])
        .arg(repo.0.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("@AGENTS.md"));
}

#[test]
fn rust_toolchain_and_mise_monorepo_scope_must_match() {
    let repo = Repository::new();
    repo.replace("rust-toolchain.toml", "1.98.1", "1.97.0");
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("Rust pins differ"))
    );
    repo.write("mise.toml", "[tools]\nnode = \"26.8.2\"\n");
    let errors = repo.errors();
    assert!(errors.iter().any(|error| error.contains("monorepo_root")));
    assert!(errors.iter().any(|error| error.contains("apps/*")));
    assert!(errors.iter().any(|error| error.contains("packages/*")));
}

#[test]
fn root_rust_pin_may_carry_tool_options_but_must_match_the_toolchain() {
    let repo = Repository::new();
    let options = "rust = { version = \"1.98.1\", mr_boxington = \"{{ get_env(name='KURU_MBX', default='1') != '0' }}\" }\nmr-boxington = { version = \"1.17.0\", os = [\"linux\", \"macos/arm64\", \"windows\"] }";
    repo.replace("mise.toml", "rust = \"1.98.1\"", options);
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
    repo.replace("rust-toolchain.toml", "1.98.1", "1.97.0");
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("Rust pins differ"))
    );
    let repo = Repository::new();
    repo.replace(
        "mise.toml",
        "rust = \"1.98.1\"",
        "rust = { mr_boxington = true }",
    );
    let errors = repo.errors();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Rust pins differ"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("rust must be exactly pinned"))
    );
}

#[test]
fn root_mise_tools_require_exact_versions() {
    for pin in [
        "\"latest\"",
        "\"1.17\"",
        "\"^1.17.0\"",
        "{ version = \"latest\", os = [\"linux\"] }",
        "{ os = [\"linux\"] }",
        "[\"1.17.0\", \"1.18.0\"]",
    ] {
        let repo = Repository::new();
        repo.replace(
            "mise.toml",
            "rust = \"1.98.1\"\n",
            &format!("rust = \"1.98.1\"\nmr-boxington = {pin}\n"),
        );
        assert!(
            repo.errors()
                .iter()
                .any(|error| error.contains("mr-boxington must be exactly pinned")),
            "{pin}"
        );
    }
}

#[test]
fn registry_dependencies_require_complete_exact_versions() {
    for version in [
        "1.2.3",
        "^1.2.3",
        "=1.*",
        "=latest",
        "=1.2",
        "=1.2.3 || =2.0.0",
    ] {
        let repo = Repository::new();
        repo.replace("Cargo.toml", "=1.0.229", version);
        assert!(
            repo.errors()
                .iter()
                .any(|error| error.contains("serde must be exactly pinned")),
            "{version}"
        );
    }
    let repo = Repository::new();
    repo.replace("Cargo.toml", "=1.0.229", "=1.0.0-alpha.2");
    assert!(repo.errors().is_empty());
    repo.replace(
        "Cargo.toml",
        "serde = \"=1.0.0-alpha.2\"",
        "serde = { git = \"https://example.invalid/repo\" }",
    );
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("exact registry pin"))
    );
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("needs an exact version"))
    );
}

#[test]
fn workspace_members_and_paths_belong_directly_to_apps_or_packages() {
    for member in [
        "outside",
        "apps/../outside",
        "apps/nested/child",
        "apps/*",
        "/tmp/outside",
        "apps/./hidden",
        "packages/.hidden",
    ] {
        let repo = Repository::new();
        repo.replace("Cargo.toml", "apps/kuru-tui", member);
        assert!(
            repo.errors()
                .iter()
                .any(|error| error.contains("must belong directly")),
            "{member}"
        );
    }
    let repo = Repository::new();
    repo.replace(
        "Cargo.toml",
        "path = \"packages/kuru-core\"",
        "path = \"../external\"",
    );
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("path must belong"))
    );
}

#[test]
fn package_manifests_tasks_and_workspace_membership_cannot_go_missing() {
    let repo = Repository::new();
    fs::remove_file(repo.0.path().join("packages/kuru-core/Cargo.toml")).unwrap();
    fs::remove_file(repo.0.path().join("apps/kuru-tui/mise.toml")).unwrap();
    fs::remove_file(repo.0.path().join("apps/kuru-docs/mise.toml")).unwrap();
    repo.write(
        "packages/orphan/Cargo.toml",
        "[package]\nname = \"orphan\"\n",
    );
    let errors = repo.errors();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("workspace member missing"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("apps/kuru-tui: package-owned mise.toml"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("apps/kuru-docs: package-owned mise.toml"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("missing from Cargo workspace: packages/orphan"))
    );
}

#[test]
fn member_dependencies_inherit_workspace_pins_including_target_sections() {
    let repo = Repository::new();
    repo.write("apps/kuru-tui/Cargo.toml", "[dependencies]\nserde = \"=1.0.229\"\n[target.'cfg(unix)'.dev-dependencies]\nserde_json = \"=1.0.151\"\n");
    let errors = repo.errors();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("dependency serde must inherit"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("dependency serde_json must inherit"))
    );
}

#[test]
fn root_language_projects_scripts_and_unneeded_interpreters_are_rejected() {
    let repo = Repository::new();
    for filename in [
        "package.json",
        "package-lock.json",
        "bun.lock",
        "bun.lockb",
        "pyproject.toml",
        "uv.lock",
    ] {
        repo.write(filename, "fixture");
    }
    repo.write("scripts/legacy.py", "fixture");
    repo.write("scripts/unowned.ps1", "fixture");
    repo.replace(
        "mise.toml",
        "[tools]",
        "[tools]\nbun = \"1.4.2\"\npython = \"3.14.7\"\nuv = \"0.10.0\"",
    );
    let errors = repo.errors();
    assert_eq!(
        errors
            .iter()
            .filter(|error| error.contains("language tooling must belong"))
            .count(),
        6
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("helper implementation must belong"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("scripts/unowned.ps1"))
    );
    assert_eq!(
        errors
            .iter()
            .filter(|error| error.contains("not required by native"))
            .count(),
        3
    );
}

#[test]
fn canonical_instructions_and_malformed_metadata_fail_explicitly() {
    let repo = Repository::new();
    repo.write("AGENTS.md", "See another file.");
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("canonical repository instructions"))
    );
    repo.write("Cargo.toml", "invalid TOML {");
    assert!(
        repo::check(repo.0.path())
            .unwrap_err()
            .to_string()
            .contains("parse")
    );
    repo.write("Cargo.toml", "[package]\nname = \"wrong-root\"\n");
    assert!(
        repo::check(repo.0.path())
            .unwrap_err()
            .to_string()
            .contains("lacks workspace")
    );
}

#[cfg(unix)]
#[test]
fn member_symlink_does_not_read_an_external_manifest() {
    let repo = Repository::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("Cargo.toml"), "invalid external TOML {").unwrap();
    fs::remove_dir_all(repo.0.path().join("packages/kuru-core")).unwrap();
    files::symlink(outside.path(), repo.0.path().join("packages/kuru-core")).unwrap();
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("escapes repository"))
    );
}
