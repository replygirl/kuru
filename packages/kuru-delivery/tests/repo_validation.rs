#![cfg(feature = "tooling")]

use kuru_delivery::command::BlockingCommand as Command;
use std::{fs, path::PathBuf};
#[path = "support/files.rs"]
mod files;

use kuru_delivery::repo;

/// The actual shared lint configuration, so fixtures start from what passes.
const ROOT_CLIPPY: &str = include_str!("../../../clippy.toml");

struct Repository(tempfile::TempDir);

impl Repository {
    fn new() -> Self {
        let repo = Self(tempfile::tempdir().unwrap());
        repo.write("Cargo.toml", "[workspace]\nmembers = [\"apps/kuru-tui\", \"packages/kuru-core\"]\n[workspace.dependencies]\nserde = \"=1.0.229\"\nkuru-core = { path = \"packages/kuru-core\" }\n");
        repo.write("mise.toml", "monorepo_root = true\n[tools]\nrust = \"1.98.1\"\n[monorepo]\nconfig_roots = [\"apps/*\", \"packages/*\"]\n");
        repo.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.98.1\"\n");
        repo.write("AGENTS.md", "# Kuru\nCanonical instructions.\n");
        repo.write("CLAUDE.md", "@AGENTS.md\n");
        repo.write("clippy.toml", ROOT_CLIPPY);
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

#[test]
fn per_platform_tool_options_require_their_complete_lock_entry() {
    const PLATFORM_PIN: &str = "[tools.\"github:aligned-team/cospec\"]\nversion = \"0.7.1\"\n\n[tools.\"github:aligned-team/cospec\".platforms.windows-arm64]\nasset_pattern = \"cospec-*-windows-x64.zip\"\n";
    const HOST_ELEMENT: &str = "[[tools.\"github:aligned-team/cospec\"]]\nversion = \"0.7.1\"\nbackend = \"github:aligned-team/cospec\"\nspecifiers = [\"0.7.1\"]\n\n[tools.\"github:aligned-team/cospec\".\"platforms.windows-x64\"]\nchecksum = \"sha256:823a\"\nurl = \"https://example.invalid/cospec-0.7.1-windows-x64.zip\"\n";
    const OPTION_ELEMENT: &str = "\n[[tools.\"github:aligned-team/cospec\"]]\nversion = \"0.7.1\"\nbackend = \"github:aligned-team/cospec\"\nspecifiers = [\"0.7.1\"]\n\n[tools.\"github:aligned-team/cospec\".options]\nasset_pattern = \"cospec-*-windows-x64.zip\"\n\n[tools.\"github:aligned-team/cospec\".\"platforms.windows-arm64\"]\nchecksum = \"sha256:823a\"\nurl = \"https://example.invalid/cospec-0.7.1-windows-x64.zip\"\n";
    let lock_error = |repo: &Repository| {
        repo.errors()
            .iter()
            .any(|error| error.contains("windows-arm64 option entry"))
    };

    let repo = Repository::new();
    let config = format!(
        "{}{PLATFORM_PIN}",
        std::fs::read_to_string(repo.0.path().join("mise.toml")).unwrap()
    );
    repo.write("mise.toml", &config);
    repo.write(
        "mise.lock",
        &format!("lockfile_version = 1\n\n{HOST_ELEMENT}{OPTION_ELEMENT}"),
    );
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());

    // An unlocked install on another host removes the whole option element.
    repo.write(
        "mise.lock",
        &format!("lockfile_version = 1\n\n{HOST_ELEMENT}"),
    );
    assert!(lock_error(&repo));

    // Auto-lock can re-add the platform row without the request binding.
    repo.write(
        "mise.lock",
        &format!("lockfile_version = 1\n\n{HOST_ELEMENT}{OPTION_ELEMENT}"),
    );
    repo.replace(
        "mise.lock",
        "specifiers = [\"0.7.1\"]\n\n[tools.\"github:aligned-team/cospec\".options]",
        "\n[tools.\"github:aligned-team/cospec\".options]",
    );
    assert!(lock_error(&repo));

    for (old, new) in [
        (
            "checksum = \"sha256:823a\"\nurl = \"https://example.invalid/cospec-0.7.1-windows-x64.zip\"\n",
            "url = \"https://example.invalid/cospec-0.7.1-windows-x64.zip\"\n",
        ),
        (
            "asset_pattern = \"cospec-*-windows-x64.zip\"\n\n[tools.\"github:aligned-team/cospec\".\"platforms.windows-arm64\"]",
            "asset_pattern = \"cospec-*-windows-arm64.zip\"\n\n[tools.\"github:aligned-team/cospec\".\"platforms.windows-arm64\"]",
        ),
    ] {
        repo.write(
            "mise.lock",
            &format!("lockfile_version = 1\n\n{HOST_ELEMENT}{OPTION_ELEMENT}"),
        );
        let content = std::fs::read_to_string(repo.0.path().join("mise.lock")).unwrap();
        let at = content.rfind(old).unwrap();
        repo.write(
            "mise.lock",
            &format!("{}{new}{}", &content[..at], &content[at + old.len()..]),
        );
        assert!(lock_error(&repo));
    }

    std::fs::remove_file(repo.0.path().join("mise.lock")).unwrap();
    assert!(
        repo::check(repo.0.path())
            .unwrap_err()
            .to_string()
            .contains("mise.lock")
    );

    repo.replace("mise.toml", "[tools.\"github:aligned-team/cospec\".platforms.windows-arm64]\nasset_pattern = \"cospec-*-windows-x64.zip\"\n", "[tools.\"github:aligned-team/cospec\".platforms]\nwindows-arm64 = \"cospec-*-windows-x64.zip\"\n");
    repo.write(
        "mise.lock",
        &format!("lockfile_version = 1\n\n{HOST_ELEMENT}{OPTION_ELEMENT}"),
    );
    assert!(
        repo.errors()
            .iter()
            .any(|error| error.contains("options must be a table"))
    );
}

