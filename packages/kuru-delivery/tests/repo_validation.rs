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
    // value options are mise 2026.9.4's global `-C/--cd`, `-E/--env` and
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
    // given none (mise 2026.9.4 `docs/cli/upgrade.md`, `docs/cli/bootstrap.md`:
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
        .map(|command| format!(".github/workflows/{name}: job {job} step Install Ubuntu native secret-store fixture tools runs `{command}` over every configured apt source; name the needed list with one -o Dir::Etc::sourcelist=/..., make the last -o Dir::Etc::sourceparts=/dev/null, and pass no -c"))
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
        FIXED_APT,
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
        FIXED_APT,
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
        FIXED_APT,
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

/// The one exemption's reason, as every finding about it quotes it.
const RELEASE_EXEMPTION: &str = "these release jobs still install every missing configured tool through `mise run`, and notes' setup task runs a bare `mise install --include-task-tools`; follow-up release-notes-docs-tool-scope scopes their tool installation, a release workflow change for the maintainer to decide";
/// The reviewed digests of release.yml's exempted jobs.
const NOTES: &str = "00e5f2b74cdf5e2362c2c9a41d16f3e3d6ef3538f59502563311da4098270f7c";
const BUILD_DOCS: &str = "0c2d21f040bfd0a778d30ef7ef48865716dcd87e00cc3c7a3e2d5356b9f50766";

impl Repository {
    /// The workflow findings, with each changed job's new digest, which the
    /// finding reports for the reviewer, replaced by `<new>`.
    fn exemption_errors(&self) -> Vec<String> {
        const NOW: &str = "and it is now SHA-256 ";
        self.workflow_errors()
            .into_iter()
            .map(|error| match error.split_once(NOW) {
                Some((before, after)) => {
                    let (digest, rest) = after.split_at(64);
                    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()), "{error}");
                    format!("{before}{NOW}<new>{rest}")
                }
                None => error,
            })
            .collect()
    }
}

