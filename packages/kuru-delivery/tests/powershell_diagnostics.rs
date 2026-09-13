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
#[test]
fn pwsh_set_content_preserves_long_cargo_json_lines() {
    use std::{fs, process::Command};

    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("cargo.json");
    let value = serde_json::json!({
        "reason": "compiler-artifact",
        "target": {"name": "fixture", "src_path": "x".repeat(4096)},
        "filenames": ["y".repeat(4096)]
    });
    let line = serde_json::to_string(&value).unwrap().replace('\'', "''");
    let script = format!(
        "@('{line}') | Set-Content -LiteralPath '{}' -Encoding utf8NoBOM; if (-not $?) {{ exit 1 }}",
        output.display()
    );
    let status = Command::new("pwsh")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = fs::read(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, value);
}