const WORKFLOWS: [&str; 6] = [
    "bundle-build.yml",
    "ci.yml",
    "native-tests.yml",
    "pr-title.yml",
    "quality.yml",
    "release.yml",
];

/// The fixed apt step text in native-tests.yml: each fetch is bounded and
/// the update and install pair runs at most twice.
const NATIVE_APT: &str = r#"          # These packages come from the Ubuntu archive. apt reads only its deb822
          # list and no other sources.list.d entry, so an outage of a third-party
          # repository on the runner image cannot fail this step; errors from the
          # archive still fail it.
          if [ ! -f /etc/apt/sources.list.d/ubuntu.sources ]; then
            echo 'The runner image has no Ubuntu archive list at /etc/apt/sources.list.d/ubuntu.sources' >&2
            exit 1
          fi
          # Job 110454097537 stalled 42 minutes inside apt-get update, past apt's
          # own bounds, so coreutils timeout caps each fetch. The cap lets apt's
          # bounds end a failure first, with apt's own error; derivation in the
          # Measured basis of openspec change bound-apt-fixture-tools-step:
          #   127 s  one stalled item under apt 2.8.3 defaults: (1 + 3 retries)
          #          x 30 s per wait + 1 + 2 + 4 s retry delays (basehttp.cc
          #          TimeOut(30), acquire-item.cc Acquire::Retries 3,
          #          acquire-worker.cc Acquire::Retries::Delay)
          #  + 17 s  that hang's mirror fallback, mirror list to first Hit:
          #  + 43 s  slowest whole step over 200 healthy partition jobs
          #  = 187 s for each fetch
          budget=187
          # The update and install pair runs at most twice; nothing else retries.
          for attempt in 1 2; do
            echo "Ubuntu archive apt attempt $attempt of 2"
            status=0
            timeout "$budget" sudo apt-get -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -o Dir::Etc::sourceparts=/dev/null update || status=$?
            if [ "$status" -eq 0 ]; then
              timeout "$budget" sudo apt-get -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -o Dir::Etc::sourceparts=/dev/null \
                install -y --no-install-recommends dbus gnome-keyring libsecret-tools || status=$?
            fi
            if [ "$status" -eq 0 ]; then
              exit 0
            elif [ "$attempt" -eq 1 ]; then
              echo "::warning::Ubuntu archive apt attempt 1 of 2 failed with exit status $status (124 = timed out after $budget s), retrying once"
            fi
          done
          echo "::error::Ubuntu archive apt attempt 2 of 2 failed with exit status $status (124 = timed out after $budget s)"
          exit "$status"
"#;
/// The fixed apt step text in release.yml.
const RELEASE_APT: &str = "          # These packages come from the Ubuntu archive. apt reads only its deb822
          # list and no other sources.list.d entry, so an outage of a third-party
          # repository on the runner image cannot fail this step; errors from the
          # archive still fail it.
          if [ ! -f /etc/apt/sources.list.d/ubuntu.sources ]; then
            echo 'The runner image has no Ubuntu archive list at /etc/apt/sources.list.d/ubuntu.sources' >&2
            exit 1
          fi
          sudo apt-get -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -o Dir::Etc::sourceparts=/dev/null update
          sudo apt-get -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -o Dir::Etc::sourceparts=/dev/null \\
            install -y --no-install-recommends dbus gnome-keyring libsecret-tools
";
/// The same step before the fix, as run 36453397286 partition 7 ran it.
const INCIDENT_APT: &str = "          sudo apt-get update
          sudo apt-get install -y --no-install-recommends dbus gnome-keyring libsecret-tools
";
const FIXED_EXEC: &str =
    "  # Jobs install only their mise-action install_args. Windows exe shims run
  # `mise x`, which would otherwise download every other configured tool.
  MISE_EXEC_AUTO_INSTALL: \"false\"
";
const PARTITION_OFFLINE: &str =
    "      # Import the run's verified bundle inputs; never download them here.
      KURU_DOLT_BUNDLE_OFFLINE: \"true\"
";

impl Repository {
    /// A repository carrying this checkout's actual workflows.
    fn with_workflows() -> Self {
        let repo = Self::new();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
        for name in WORKFLOWS {
            let text = fs::read_to_string(source.join(name)).unwrap();
            repo.write(&format!(".github/workflows/{name}"), &text);
        }
        repo
    }

    fn workflow_errors(&self) -> Vec<String> {
        self.errors()
            .into_iter()
            .filter(|error| error.starts_with(".github/workflows/"))
            .collect()
    }
}

#[test]
fn actual_workflows_fetch_only_what_each_job_uses() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let mut present: Vec<_> = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    present.sort();
    assert_eq!(present, WORKFLOWS, "a new workflow must join this check");
    let repo = Repository::with_workflows();
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
}

