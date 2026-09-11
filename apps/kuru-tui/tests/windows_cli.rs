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

fn source_entrypoint_fixture(root: &Path) -> Command {
    let tools = root.join("tools");
    let scripts = root.join("checkout & spaced 日本語").join("scripts");
    fs::create_dir(&tools).unwrap();
    fs::create_dir_all(&scripts).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("mise.exe"),
    )
    .unwrap();
    // Execute the unmodified public entrypoint in an isolated checkout layout.
    let entrypoint = scripts.join("install.ps1");
    fs::write(&entrypoint, include_bytes!("../../../scripts/install.ps1")).unwrap();
    let probe = root.join("source-entrypoint-probe.ps1");
    fs::write(
        &probe,
        r#"$ErrorActionPreference = 'Stop'
function Snapshot {
    return @{
        no_hooks = [Environment]::GetEnvironmentVariable('MISE_NO_HOOKS', 'Process')
        auto_install = [Environment]::GetEnvironmentVariable('MISE_TASK_RUN_AUTO_INSTALL', 'Process')
        install_dir = [Environment]::GetEnvironmentVariable('KURU_INSTALL_DIR', 'Process')
    }
}
$before = Snapshot
$failure = $null
try {
    if ($env:KURU_ENTRYPOINT_INVALID) {
        & $env:KURU_ENTRYPOINT_SCRIPT -Source -Version '0.2.0' -InstallDir $env:KURU_ENTRYPOINT_INSTALL
    } else {
        & $env:KURU_ENTRYPOINT_SCRIPT -Source -InstallDir $env:KURU_ENTRYPOINT_INSTALL
    }
} catch {
    $failure = $_.Exception.Message
} finally {
    $report = @{ before = $before; after = (Snapshot); failure = $failure }
    [IO.File]::WriteAllText($env:KURU_ENTRYPOINT_REPORT, ($report | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
}
if ($null -ne $failure) { Write-Output $failure; exit 1 }
"#,
    )
    .unwrap();
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let mut child = command(root, &powershell);
    child
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env("KURU_ENTRYPOINT_SCRIPT", &entrypoint)
        .env("KURU_ENTRYPOINT_INSTALL", ".\\installed & 日本語")
        .env("KURU_ENTRYPOINT_REPORT", root.join("restoration.json"))
        .env("KURU_CLI_FIXTURE_SETUP_LOG", root.join("setup.jsonl"))
        .env("KURU_CLI_FIXTURE_SETUP_EXIT", "0")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&probe);
    child
}

#[test]
fn source_install_entrypoint_scopes_first_mise_and_restores_environment_on_success_or_failure() {
    for opposing in [false, true] {
        for status in [0, 23] {
            let root = tempfile::tempdir().unwrap();
            let mut child = source_entrypoint_fixture(root.path());
            child.env("KURU_CLI_FIXTURE_SETUP_EXIT", status.to_string());
            let before = if opposing {
                child
                    .env("MISE_NO_HOOKS", "0")
                    .env("MISE_TASK_RUN_AUTO_INSTALL", "true")
                    .env("KURU_INSTALL_DIR", "caller relative destination 日本語");
                serde_json::json!({
                    "no_hooks": "0", "auto_install": "true",
                    "install_dir": "caller relative destination 日本語",
                })
            } else {
                serde_json::json!({"no_hooks": null, "auto_install": null, "install_dir": null})
            };
            let output = child.output().unwrap();
            let diagnostic = format!(
                "opposing={opposing}, status={status}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.status.success(), status == 0, "{diagnostic}");
            let calls: Vec<serde_json::Value> = fs::read_to_string(root.path().join("setup.jsonl"))
                .expect(&diagnostic)
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(calls.len(), 1, "{diagnostic}");
            let actual = &calls[0];
            assert_eq!(actual["no_hooks"], "1", "{diagnostic}");
            assert_eq!(actual["auto_install"], "false", "{diagnostic}");
            let arguments = actual["arguments"].as_array().unwrap();
            assert_eq!(arguments.len(), 4);
            assert_eq!(arguments[0], "-C");
            assert_eq!(
                Path::new(arguments[1].as_str().unwrap())
                    .canonicalize()
                    .unwrap(),
                root.path()
                    .join("checkout & spaced 日本語")
                    .canonicalize()
                    .unwrap()
            );
            assert_eq!(arguments[2], "run");
            assert_eq!(arguments[3], "//apps/kuru-tui:install");
            let selected = Path::new(actual["install_dir"].as_str().unwrap());
            assert!(selected.is_absolute());
            assert_eq!(selected.file_name().unwrap(), "installed & 日本語");
            assert_eq!(
                selected.parent().unwrap().canonicalize().unwrap(),
                root.path().canonicalize().unwrap()
            );
            let report: serde_json::Value = serde_json::from_slice(
                &fs::read(root.path().join("restoration.json")).expect(&diagnostic),
            )
            .unwrap();
            assert_eq!(report["before"], before, "{diagnostic}");
            assert_eq!(report["after"], before, "{diagnostic}");
            if status == 0 {
                assert!(report["failure"].is_null(), "{diagnostic}");
            } else {
                assert_eq!(report["failure"], "Source installation failed (23).");
            }
        }
    }
}

#[test]
fn source_install_entrypoint_rejects_release_options_before_mise_or_environment_changes() {
    let root = tempfile::tempdir().unwrap();
    let output = source_entrypoint_fixture(root.path())
        .env("KURU_ENTRYPOINT_INVALID", "1")
        .env("MISE_NO_HOOKS", "0")
        .env("MISE_TASK_RUN_AUTO_INSTALL", "true")
        .env("KURU_INSTALL_DIR", "caller relative destination")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("setup.jsonl").exists());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("restoration.json")).unwrap()).unwrap();
    let before = serde_json::json!({
        "no_hooks": "0", "auto_install": "true", "install_dir": "caller relative destination",
    });
    assert_eq!(report["before"], before);
    assert_eq!(report["after"], before);
    assert_eq!(
        report["failure"],
        "Source installation does not accept release-selection or recovery options."
    );
}
