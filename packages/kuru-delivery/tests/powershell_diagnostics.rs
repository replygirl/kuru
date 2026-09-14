#![cfg(feature = "tooling")]

#[path = "support/powershell_diagnostic.rs"]
mod powershell_diagnostic;

#[test]
fn actual_wrapped_native_acl_error_is_decoded_without_progress_or_warning_text() {
    let output = br#"#< CLIXML
<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">
<Obj S="progress"><S>Preparing modules for first use.</S></Obj>
<S S="warning">unrelated warning</S>
<S S="Error">install.ps1 : Exception calling &quot;.ctor&quot;: &quot;private _x000D__x000A_</S>
<S S="Error">ACL grants another principal access&quot;_x000D__x000A_</S>
</Objs>"#;
    assert_eq!(
        powershell_diagnostic::message(output),
        "install.ps1 : Exception calling \".ctor\": \"private ACL grants another principal access\""
    );
}

#[test]
fn error_decoding_is_single_pass_and_retains_plain_or_malformed_diagnostics() {
    assert_eq!(
        powershell_diagnostic::message(
            b"#< CLIXML\n<Objs><S S=\"Error\">_x005F_x0041_ _xD83D__xDE80_ &amp;</S></Objs>"
        ),
        "_x0041_ \u{1f680} &"
    );
    for plain in [
        "Error: private ACL grants\r\n another principal access",
        "<S S=\"Error\">literal XML _x000A_</S>",
        "#< CLIXML\nmalformed diagnostic",
    ] {
        assert_eq!(
            powershell_diagnostic::message(plain.as_bytes()),
            plain.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
}

#[test]
fn valid_envelopes_without_errors_cannot_satisfy_a_rejection_message() {
    for envelope in [
        "#< CLIXML\n<Objs><S S=\"warning\">private ACL grants another principal access</S></Objs>",
        "#< CLIXML\n<Objs><Obj S=\"progress\"><S>private ACL grants another principal access</S></Obj></Objs>",
        "#< CLIXML\n<Objs><S S=\"Error\"></S></Objs>",
    ] {
        assert_eq!(powershell_diagnostic::message(envelope.as_bytes()), "");
    }
}

#[cfg(windows)]
#[tokio::test]
async fn pwsh_set_content_preserves_long_cargo_json_lines() {
    use std::{fs, time::Duration};

    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("cargo.json");
    let value = serde_json::json!({
        "reason": "compiler-artifact",
        "target": {"name": "fixture", "src_path": "x".repeat(4096)},
        "filenames": ["y".repeat(4096)]
    });
    let line = serde_json::to_string(&value).unwrap().replace('\'', "''");
    let script = format!(
        "@('{line}') | Set-Content -LiteralPath '{}' -Encoding utf8NoBOM; if (-not $?) {{ exit 1 }}",
        destination.display()
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = kuru_delivery::command::rooted(&root, "pwsh");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]);
    let command_output =
        kuru_delivery::command::bounded_output(&mut command, Duration::from_secs(10), 32 * 1024)
            .await
            .unwrap();
    assert!(
        command_output.status.success(),
        "pwsh failed: stdout={} stderr={}",
        String::from_utf8_lossy(&command_output.stdout),
        String::from_utf8_lossy(&command_output.stderr)
    );
    let bytes = fs::read(destination).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, value);
}

#[test]
fn windows_coverage_tasks_launch_pwsh_without_cmd_metacharacters() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = std::fs::read_to_string(root.join("packages/kuru-delivery/mise.toml")).unwrap();
    let script =
        std::fs::read_to_string(root.join("packages/kuru-delivery/support/windows-coverage.ps1"))
            .unwrap();

    let prefix = "pwsh.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File packages/kuru-delivery/support/windows-coverage.ps1 -Mode ";
    assert!(task.contains(&format!("run_windows = \"{prefix}Shard\"")));
    assert!(task.contains(&format!("run_windows = \"{prefix}Collect\"")));
    assert!(
        !task.contains("run_windows = \"& packages/kuru-delivery/support/windows-coverage.ps1")
    );
    for binding in [
        "$env:KURU_COVERAGE_TARGET",
        "$env:KURU_COVERAGE_SOURCE",
        "$env:KURU_COVERAGE_ATTEMPT",
        "$env:KURU_COVERAGE_SHARD",
        "$env:KURU_COVERAGE_PACKAGES",
        "$env:KURU_COVERAGE_OUTPUT",
        "$env:KURU_COVERAGE_INPUTS",
        "$env:KURU_COVERAGE_REPORT",
    ] {
        assert!(script.contains(binding), "missing script binding {binding}");
    }
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_mise_launches_published_windows_task_wrapper_before_cargo() {
    use std::time::Duration;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut child = kuru_delivery::command::rooted(&root, "mise");
    child.args(["run", "//packages/kuru-delivery:verify:published-windows"]);
    child.env("MISE_TASK_RUN_AUTO_INSTALL", "false");
    child.env("MISE_WINDOWS_DEFAULT_INLINE_SHELL_ARGS", "cmd.exe /d /s /c");
    for variable in [
        "RELEASE_VERSION",
        "RELEASE_SHA",
        "RELEASE_RUN_URL",
        "KURU_PUBLISHED_WINDOWS_RECEIPT",
    ] {
        child.env_remove(variable);
    }
    let output =
        kuru_delivery::command::bounded_output(&mut child, Duration::from_secs(15), 16 * 1024)
            .await
            .unwrap();
    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("Published Windows verification requires"),
        "task did not reach wrapper validation: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("is not recognized as an internal or external command"),
        "task executed its PowerShell body through cmd: {diagnostic}"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_launches_the_exact_shard_task_and_reaches_script_validation() {
    use std::time::Duration;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = std::fs::read_to_string(root.join("packages/kuru-delivery/mise.toml")).unwrap();
    let shard = task
        .split("[tasks.\"coverage:windows:shard\"]")
        .nth(1)
        .unwrap()
        .split("[tasks.\"coverage:windows:shard\".env]")
        .next()
        .unwrap();
    let command = shard
        .lines()
        .find_map(|line| {
            line.strip_prefix("run_windows = \"")
                .and_then(|line| line.strip_suffix('"'))
        })
        .unwrap();
    let mut child = kuru_delivery::command::rooted(&root, "cmd.exe");
    child.args(["/d", "/s", "/c", command]);
    for variable in [
        "KURU_COVERAGE_TARGET",
        "KURU_COVERAGE_SOURCE",
        "KURU_COVERAGE_ATTEMPT",
        "KURU_COVERAGE_SHARD",
        "KURU_COVERAGE_PACKAGES",
        "KURU_COVERAGE_OUTPUT",
        "KURU_COVERAGE_INPUTS",
        "KURU_COVERAGE_REPORT",
    ] {
        child.env_remove(variable);
    }
    let output =
        kuru_delivery::command::bounded_output(&mut child, Duration::from_secs(10), 16 * 1024)
            .await
            .unwrap();
    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("Coverage target, expected source and run attempt are required"),
        "task did not reach the script validation: {diagnostic}"
    );
    assert!(!diagnostic.contains("was unexpected at this time"));
}