#[test]
fn incident_exec_auto_install_workflow_is_rejected() {
    // Run 36319170835: without the workflow-level opt-out, a Windows shim's
    // `mise x` downloaded five unselected tools and mr-boxington returned 500.
    let repo = Repository::with_workflows();
    repo.replace(".github/workflows/native-tests.yml", FIXED_EXEC, "");
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/native-tests.yml: uses mise, so its workflow-level env must set MISE_EXEC_AUTO_INSTALL: \"false\""
        ]
    );
    // An explicit "true" or an override below the workflow brings it back.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/quality.yml",
        "MISE_EXEC_AUTO_INSTALL: \"false\"",
        "MISE_EXEC_AUTO_INSTALL: \"true\"",
    );
    repo.replace(
        ".github/workflows/native-tests.yml",
        "      KURU_NATIVE_MODE:",
        "      MISE_EXEC_AUTO_INSTALL: \"true\"\n      KURU_NATIVE_MODE:",
    );
    repo.replace(
        ".github/workflows/native-tests.yml",
        "        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      # Provisioning, not hiding:",
        "        env:\n          GITHUB_TOKEN: ${{ github.token }}\n          MISE_EXEC_AUTO_INSTALL: \"true\"\n      # Provisioning, not hiding:",
    );
    repo.replace(
        ".github/workflows/ci.yml",
        "        run: printf 'KURU_DOLT_BUNDLE_DIR=%s/kuru-bundles\\n' \"$RUNNER_TEMP\" >> \"$GITHUB_ENV\"\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          experimental: true\n          version: 2026.9.18\n          install_args: rust\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2\n        with:\n          cache-bin: false\n          shared-key: bundle-inputs",
        "        run: echo 'MISE_EXEC_AUTO_INSTALL=true' >> \"$GITHUB_ENV\"\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          experimental: true\n          version: 2026.9.18\n          install_args: rust\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2\n        with:\n          cache-bin: false\n          shared-key: bundle-inputs",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/ci.yml: job bundle-inputs step Select private bundle build inputs overrides MISE_EXEC_AUTO_INSTALL in its script; set it only in the workflow-level env",
            ".github/workflows/native-tests.yml: job shard overrides MISE_EXEC_AUTO_INSTALL; set it only in the workflow-level env",
            ".github/workflows/native-tests.yml: job shard step jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c overrides MISE_EXEC_AUTO_INSTALL; set it only in the workflow-level env",
            ".github/workflows/quality.yml: uses mise, so its workflow-level env must set MISE_EXEC_AUTO_INSTALL: \"false\"",
        ]
    );
}

#[test]
fn mise_steps_must_name_the_tools_they_install() {
    let repo = Repository::new();
    let workflow = "name: Fixture\non: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: false\n  MISE_TASK_RUN_AUTO_INSTALL: false\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          install_args: rust\n      - name: Selected tools\n        run: |\n          mise install --locked rust aqua:rhysd/actionlint\n          MISE_LOCKED=1 mise i cargo:cargo-audit\n";
    repo.write(".github/workflows/fixture.yaml", workflow);
    repo.write(".github/workflows/notes.md", "mise install");
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
    repo.replace(
        ".github/workflows/fixture.yaml",
        "        with:\n          install_args: rust\n",
        "        with:\n          install_args: \" \"\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n",
    );
    repo.replace(
        ".github/workflows/fixture.yaml",
        "          MISE_LOCKED=1 mise i cargo:cargo-audit\n",
        "          MISE_LOCKED=1 mise i\n          sudo env mise install --locked # every configured tool\n          echo ready && mise.exe install --yes\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/fixture.yaml: job build step Selected tools runs `mise install` without naming the tools to install",
            ".github/workflows/fixture.yaml: job build step jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c uses jdx/mise-action without explicit install_args naming the tools the job uses",
        ]
    );
    // A workflow that never runs mise needs no opt-out.
    let repo = Repository::new();
    repo.write(
        ".github/workflows/plain.yml",
        "on: push\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo mise is not called here\n",
    );
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
    repo.write(
        ".github/workflows/plain.yml",
        "on: push\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: mise run lint\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/plain.yml: uses mise, so its workflow-level env must set MISE_EXEC_AUTO_INSTALL: \"false\"",
            ".github/workflows/plain.yml: uses mise, so its workflow-level env must set MISE_TASK_RUN_AUTO_INSTALL: \"false\"",
        ]
    );
    // Options, their values and prefixes do not hide a bare install. The
    // value options are mise 2026.9.18's global `-C/--cd`, `-E/--env` and
    // `-j/--jobs`, and install's `-j/--jobs`, `--minimum-release-age` and
    // `--shared`.
    let script = |command: &str| {
        format!(
            "on: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: \"false\"\n  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {command}\n"
        )
    };
    for bare in [
        "sudo -E mise install",
        "mise install --jobs 4",
        "mise install --env ci",
        "mise install --shared /opt/mise",
        "mise -y install",
        "mise --cd . install",
        "mise install -j 4",
        "mise install --minimum-release-age 90d",
        "mise -C . -E ci -yj 4 install --locked",
        "/home/runner/.local/bin/mise i --",
        // A prefix's option value or timeout's duration is not the program.
        "timeout 600 mise install",
        "timeout -k 5 -s KILL 600 mise install",
        "sudo -u runner mise install",
        "sudo -Eu runner env -u MISE_ENV nice -n 5 nohup mise install",
        "exec -a mise-install mise install",
        "env -S 'mise install'",
    ] {
        repo.write(".github/workflows/plain.yml", &script(bare));
        assert_eq!(
            repo.workflow_errors(),
            [
                ".github/workflows/plain.yml: job lint step step 1 runs `mise install` without naming the tools to install"
            ],
            "{bare}"
        );
    }
    for named in [
        "mise install --jobs 4 rust",
        "mise --cd . install --shared /opt/mise rust",
        "mise install --jobs=4 rust",
        "mise -j4 install rust",
        "mise install -- rust",
        "mise run install",
        "sudo -u runner mise install rust",
        "timeout -k 5 600 mise install rust",
        "mise upgrade rust",
        "mise bootstrap packages apply --yes",
    ] {
        repo.write(".github/workflows/plain.yml", &script(named));
        assert!(repo.errors().is_empty(), "{named}: {:?}", repo.errors());
    }
    // `upgrade` and `bootstrap` also install every configured tool when
    // given none (mise 2026.9.18 `docs/cli/upgrade.md`, `docs/cli/bootstrap.md`:
    // phase 6, versioned tools).
    for (bare, subcommand) in [
        ("mise upgrade", "upgrade"),
        ("mise up -x go --minimum-release-age 90d", "upgrade"),
        ("mise bootstrap", "bootstrap"),
        ("mise bs --yes --only tools", "bootstrap"),
        (
            "timeout 900 mise bootstrap --from-dir /tmp/x -y",
            "bootstrap",
        ),
    ] {
        repo.write(".github/workflows/plain.yml", &script(bare));
        assert_eq!(
            repo.workflow_errors(),
            [format!(
                ".github/workflows/plain.yml: job lint step step 1 runs `mise {subcommand}` without naming the tools to install"
            )],
            "{bare}"
        );
    }
    // mise-action runs `mise install <install_args>`: options alone are the
    // same bare install.
    let action = |arguments: &str| {
        format!(
            "on: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: \"false\"\n  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          install_args: {arguments}\n"
        )
    };
    for bare in [
        "--jobs 4",
        "-y",
        "--shared /opt/x",
        "-j 4 --locked --",
        "\"\"",
    ] {
        repo.write(".github/workflows/plain.yml", &action(bare));
        assert_eq!(
            repo.workflow_errors(),
            [
                ".github/workflows/plain.yml: job lint step jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c uses jdx/mise-action without explicit install_args naming the tools the job uses"
            ],
            "{bare}"
        );
    }
    for named in ["--jobs 4 rust", "-- rust", "rust aqua:jdx/hk"] {
        repo.write(".github/workflows/plain.yml", &action(named));
        assert!(repo.errors().is_empty(), "{named}: {:?}", repo.errors());
    }
    repo.write(".github/workflows/plain.yml", "jobs: [unterminated");
    assert!(
        repo::check(repo.0.path())
            .unwrap_err()
            .to_string()
            .contains("parse .github/workflows/plain.yml")
    );
}

