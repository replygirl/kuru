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

/// The fixed apt step text in native-tests.yml and release.yml.
const FIXED_APT: &str = "          # These packages come from the Ubuntu archive. apt reads only its deb822
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
        "        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - name: Install coverage components",
        "        env:\n          GITHUB_TOKEN: ${{ github.token }}\n          MISE_EXEC_AUTO_INSTALL: \"true\"\n      - name: Install coverage components",
    );
    repo.replace(
        ".github/workflows/ci.yml",
        "        run: printf 'KURU_DOLT_BUNDLE_DIR=%s/kuru-bundles\\n' \"$RUNNER_TEMP\" >> \"$GITHUB_ENV\"\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          experimental: true\n          version: 2026.9.4\n          install_args: rust\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2\n        with:\n          cache-bin: false\n          shared-key: bundle-inputs",
        "        run: echo 'MISE_EXEC_AUTO_INSTALL=true' >> \"$GITHUB_ENV\"\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          experimental: true\n          version: 2026.9.4\n          install_args: rust\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2\n        with:\n          cache-bin: false\n          shared-key: bundle-inputs",
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
    let workflow = "name: Fixture\non: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: false\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c # v4.3.0\n        with:\n          install_args: rust\n      - name: Selected tools\n        run: |\n          mise install --locked rust aqua:rhysd/actionlint\n          MISE_LOCKED=1 mise i cargo:cargo-audit\n";
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
            ".github/workflows/plain.yml: uses mise, so its workflow-level env must set MISE_EXEC_AUTO_INSTALL: \"false\""
        ]
    );
    // Prefix options do not hide the program.
    repo.write(
        ".github/workflows/plain.yml",
        "on: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: \"false\"\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: sudo -E mise install\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/plain.yml: job lint step step 1 runs `mise install` without naming the tools to install"
        ]
    );
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
    for name in ["native-tests.yml", "release.yml"] {
        repo.replace(
            &format!(".github/workflows/{name}"),
            FIXED_APT,
            INCIDENT_APT,
        );
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
        .map(|command| format!(".github/workflows/{name}: job {job} step Install Ubuntu native secret-store fixture tools runs `{command}` over every configured apt source; name the needed list with -o Dir::Etc::sourcelist=/... and -o Dir::Etc::sourceparts=/dev/null"))
    })
    .collect();
    assert_eq!(errors, expected);
    // Naming a list is not enough while the parts directory is still read,
    // and the `apt` front end fetches the same way.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        FIXED_APT,
        "          sudo apt -o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources update; sudo apt-get -oDir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -oDir::Etc::sourceparts=/dev/null install -y dbus\n          sudo -E DEBIAN_FRONTEND=noninteractive apt-get upgrade\n",
    );
    let unrestricted = |command: &str| {
        format!(
            ".github/workflows/release.yml: job tests step Install Ubuntu native secret-store fixture tools runs `{command}` over every configured apt source; name the needed list with -o Dir::Etc::sourcelist=/... and -o Dir::Etc::sourceparts=/dev/null"
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
}

#[test]
fn incident_partition_download_is_rejected() {
    // Run 36453397286, Windows on Arm behavior partition 2: every partition
    // downloaded the Dolt Windows x64 archive itself, and one got HTTP 500.
    let repo = Repository::with_workflows();
    repo.replace(".github/workflows/native-tests.yml", PARTITION_OFFLINE, "");
    repo.replace(".github/workflows/ci.yml", PARTITION_OFFLINE, "");
    let offline = "must set KURU_DOLT_BUNDLE_OFFLINE: \"true\" in its job env and import the bundle inputs an earlier job of the run verified";
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
    // A matrix without partitions is not a fan-out of the same inputs.
    let repo = Repository::new();
    repo.write(
        ".github/workflows/matrix.yml",
        "on: push\njobs:\n  build:\n    strategy:\n      matrix:\n        target: [a, b]\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo build\n  shard:\n    strategy:\n      matrix:\n        partition: [1, 2]\n    env:\n      KURU_DOLT_BUNDLE_OFFLINE: true\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo test\n",
    );
    assert!(repo.errors().is_empty(), "{:?}", repo.errors());
}
