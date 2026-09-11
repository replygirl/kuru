#![cfg(windows)]

use kuru_delivery::command::BlockingCommand as Command;
use kuru_platform::fs::regular_file_info;
use sha2::{Digest, Sha256};
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

#[tokio::test]
async fn pe_inspection_uses_native_paths_and_real_msvc_imports() {
    use std::io::Read;

    let root = kuru_memory::test_support::tempdir().unwrap();
    let tools = root.path().join("tools");
    let images = root.path().join("PE images 日本語");
    fs::create_dir(&tools).unwrap();
    fs::create_dir(&images).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_kuru-cli-windows-fixture"),
        tools.join("mise.exe"),
    )
    .unwrap();
    let config = kuru_core::MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let engine =
        kuru_memory::provision::provision(&config, &kuru_memory::test_support::cache_dir())
            .await
            .unwrap()
            .canonicalize()
            .unwrap();
    let evidence = |path: &Path| {
        let mut file = File::open(path).unwrap();
        let info = regular_file_info(&file).unwrap();
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let length = file.read(&mut buffer).unwrap();
            if length == 0 {
                break;
            }
            digest.update(&buffer[..length]);
        }
        (file, info, digest.finalize().to_vec())
    };
    let engine_before = evidence(&engine);
    // Instrumented development Kuru may use a dynamic CRT. Both fixture slots
    // instead inspect the actual verified static-runtime Dolt PE. Shipping CI
    // separately inspects the source-installed Kuru and real owning prefetch.
    let binary = images.join("copied engine.exe");
    assert_eq!(fs::copy(&engine, &binary).unwrap(), engine_before.1.len);
    let binary_before = evidence(&binary);
    assert_eq!(binary_before.2, engine_before.2);
    assert_ne!(binary_before.1.identity, engine_before.1.identity);
    let invalid = images.join("not a PE.exe");
    fs::write(&invalid, b"This is not a Portable Executable.\n").unwrap();
    let invalid_before = evidence(&invalid);

    let script = root.path().join("verify-windows-imports.ps1");
    fs::write(
        &script,
        include_bytes!("../support/verify-windows-imports.ps1"),
    )
    .unwrap();
    let launcher = root.path().join("inspect-pe.ps1");
    fs::write(
        &launcher,
        r#"$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) { throw 'expected stock PowerShell 5.1' }
# Native Rust fixture output and exact Unicode result assertions use UTF-8.
# Keep this ASCII script independent of Windows PowerShell's source encoding.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
& $env:KURU_PE_SCRIPT -Binary $env:KURU_PE_INPUT
"#,
    )
    .unwrap();
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let program_files = std::env::var_os("ProgramFiles(x86)")
        .expect("native MSVC acceptance requires ProgramFiles(x86)");
    let launch = |input: &str| {
        let mut child = command(root.path(), &powershell);
        child
            .env("OS", "Windows_NT")
            .env("ProgramFiles(x86)", &program_files)
            .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
            .env("KURU_CLI_FIXTURE_ENGINE", &engine)
            .env("KURU_PE_SCRIPT", &script)
            .env("KURU_PE_INPUT", input)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-OutputFormat",
                "Text",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&launcher);
        child.output().unwrap()
    };
    let extended = binary.canonicalize().unwrap();
    let extended = extended.to_str().unwrap();
    // Construct alternate spellings of this same retained fixture file only;
    // production resolution preserves the genuine extended prefix unchanged.
    let ordinary = match extended.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => extended.strip_prefix(r"\\?\").unwrap().to_owned(),
    };
    let qualified = format!(r"Microsoft.PowerShell.Core\FileSystem::{extended}");
    for input in [ordinary.as_str(), extended, qualified.as_str()] {
        let output = launch(input);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "input={input:?}: {diagnostic}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = stdout.lines().collect();
        assert_eq!(lines.len(), 2, "input={input:?}: {stdout:?}");
        let mut inventories = Vec::new();
        for (line, expected) in lines.into_iter().zip([&binary, &engine]) {
            let (path, libraries) = line
                .strip_prefix("OS-only PE imports: ")
                .unwrap()
                .rsplit_once(": ")
                .unwrap();
            assert!(!path.contains("FileSystem::"), "{line}");
            assert_eq!(
                Path::new(path).canonicalize().unwrap(),
                expected.canonicalize().unwrap()
            );
            assert!(!libraries.is_empty(), "{line}");
            assert!(
                libraries
                    .split(", ")
                    .all(|library| library.ends_with(".dll"))
            );
            inventories.push(libraries);
        }
        assert_eq!(inventories[0], inventories[1]);
    }
    let output = launch(invalid.to_str().unwrap());
    assert!(!output.status.success(), "non-PE input must fail");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("OS-only PE imports:"));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("Cannot inspect PE imports")
            || diagnostic.contains("No DLL imports were parsed"),
        "non-PE input must reach actual inspection: {diagnostic}"
    );
    let calls: Vec<Vec<String>> = fs::read_to_string(root.path().join("actions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls,
        vec![vec!["run", "//packages/kuru-memory:prefetch"]; 4]
    );
    for (path, before) in [
        (&engine, &engine_before),
        (&binary, &binary_before),
        (&invalid, &invalid_before),
    ] {
        let after = evidence(path);
        assert_eq!(regular_file_info(&before.0).unwrap(), before.1);
        assert_eq!(
            after.1,
            before.1,
            "identity/size changed: {}",
            path.display()
        );
        assert_eq!(after.2, before.2, "bytes changed: {}", path.display());
    }
}