#[test]
fn incident_apt_update_over_every_source_is_rejected() {
    // Run 36453397286, Ubuntu coverage partition 7: `apt-get update` failed
    // on packages.microsoft.com (403), a runner-image list the step never used.
    let repo = Repository::with_workflows();
    for (name, fixed) in [
        ("native-tests.yml", NATIVE_APT),
        ("release.yml", RELEASE_APT),
    ] {
        repo.replace(&format!(".github/workflows/{name}"), fixed, INCIDENT_APT);
    }
    let errors = repo.workflow_errors();
    let expected: Vec<String> = [
        ("native-tests.yml", "shard"),
        ("release.yml", "tests"),
    ]
    .iter()
    .flat_map(|(name, job)| {
        [
            "apt-get install -y --no-install-recommends dbus gnome-keyring libsecret-tools",
            "apt-get update",
        ]
        .map(|command| format!(".github/workflows/{name}: job {job} step Install Ubuntu native secret-store fixture tools runs `{command}` over every configured apt source; name the needed list with one -o Dir::Etc::sourcelist=/..., make the last -o Dir::Etc::sourceparts=/dev/null, and pass no -c"))
    })
    .collect();
    assert_eq!(errors, expected);
    // The bounded retry loop hides neither fetch: each begins its own command
    // after only the `timeout "$budget" sudo` prefix, so reading the parts
    // directory again in one of them is still rejected.
    let list = "-o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources";
    let parts = " -o Dir::Etc::sourceparts=/dev/null";
    for (line, command) in [
        (
            format!(
                "            timeout \"$budget\" sudo apt-get {list}{parts} update || status=$?\n"
            ),
            format!("apt-get {list} update"),
        ),
        (
            format!("              timeout \"$budget\" sudo apt-get {list}{parts} \\\n"),
            format!(
                "apt-get {list} install -y --no-install-recommends dbus gnome-keyring libsecret-tools"
            ),
        ),
    ] {
        let repo = Repository::with_workflows();
        repo.replace(
            ".github/workflows/native-tests.yml",
            &line,
            &line.replacen(parts, "", 1),
        );
        assert_eq!(
            repo.workflow_errors(),
            [format!(
                ".github/workflows/native-tests.yml: job shard step Install Ubuntu native secret-store fixture tools runs `{command}` over every configured apt source; name the needed list with one -o Dir::Etc::sourcelist=/..., make the last -o Dir::Etc::sourceparts=/dev/null, and pass no -c"
            )]
        );
    }
    // Naming a list is not enough while the parts directory is still read,
    // and the `apt` front end fetches the same way.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        RELEASE_APT,
        "          sudo apt -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources update; sudo apt-get -oDir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -oDir::Etc::sourceparts=/dev/null install -y dbus\n          sudo -E DEBIAN_FRONTEND=noninteractive apt-get upgrade\n",
    );
    let unrestricted = |command: &str| {
        format!(
            ".github/workflows/release.yml: job tests step Install Ubuntu native secret-store fixture tools runs `{command}` over every configured apt source; name the needed list with one -o Dir::Etc::sourcelist=/..., make the last -o Dir::Etc::sourceparts=/dev/null, and pass no -c"
        )
    };
    assert_eq!(
        repo.workflow_errors(),
        [
            unrestricted(
                "apt -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources update"
            ),
            unrestricted("apt-get upgrade"),
        ]
    );
    // A shell's -c string and add-apt-repository, which updates every
    // source unless told not to (add-apt-repository(1) on noble: `-n,
    // --no-update`), are read as the fetches they run.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        RELEASE_APT,
        "          sudo sh -c 'apt-get update'\n          bash --noprofile -ec \"apt-get -y upgrade\"\n          bash -o pipefail -c 'apt-get dist-upgrade'\n          sudo add-apt-repository -y universe\n          sudo add-apt-repository -yn universe\n          sudo apt-add-repository --no-update ppa:example/tools\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/release.yml: job tests step Install Ubuntu native secret-store fixture tools runs `add-apt-repository -y universe`, which refreshes every configured apt source; pass -n (--no-update) and update only the needed list".to_owned(),
            unrestricted("apt-get -y upgrade"),
            unrestricted("apt-get dist-upgrade"),
            unrestricted("apt-get update"),
        ]
    );
    // Prefix values and durations do not hide apt. apt applies settings in
    // order, so a later parts directory, a second list or a configuration
    // file can restore every source (apt-get(8), apt.conf(5)); its keys
    // ignore case.
    let repo = Repository::with_workflows();
    let restricted = "-o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -o Dir::Etc::sourceparts=/dev/null";
    repo.replace(
        ".github/workflows/release.yml",
        RELEASE_APT,
        &format!(
            "          sudo -u root apt-get update\n          timeout 300 sudo apt-get -q update\n          sudo apt-get {restricted} -o Dir::Etc::sourceparts=/etc/apt/sources.list.d update\n          sudo apt-get {restricted} -o Dir::Etc::sourcelist=/etc/apt/sources.list install -y dbus\n          sudo apt-get -c /tmp/apt.conf {restricted} upgrade\n          sudo apt-get -o dir::etc::SOURCELIST=/etc/apt/sources.list.d/ubuntu.sources --option=DIR::ETC::SOURCEPARTS=/dev/null -yo Dir::Etc::sourceparts=/dev/null update\n"
        ),
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            unrestricted(&format!("apt-get -c /tmp/apt.conf {restricted} upgrade")),
            unrestricted(&format!(
                "apt-get {restricted} -o Dir::Etc::sourcelist=/etc/apt/sources.list install -y dbus"
            )),
            unrestricted(&format!(
                "apt-get {restricted} -o Dir::Etc::sourceparts=/etc/apt/sources.list.d update"
            )),
            unrestricted("apt-get -q update"),
            unrestricted("apt-get update"),
        ]
    );
    // APT_CONFIG names a configuration file apt reads, at any level.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        RELEASE_APT,
        &format!("          APT_CONFIG=/tmp/apt.conf sudo -E apt-get {restricted} update\n"),
    );
    repo.replace(
        ".github/workflows/native-tests.yml",
        FIXED_EXEC,
        &format!("{FIXED_EXEC}  APT_CONFIG: /tmp/apt.conf\n"),
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/native-tests.yml: sets APT_CONFIG at workflow level; apt must read only the lists each fetch names",
            ".github/workflows/release.yml: job tests step Install Ubuntu native secret-store fixture tools sets APT_CONFIG; apt must read only the lists each fetch names",
        ]
    );
}