#[test]
fn release_task_auto_install_exemption_covers_only_two_reviewed_jobs() {
    // release.yml's notes and build-docs still run `mise run` with task
    // auto-install on. They are exempted by name, each pinned by the digest
    // of its whole parsed job and the workflow env and defaults it inherits;
    // every other release job that uses mise opts out itself.
    let changed = |job: &str, now: &str| {
        let digest = if job == "notes" { NOTES } else { BUILD_DOCS };
        format!(
            ".github/workflows/release.yml: job {job} is exempted from MISE_TASK_RUN_AUTO_INSTALL only as reviewed (SHA-256 {digest} of the job with the workflow env and defaults it inherits), and it is now {now}; re-review the exemption: scope the job's tool installation and remove the exemption, or review the whole changed job and record its new digest ({RELEASE_EXEMPTION})"
        )
    };
    let shape = |job: &str| changed(job, "SHA-256 <new>");
    assert!(Repository::with_workflows().exemption_errors().is_empty());
    // Another task run, other tools or a job-level opt-out change the job.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "        run: mise run //packages/kuru-delivery:setup\n",
        "        run: |\n          mise run //packages/kuru-delivery:setup\n          mise run lint\n",
    );
    repo.replace(
        ".github/workflows/release.yml",
        "          install_args: rust aqua:jdx/hk\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - run: mise run //apps/kuru-docs:setup\n",
        "          install_args: rust\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - run: mise run //apps/kuru-docs:setup\n",
    );
    assert_eq!(
        repo.exemption_errors(),
        [shape("build-docs"), shape("notes")]
    );
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    steps:\n",
        "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    env:\n      MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n    steps:\n",
    );
    assert_eq!(repo.exemption_errors(), [shape("notes")]);
    // Every other part of the job, and what it inherits, is pinned too: X1
    // another mise-action input that adds tools, X2 a job env entry that
    // loads another configuration, X3 the mise version, then a step's shell,
    // working directory, condition and env, and a workflow env or defaults
    // entry.
    let notes_action = "          version: 2026.9.4\n          install_args: rust aqua:jdx/hk\n        env:\n          GITHUB_TOKEN: ${{ github.token }}\n      - name: Install package-owned release tools\n";
    for (old, new) in [
        (
            notes_action,
            notes_action.replace(
                "install_args: rust aqua:jdx/hk\n",
                "install_args: rust aqua:jdx/hk\n          mise_toml: |\n            [tools]\n            node = \"24\"\n",
            ),
        ),
        (
            "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    steps:\n",
            "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    env:\n      MISE_ENV: release\n    steps:\n".to_owned(),
        ),
        (
            notes_action,
            notes_action.replace("version: 2026.9.4", "version: latest"),
        ),
        (
            "        run: mise run //packages/kuru-delivery:setup\n",
            "        shell: bash\n        working-directory: packages/kuru-delivery\n        run: mise run //packages/kuru-delivery:setup\n".to_owned(),
        ),
        (
            "        run: mise run //packages/kuru-delivery:setup\n",
            "        if: always()\n        run: mise run //packages/kuru-delivery:setup\n".to_owned(),
        ),
        (
            "      - name: Install package-owned release tools\n        env:\n",
            "      - name: Install package-owned release tools\n        env:\n          MISE_ENV: release\n".to_owned(),
        ),
    ] {
        let repo = Repository::with_workflows();
        repo.replace(".github/workflows/release.yml", old, &new);
        assert_eq!(repo.exemption_errors(), [shape("notes")], "{new}");
    }
    for inherited in [
        "  MISE_EXEC_AUTO_INSTALL: \"false\"\n  MISE_ENV: release\n",
        "  MISE_EXEC_AUTO_INSTALL: \"false\"\n\ndefaults:\n  run:\n    working-directory: packages\n",
    ] {
        let repo = Repository::with_workflows();
        repo.replace(
            ".github/workflows/release.yml",
            "  MISE_EXEC_AUTO_INSTALL: \"false\"\n",
            inherited,
        );
        assert_eq!(
            repo.exemption_errors(),
            [shape("build-docs"), shape("notes")],
            "{inherited}"
        );
    }
    // Comments and formatting are not part of the reviewed job.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "  notes:\n    # Generate read-only notes",
        "  notes:\n    # A reworded comment.\n    # Generate read-only notes",
    );
    repo.replace(
        ".github/workflows/release.yml",
        "    needs: [plan, bump]\n    runs-on: ubuntu-latest\n    steps:\n",
        "    needs: [ plan, bump ]\n    runs-on: 'ubuntu-latest'\n    steps:\n",
    );
    assert!(
        repo.exemption_errors().is_empty(),
        "{:?}",
        repo.exemption_errors()
    );
    let unexempted = |job: &str| {
        format!(
            ".github/workflows/release.yml: job {job} uses mise and is not exempted, so its job env must set MISE_TASK_RUN_AUTO_INSTALL: \"false\" while the workflow level does not ({RELEASE_EXEMPTION})"
        )
    };
    // A missing job is a stale exemption, and a renamed one is not exempted.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "\n  notes:\n",
        "\n  release-notes:\n",
    );
    assert_eq!(
        repo.exemption_errors(),
        [changed("notes", "missing"), unexempted("release-notes")]
    );
    // No other release job may leave task auto-install on.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "      MISE_NO_HOOKS: \"1\"\n      MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n      CARGO_INCREMENTAL: \"0\"\n      CARGO_PROFILE_DEV_DEBUG: line-tables-only\n      # Keep the genuine",
        "      MISE_NO_HOOKS: \"1\"\n      MISE_TASK_RUN_AUTO_INSTALL: \"true\"\n      CARGO_INCREMENTAL: \"0\"\n      CARGO_PROFILE_DEV_DEBUG: line-tables-only\n      # Keep the genuine",
    );
    repo.replace(
        ".github/workflows/release.yml",
        "\n  deploy-docs:\n",
        "\n  extra:\n    runs-on: ubuntu-latest\n    steps:\n      - run: mise run lint\n\n  deploy-docs:\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [unexempted("extra"), unexempted("tests")]
    );
    // A step still cannot override it, even in an exempted job (X4), and
    // the change is also one to the reviewed job.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "      - run: mise run //apps/kuru-docs:setup\n",
        "      - run: mise run //apps/kuru-docs:setup\n        env:\n          MISE_TASK_RUN_AUTO_INSTALL: \"true\"\n",
    );
    assert_eq!(
        repo.exemption_errors(),
        [
            shape("build-docs"),
            ".github/workflows/release.yml: job build-docs step step 3 overrides MISE_TASK_RUN_AUTO_INSTALL; set it only in the workflow-level env".to_owned(),
        ]
    );
    // Setting it for the whole workflow ends the exemption, which must go.
    let repo = Repository::with_workflows();
    repo.replace(
        ".github/workflows/release.yml",
        "  MISE_EXEC_AUTO_INSTALL: \"false\"\n\nconcurrency",
        "  MISE_EXEC_AUTO_INSTALL: \"false\"\n  MISE_TASK_RUN_AUTO_INSTALL: \"false\"\n\nconcurrency",
    );
    assert_eq!(
        repo.workflow_errors(),
        [format!(
            ".github/workflows/release.yml: sets MISE_TASK_RUN_AUTO_INSTALL at workflow level, so the exemption for jobs notes, build-docs is stale; remove it ({RELEASE_EXEMPTION})"
        )]
    );
    // The exemption is release.yml's alone.
    let repo = Repository::new();
    repo.write(
        ".github/workflows/notes.yml",
        "on: push\nenv:\n  MISE_EXEC_AUTO_INSTALL: \"false\"\njobs:\n  notes:\n    runs-on: ubuntu-latest\n    steps:\n      - run: mise run //packages/kuru-delivery:setup\n",
    );
    assert_eq!(
        repo.workflow_errors(),
        [
            ".github/workflows/notes.yml: uses mise, so its workflow-level env must set MISE_TASK_RUN_AUTO_INSTALL: \"false\""
        ]
    );
}