#[test]
fn built_in_shell_reconstructs_stock_module_paths_without_losing_other_environment() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("workspace 日本語");
    let modules = root.path().join("incompatible modules");
    for directory in [&project, &modules, &root.path().join("tools")] {
        fs::create_dir(directory).unwrap();
    }
    let incompatible = modules.join("Microsoft.PowerShell.Utility");
    fs::create_dir(&incompatible).unwrap();
    // Discovery reads these explicit exports before import checks the minimum
    // engine version. No cmdlet, script module or DLL implements the command.
    fs::write(
        incompatible.join("Microsoft.PowerShell.Utility.psd1"),
        r#"@{
    ModuleVersion = '7.0.0.0'
    PowerShellVersion = '7.0'
    CmdletsToExport = @('Get-FileHash')
    FunctionsToExport = @()
    AliasesToExport = @()
}
"#,
    )
    .unwrap();
    let input = project.join("hash input 日本語.bin");
    let bytes = b"real stock PowerShell file hashing\0\xff\n";
    fs::write(&input, bytes).unwrap();
    let identity = regular_file_info(&File::open(&input).unwrap())
        .unwrap()
        .identity;
    let sentinel = "retained & literal 日本語";
    let source = r#"$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) { throw 'expected stock PowerShell 5.1' }
$hash = (Get-FileHash -LiteralPath $env:KURU_HASH_INPUT -Algorithm SHA256).Hash
[Console]::Write($hash + '|' + $env:KURU_SHELL_SENTINEL)
"#;
    let child = |binary: &Path| {
        let mut child = command(root.path(), binary);
        child
            // This incompatible manifest precedes the system-module fallback
            // that lets stock 5.1 find commands through an empty module path.
            // Mixed casing exercises Windows environment-key comparison.
            .env("pSmOdUlEpAtH", &modules)
            .env("KURU_HASH_INPUT", &input)
            .env("KURU_SHELL_SENTINEL", sentinel);
        child
    };
    let powershell = kuru_platform::windows::process::system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let control = child(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-OutputFormat",
            "Text",
            "-Command",
            source,
        ])
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&control.stderr);
    let preview =
        |bytes: &[u8]| String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned();
    assert!(
        !control.status.success(),
        "unsanitized control must fail: status={} stdout={:?} stderr={:?}",
        control.status,
        preview(&control.stdout),
        preview(&control.stderr)
    );
    assert!(
        diagnostic.contains("Get-FileHash")
            && diagnostic.contains("CouldNotAutoloadMatchingModule")
            && diagnostic.contains("Microsoft.PowerShell.Utility"),
        "control must reach the incompatible module's autoload failure: {:?}",
        preview(&control.stderr)
    );
    assert!(control.stdout.is_empty());

    let arguments = serde_json::json!({ "command": source }).to_string();
    let output = child(Path::new(env!("CARGO_BIN_EXE_kuru")))
        .arg("-C")
        .arg(&project)
        .args(["--allow-shell", "tool", "shell", "--args", &arguments])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["exit_code"], 0);
    assert_eq!(result["success"], true);
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    assert_eq!(result["stdout"], format!("{digest}|{sentinel}"));
    assert_eq!(result["stderr"], "");
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert_eq!(
        regular_file_info(&File::open(&input).unwrap())
            .unwrap()
            .identity,
        identity
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