#[test]
fn incident_partition_download_is_rejected() {
    // Run 36453397286, Windows on Arm behavior partition 2: every partition
    // downloaded the Dolt Windows x64 archive itself, and one got HTTP 500.
    let repo = Repository::with_workflows();
    repo.replace(".github/workflows/native-tests.yml", PARTITION_OFFLINE, "");
    repo.replace(".github/workflows/ci.yml", PARTITION_OFFLINE, "");
    let offline = "must run with KURU_DOLT_BUNDLE_OFFLINE: \"true\" from its job env, or the workflow env without a job override, and import the bundle inputs an earlier job of the run verified";
    assert_eq!(
        repo.workflow_errors(),
        [
            format!(".github/workflows/ci.yml: partition job native-memory {offline}"),
            format!(".github/workflows/native-tests.yml: partition job shard {offline}"),
        ]
    );
    // A step cannot reopen the network for bundle preparation.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/native-tests.yml",
        "          KURU_COVERAGE_PACKAGES: kuru,",
        "          KURU_DOLT_BUNDLE_OFFLINE: \"false\"\n          KURU_COVERAGE_PACKAGES: kuru,",
    );
    repo.replace(
        ".github/workflows/ci.yml",
        "          cd -- \"$GITHUB_WORKSPACE\"\n",
        "          cd -- \"$GITHUB_WORKSPACE\"\n          unset KURU_DOLT_BUNDLE_OFFLINE\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/ci.yml: job native-memory step Import the run's verified bundle inputs overrides the partition's KURU_DOLT_BUNDLE_OFFLINE in its script",
            ".github/workflows/native-tests.yml: job shard step Run one checked uninstrumented Windows on Arm partition overrides the partition's KURU_DOLT_BUNDLE_OFFLINE",
        ]
    );
    // Renaming the axis does not hide a job that runs a partition task.
    let repo = Repository::with_workflows();
    repo.replace(".github/workflows/native-tests.yml", PARTITION_OFFLINE, "");
    repo.replace(
        ".github/workflows/native-tests.yml",
        "        partition: ${{ fromJSON",
        "        part: ${{ fromJSON",
    );
    assert_eq!(
        repo.workflow_errors(),
        [format!(
            ".github/workflows/native-tests.yml: partition job shard {offline}"
        )]
    );
    // The workflow env may supply the setting; a job override may not undo it.
    let repo = Repository::new();
    let fan_out = |job_env: &str| {
        format!(
            "on: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: \"false\"\n  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n  KURU_DOLT_BUNDLE_OFFLINE: \"true\"\njobs:\n  legs:\n    strategy:\n      matrix:\n        leg: [1, 2]\n    runs-on: ubuntu-latest{job_env}\n    steps:\n      - run: mise run //packages/kuru-delivery:test:partition\n"
        )
    };
    repo.write(".github/workflows/legs.yml", &fan_out(""));
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
    repo.write(
        ".github/workflows/legs.yml",
        &fan_out("\n    env:\n      KURU_DOLT_BUNDLE_OFFLINE: \"false\""),
    );
    assert_eq!(
        repo.workflow_errors(),
        [format!(
            ".github/workflows/legs.yml: partition job legs {offline}"
        )]
    );
    // A matrix without partitions is not a fan-out of the same inputs.
    let repo = Repository::new();
    repo.write(
        ".github/workflows/matrix.yml",
        "on: push\njobs:\n  build:\n    strategy:\n      matrix:\n        target: [a, b]\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo build\n  shard:\n    strategy:\n      matrix:\n        partition: [1, 2]\n    env:\n      KURU_DOLT_BUNDLE_OFFLINE: true\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo test\n",
    );
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
}

