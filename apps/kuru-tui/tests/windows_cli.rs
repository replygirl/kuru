#![cfg(windows)]

use kuru_delivery::command::BlockingCommand as Command;
use kuru_platform::fs::regular_file_info;
use std::{
    fs::{self, File},
    path::Path,
};

fn command(root: &Path, binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .current_dir(root)
        .env("USERPROFILE", root.join("home"))
        .env("APPDATA", root.join("config"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("TMP", root)
        .env("TEMP", root)
        .env("PATH", root.join("tools"))
        .env("KURU_CLI_FIXTURE_LOG", root.join("actions.jsonl"));
    for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn success(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn all_authentication_actions_use_the_native_provider_command_without_token_access() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("tools");
    fs::create_dir(&tools).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("codex.exe"),
    )
    .unwrap();
    let project = root.path().join("workspace");
    fs::create_dir(&project).unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_kuru"));
    for arguments in [
        vec!["login"],
        vec!["login", "--device"],
        vec!["auth"],
        vec!["logout"],
    ] {
        success(
            command(root.path(), binary)
                .arg("-C")
                .arg(&project)
                .args(arguments),
        );
    }
    let actual: Vec<Vec<String>> = fs::read_to_string(root.path().join("actions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        actual,
        [
            vec!["login"],
            vec!["login", "--device-auth"],
            vec!["login", "status"],
            vec!["logout"]
        ]
    );
    assert!(!root.path().join("local/kuru/memory").exists());
}

#[test]
fn source_update_builds_through_mise_and_publishes_after_the_trusted_build() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("tools");
    let install = root.path().join("installed 日本語");
    let checkout = root.path().join("checkout & spaced 日本語");
    let target = root.path().join("target with spaces");
    for directory in [&tools, &install, &checkout] {
        fs::create_dir(directory).unwrap();
    }
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("mise.exe"),
    )
    .unwrap();
    let binary = install.join("kuru.exe");
    fs::copy(env!("CARGO_BIN_EXE_kuru"), &binary).unwrap();
    let before = regular_file_info(&File::open(&binary).unwrap())
        .unwrap()
        .identity;
    let mut update = command(root.path(), &binary);
    update
        .env("KURU_CLI_FIXTURE_BINARY", env!("CARGO_BIN_EXE_kuru"))
        .env("KURU_CLI_FIXTURE_TARGET", &target)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_TARGET", "host")
        .env("git_dir", root.path().join("unrelated repository"))
        .env("GIT_WORK_TREE", root.path().join("unrelated work tree"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_SSH_COMMAND", "fixture-ssh-command")
        .env("GIT_CONFIG_GLOBAL", "fixture-global-config")
        .args(["update", "--source"])
        .arg(&checkout);
    success(&mut update);
    assert_ne!(
        regular_file_info(&File::open(&binary).unwrap())
            .unwrap()
            .identity,
        before
    );
    let actions: Vec<Vec<String>> = fs::read_to_string(root.path().join("actions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(actions.len(), 2);
    let source = checkout
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(actions[0], ["-C", &source, "install", "rust"]);
    assert_eq!(
        actions[1],
        [
            "-C",
            &source,
            "run",
            "//apps/kuru-tui:build:release",
            "--",
            "--target",
            "x86_64-pc-windows-msvc"
        ]
    );
    success(command(root.path(), &binary).arg("--version"));
    let output = command(root.path(), &binary)
        .args(["update", "--version", "0.2.0", "--source"])
        .arg(&checkout)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("actions.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2,
        "invalid invocation must not build or update"
    );
}