#[test]
fn every_partition_task_fan_out_must_be_offline() {
    // The check names the package's partition tasks; each task that runs
    // `coverage shard` must be recognized under any axis name.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mise.toml");
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(manifest).unwrap()).unwrap();
    let mut tasks: Vec<&str> = manifest["tasks"]
        .as_table()
        .unwrap()
        .iter()
        .filter(|(_, task)| {
            task.get("run")
                .and_then(toml::Value::as_str)
                .is_some_and(|run| run.contains("-- coverage shard"))
        })
        .map(|(name, _)| name.as_str())
        .collect();
    tasks.sort_unstable();
    assert_eq!(tasks, ["coverage:shard", "test:partition"]);
    let repo = Repository::new();
    for task in tasks {
        repo.write(
            ".github/workflows/legs.yml",
            &format!(
                "on: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: \"false\"\n  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\njobs:\n  legs:\n    strategy:\n      matrix:\n        slice: [1, 2]\n    runs-on: ubuntu-latest\n    steps:\n      - run: mise run //packages/kuru-delivery:{task}\n"
            ),
        );
        assert_eq!(
            repo.workflow_errors(),
            [
                ".github/workflows/legs.yml: partition job legs must run with KURU_DOLT_BUNDLE_OFFLINE: \"true\" from its job env, or the workflow env without a job override, and import the bundle inputs an earlier job of the run verified"
            ],
            "{task}"
        );
    }
}

const FIXED_TASK: &str = "  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n";

#[test]
fn task_auto_install_is_off_at_workflow_level_only() {
    // `mise run` installs every missing configured tool before a task unless
    // MISE_TASK_RUN_AUTO_INSTALL is false, the same download as run
    // 36319170835's `mise x`.
    let repo = Repository::with_workflows();
    repo.replace(".github/workflows/native-tests.yml", FIXED_TASK, "");
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/native-tests.yml: uses mise, so its workflow-level env must set MISE_TASK_RUN_AUTO_INSTALL: \"false\""
        ]
    );
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/ci.yml",
        "      MISE_LOCKED: \"1\"\n      MISE_NO_HOOKS: \"1\"\n      CARGO_INCREMENTAL: \"0\"\n      # Import",
        "      MISE_LOCKED: \"1\"\n      MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n      MISE_NO_HOOKS: \"1\"\n      CARGO_INCREMENTAL: \"0\"\n      # Import",
    );
    repo.replace(
        ".github/workflows/quality.yml",
        "      - run: mise run docs:check\n",
        "      - run: mise run docs:check\n        env:\n          MISE_TASK_RUN_AUTO_INSTALL: \"true\"\n      - run: export MISE_TASK_RUN_AUTO_INSTALL=true\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/ci.yml: job native-memory overrides MISE_TASK_RUN_AUTO_INSTALL; set it only in the workflow-level env",
            ".github/workflows/quality.yml: job docs step step 5 overrides MISE_TASK_RUN_AUTO_INSTALL; set it only in the workflow-level env",
            ".github/workflows/quality.yml: job docs step step 6 overrides MISE_TASK_RUN_AUTO_INSTALL in its script; set it only in the workflow-level env",
        ]
    );
}

#[test]
fn release_workflow_has_no_task_auto_install_exemption() {
    // release.yml is held to the same rule as every other workflow: its
    // notes and build-docs jobs, once exempted, name their tool installs.
    let task_off = "  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n\nconcurrency";
    let workflow_level = ".github/workflows/release.yml: uses mise, so its workflow-level env must set MISE_TASK_RUN_AUTO_INSTALL: \"false\"";
    let job_level = |job: &str| {
        format!(
            ".github/workflows/release.yml: job {job} overrides MISE_TASK_RUN_AUTO_INSTALL; set it only in the workflow-level env"
        )
    };
    let repo = Repository::with_workflows();
    repo.replace(".github/workflows/release.yml", task_off, "\nconcurrency");
    assert_eq!(repo.workflow_errors(), [workflow_level]);
    // The exemption's arrangement, every other job opting out in its own env
    // while notes and build-docs leave task auto-install on, is rejected.
    repo.replace(
        ".github/workflows/release.yml",
        "    needs: plan\n    runs-on: ubuntu-latest\n",
        "    needs: plan\n    runs-on: ubuntu-latest\n    env:\n      MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [job_level("bump"), workflow_level.to_owned()]
    );
    // Neither formerly exempted job may set it for itself, even to "false",
    // nor may one of its steps.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    steps:\n",
        "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    env:\n      MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n    steps:\n",
    );
    repo.replace(
        ".github/workflows/release.yml",
        "      contents: read\n      pages: read\n    steps:\n",
        "      contents: read\n      pages: read\n    env:\n      MISE_TASK_RUN_AUTO_INSTALL: \"true\"\n    steps:\n",
    );
    repo.replace(
        ".github/workflows/release.yml",
        "      - run: mise run //apps/kuru-docs:setup\n",
        "      - run: mise run //apps/kuru-docs:setup\n        env:\n          MISE_TASK_RUN_AUTO_INSTALL: \"true\"\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            job_level("build-docs"),
            ".github/workflows/release.yml: job build-docs step step 4 overrides MISE_TASK_RUN_AUTO_INSTALL; set it only in the workflow-level env".to_owned(),
            job_level("notes"),
        ]
    );
    // Their bare installs are rejected like any other job's.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "        run: mise run //packages/kuru-delivery:setup:test-tools\n      - name: Generate notes",
        "        run: mise install --include-task-tools\n      - name: Generate notes",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/release.yml: job notes step Install package-owned release tools runs `mise install` without naming the tools to install"
        ]
    );
}

impl Repository {
    fn lint_errors(&self) -> Vec<String> {
        self.errors()
            .into_iter()
            .filter(|error| !error.starts_with(".github/workflows/"))
            .collect()
    }
}

#[test]
fn actual_lint_configuration_is_the_only_one_and_is_never_switched_off() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let errors: Vec<String> = repo::check(&root)
        .unwrap()
        .into_iter()
        .filter(|error| error.contains("clippy") || error.contains("disallowed"))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn package_clippy_configuration_would_shadow_the_root_and_is_rejected() {
    let repo = Repository::new();
    repo.write(
        "packages/kuru-core/clippy.toml",
        "disallowed-methods = []\n",
    );
    repo.write("apps/.clippy.toml", "");
    repo.write("packages/kuru-core/src/nested/clippy.toml", "");
    // Build output and installed Node packages are never read.
    repo.write("packages/kuru-core/target/clippy.toml", "");
    repo.write("apps/kuru-docs/node_modules/x/clippy.toml", "");
    assert_eq!(
        repo.lint_errors(),
        [
            "apps/.clippy.toml: Clippy would use this file instead of the root clippy.toml and drop its bans; keep lint configuration in the root file",
            "packages/kuru-core/clippy.toml: Clippy would use this file instead of the root clippy.toml and drop its bans; keep lint configuration in the root file",
            "packages/kuru-core/src/nested/clippy.toml: Clippy would use this file instead of the root clippy.toml and drop its bans; keep lint configuration in the root file",
        ]
    );
    // The real CLI fails on it too.
    let output = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"))
        .args(["repo", "--root"])
        .arg(repo.0.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("packages/kuru-core/clippy.toml"));
}

#[test]
fn crate_and_module_allowances_of_the_ban_are_rejected_but_statements_are_not() {
    let repo = Repository::new();
    repo.write(
        "packages/kuru-core/src/lib.rs",
        "#![allow(clippy::disallowed_methods)]\n#[expect(clippy::style, reason = \"x\")]\nmod quiet;\n",
    );
    repo.write(
        "apps/kuru-tui/src/main.rs",
        "fn main() {\n    #[expect(clippy::disallowed_methods, reason = \"reviewed\")]\n    call();\n    let _ = \"#![allow(clippy::all)]\";\n}\n",
    );
    repo.write(
        "packages/kuru-core/target/debug/build.rs",
        "#![allow(warnings)]\n",
    );
    assert_eq!(
        repo.lint_errors(),
        [
            "packages/kuru-core/src/lib.rs:1: `#![allow(clippy::disallowed_methods)]` switches off every disallowed-methods ban for a whole crate or module; expect clippy::disallowed_methods on the reviewed statement instead",
            "packages/kuru-core/src/lib.rs:2: `#[expect(clippy::style, reason = \" \")]` switches off every disallowed-methods ban for a whole crate or module; expect clippy::disallowed_methods on the reviewed statement instead",
        ]
    );
}

#[test]
fn cargo_lint_tables_cannot_lower_the_ban() {
    let repo = Repository::new();
    repo.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"apps/kuru-tui\", \"packages/kuru-core\"]\n[workspace.dependencies]\nserde = \"=1.0.229\"\nkuru-core = { path = \"packages/kuru-core\" }\n[workspace.lints.clippy]\nstyle = { level = \"allow\", priority = -1 }\npedantic = \"allow\"\n",
    );
    repo.write(
        "packages/kuru-core/Cargo.toml",
        "[package]\nname = \"kuru-core\"\n[lints.clippy]\ndisallowed-methods = \"expect\"\n[lints.rust]\nwarnings = \"allow\"\ndead_code = \"allow\"\n",
    );
    assert_eq!(
        repo.lint_errors(),
        [
            "Cargo.toml: [workspace.lints.clippy] style switches off every disallowed-methods ban; expect clippy::disallowed_methods on the reviewed statement instead",
            "packages/kuru-core/Cargo.toml: [lints.clippy] disallowed-methods switches off every disallowed-methods ban; expect clippy::disallowed_methods on the reviewed statement instead",
            "packages/kuru-core/Cargo.toml: [lints.rust] warnings switches off every disallowed-methods ban; expect clippy::disallowed_methods on the reviewed statement instead",
        ]
    );
}

#[test]
fn configuration_cannot_redirect_or_allow_the_ban_from_the_command_line() {
    let repo = Repository::new();
    repo.replace(
        "packages/kuru-core/mise.toml",
        "run = \"cargo test -p kuru-core\"",
        "env.CLIPPY_CONF_DIR = \"/elsewhere\"\n[tasks.lint]\nrun = \"cargo clippy -p kuru-core -- -D warnings -A clippy::disallowed_methods\"",
    );
    repo.write(
        ".cargo/config.toml",
        "[build]\nrustflags = [\"--cap-lints\", \"warn\"]\n",
    );
    repo.write(
        ".github/workflows/lint.yml",
        "on: push\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo clippy -- -Aclippy::all\n",
    );
    let errors = repo.lint_errors();
    assert_eq!(
        errors,
        [
            ".cargo/config.toml: passes --cap-lints, which lowers every disallowed-methods ban",
            "packages/kuru-core/mise.toml: allows clippy::disallowed_methods on the command line, which switches off every disallowed-methods ban",
            "packages/kuru-core/mise.toml: sets CLIPPY_CONF_DIR, which makes Clippy read another configuration instead of the root clippy.toml",
        ]
    );
    assert!(repo.errors().iter().any(|error| error
        == ".github/workflows/lint.yml: allows clippy::all on the command line, which switches off every disallowed-methods ban"));
}

#[test]
fn package_cargo_configuration_and_inline_assignments_cannot_allow_the_ban() {
    let repo = Repository::new();
    // Package tasks run in the package directory, and Cargo reads every
    // `.cargo/config{,.toml}` from there up.
    repo.write(
        "packages/kuru-core/.cargo/config.toml",
        "[build]\nrustflags = [\"-Aclippy::disallowed_methods\"]\n",
    );
    repo.write(
        "apps/.cargo/config",
        "[target.x86_64-pc-windows-msvc]\nrustflags = [\"--allow\", \"clippy::style\"]\n",
    );
    repo.write(
        "apps/kuru-tui/src/.cargo/config.toml",
        "[env]\nCLIPPY_CONF_DIR = \"/elsewhere\"\n",
    );
    repo.write(
        "packages/kuru-core/.cargo/notes.toml",
        "rustflags = [\"-Awarnings\"]\n",
    );
    repo.replace(
        "packages/kuru-core/mise.toml",
        "run = \"cargo test -p kuru-core\"",
        "run = \"RUSTFLAGS=-Aclippy::disallowed_methods cargo clippy -p kuru-core\"",
    );
    assert_eq!(
        repo.lint_errors(),
        [
            "apps/.cargo/config: allows clippy::style on the command line, which switches off every disallowed-methods ban",
            "apps/kuru-tui/src/.cargo/config.toml: sets CLIPPY_CONF_DIR, which makes Clippy read another configuration instead of the root clippy.toml",
            "packages/kuru-core/.cargo/config.toml: allows clippy::disallowed_methods on the command line, which switches off every disallowed-methods ban",
            "packages/kuru-core/mise.toml: allows clippy::disallowed_methods on the command line, which switches off every disallowed-methods ban",
        ]
    );
}

#[test]
fn item_allowances_outer_allows_and_unexplained_expectations_are_rejected() {
    let repo = Repository::new();
    repo.write(
        "apps/kuru-tui/src/capture.rs",
        concat!(
            "#[allow(clippy::disallowed_methods)]\n",
            "pub(crate) fn capture<T>(subscriber: Registry, f: impl FnOnce() -> T) -> T {\n",
            "    tracing::subscriber::with_default(subscriber, f)\n",
            "}\n",
            "#[expect(clippy::disallowed_methods, reason = \"reviewed\")]\n",
            "impl Capture {}\n",
            "#[cfg_attr(test, expect(clippy::all, reason = \"reviewed\"))]\n",
            "unsafe trait Quiet {}\n",
            "fn body() {\n",
            "    #[allow(clippy::disallowed_methods, reason = \"reviewed\")]\n",
            "    call();\n",
            "    #[expect(clippy::disallowed_methods)]\n",
            "    call();\n",
            "    #[expect(clippy::disallowed_methods, reason = \"reviewed\")]\n",
            "    call();\n",
            "    #[allow(dead_code, reason = \"unrelated\")]\n",
            "    let unused = 1;\n",
            "}\n",
        ),
    );
    assert_eq!(
        repo.lint_errors(),
        [
            "apps/kuru-tui/src/capture.rs:10: `#[allow(clippy::disallowed_methods, reason = \" \")]` allows a disallowed-methods ban and stays silent once the call is gone; use `expect` with a reason on the reviewed statement instead",
            "apps/kuru-tui/src/capture.rs:12: `#[expect(clippy::disallowed_methods)]` expects a disallowed-methods ban without a reason; add `reason = \"...\"` saying why the statement is safe",
            "apps/kuru-tui/src/capture.rs:1: `#[allow(clippy::disallowed_methods)]` on the `fn` item switches off every disallowed-methods ban for every statement in it; expect clippy::disallowed_methods on the reviewed statement instead",
            "apps/kuru-tui/src/capture.rs:5: `#[expect(clippy::disallowed_methods, reason = \" \")]` on the `impl` item switches off every disallowed-methods ban for every statement in it; expect clippy::disallowed_methods on the reviewed statement instead",
            "apps/kuru-tui/src/capture.rs:7: `#[cfg_attr(test, expect(clippy::all, reason = \" \"))]` on the `trait` item switches off every disallowed-methods ban for every statement in it; expect clippy::disallowed_methods on the reviewed statement instead",
        ]
    );
}

#[test]
fn root_configuration_must_exist_ban_every_required_method_and_give_reasons() {
    let repo = Repository::new();
    fs::remove_file(repo.0.path().join("clippy.toml")).unwrap();
    assert_eq!(
        repo.lint_errors(),
        [
            "clippy.toml: the root Clippy configuration is missing; it holds the workspace's disallowed-methods bans"
        ]
    );
    repo.write(
        "clippy.toml",
        &ROOT_CLIPPY.replace(
            "{ path = \"tracing::subscriber::set_default\", reason = ",
            "\"tracing::subscriber::set_default\", { path = \"unused\", reason = ",
        ),
    );
    repo.replace(
        "clippy.toml",
        "{ path = \"tracing_subscriber::util::SubscriberInitExt::init\", reason = \"",
        "{ path = \"tracing_subscriber::util::SubscriberInitExt::init\", reason = \"  \", old = \"",
    );
    repo.replace(
        "clippy.toml",
        "{ path = \"tracing::dispatcher::with_default\"",
        "{ path = \"tracing::dispatcher::with_defaults\"",
    );
    assert_eq!(
        repo.lint_errors(),
        [
            "clippy.toml: disallowed-methods entry 1 needs a `path` and a non-empty `reason` naming the safe alternative",
            "clippy.toml: disallowed-methods entry 10 needs a `path` and a non-empty `reason` naming the safe alternative",
            "clippy.toml: disallowed-methods must ban tracing::dispatcher::with_default",
        ]
    );
    repo.write("clippy.toml", "avoid-breaking-exported-api = false\n");
    assert_eq!(repo.lint_errors().len(), 9);
}
