#![cfg(windows)]
#![forbid(unsafe_code)]

use kuru_connectors::ToolHost;
use kuru_core::{Config, McpConfig};
use kuru_platform::windows::process::{
    NativeSpawnSpec, Stdio, configured_command, environment_key_eq, merge_environment,
    resolve_executable,
};
use serde_json::{Value, json};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncReadExt;

const PEER: &str = env!("CARGO_BIN_EXE_kuru-connectors-windows-peer");
// Ensures Cargo builds the non-test-harness executable used by library tests.
const STDIO_PEER: &str = env!("CARGO_BIN_EXE_kuru-connectors-stdio-fixture");

fn environment(root: &Path) -> Vec<(OsString, OsString)> {
    merge_environment(
        std::env::vars_os(),
        [
            ("pAtH".into(), root.as_os_str().into()),
            ("PATHEXT".into(), ".EXE;.COM;.CMD;.BAT".into()),
            ("KURU_NATIVE_TEST_MARKER".into(), "child-only ✓".into()),
        ],
    )
    .unwrap()
}

async fn capture(mut spec: NativeSpawnSpec) -> Value {
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec.spawn().await.unwrap();
    let mut out = child.take_stdout().unwrap();
    let mut err = child.take_stderr().unwrap();
    let operation = async {
        async fn read(pipe: &mut kuru_platform::windows::pipe::Pipe) -> Vec<u8> {
            let mut bytes = Vec::new();
            pipe.take(1 << 20).read_to_end(&mut bytes).await.unwrap();
            bytes
        }
        let (stdout, stderr) = tokio::join!(read(&mut out), read(&mut err));
        let status = child.wait(Duration::from_secs(10)).await.unwrap();
        assert!(
            status.success(),
            "{status}: {}",
            String::from_utf8_lossy(&stderr)
        );
        serde_json::from_slice(&stdout).unwrap()
    };
    let result = tokio::time::timeout(Duration::from_secs(15), operation).await;
    if result.is_err() {
        child.terminate().unwrap();
    }
    out.close(Duration::from_secs(5)).await.unwrap();
    err.close(Duration::from_secs(5)).await.unwrap();
    child.wait(Duration::from_secs(5)).await.unwrap();
    result.unwrap()
}

fn raw(values: &[OsString]) -> Value {
    json!(
        values
            .iter()
            .map(|arg| arg.encode_wide().collect::<Vec<_>>())
            .collect::<Vec<_>>()
    )
}

fn fixture(root: &Path) -> PathBuf {
    let executable = root.join("native peer.exe");
    std::fs::copy(PEER, &executable).unwrap();
    executable
}

#[tokio::test]
async fn native_exe_com_resolution_preserves_utf16_and_child_environment() {
    assert!(Path::new(STDIO_PEER).is_file());
    let root = tempfile::tempdir().unwrap();
    let executable = fixture(root.path());
    std::fs::copy(PEER, root.path().join("native.com")).unwrap();
    let values = vec![
        "".into(),
        "two words".into(),
        "a\"b\\".into(),
        "&|<>^()%!".into(),
        "日本語 🦀".into(),
        OsString::from_wide(&[0xd800, b'x' as u16]),
    ];
    let args: Vec<_> = [OsString::from("argv")]
        .into_iter()
        .chain(values.clone())
        .collect();
    for program in [executable.as_os_str(), OsStr::new("native.com")] {
        let result = capture(
            configured_command(program, &args, root.path(), environment(root.path())).unwrap(),
        )
        .await;
        assert_eq!(result["args"], raw(&values));
        assert_eq!(result["marker"], "child-only ✓");
    }
    let mut env = environment(root.path());
    env.iter_mut()
        .find(|(key, _)| environment_key_eq(key, OsStr::new("PATH")))
        .unwrap()
        .1 = std::env::join_paths([
        Path::new("relative"),
        &root.path().join("absent"),
        root.path(),
    ])
    .unwrap();
    assert_eq!(
        resolve_executable(OsStr::new("native peer"), root.path(), &env).unwrap(),
        executable.canonicalize().unwrap()
    );
    assert!(resolve_executable(OsStr::new("C:ambiguous.exe"), root.path(), &env).is_err());
    assert!(
        merge_environment(
            [("PATH".into(), "one".into()), ("Path".into(), "two".into())],
            []
        )
        .is_err()
    );
}

#[tokio::test]
async fn batch_shims_and_explicit_cmd_keep_literal_arguments_and_source_distinct() {
    let root = tempfile::tempdir().unwrap();
    let executable = fixture(root.path());
    for extension in ["cmd", "bat"] {
        std::fs::write(
            root.path().join(format!("npx.{extension}")),
            format!("@\"{}\" argv %*\r\n", executable.display()),
        )
        .unwrap();
    }
    let values: Vec<OsString> = [
        "",
        "two words",
        "double\"quote",
        "a&b|c<d>e^f(g)",
        "%KURU_NATIVE_TEST_MARKER%",
        "bang!",
        "trailing\\",
        "日本語 🦀",
    ]
    .map(Into::into)
    .into();
    for program in ["npx.cmd", "npx.bat", "npx"] {
        let result = capture(
            configured_command(
                OsStr::new(program),
                &values,
                root.path(),
                environment(root.path()),
            )
            .unwrap(),
        )
        .await;
        assert_eq!(result["args"], raw(&values), "{program}");
    }
    let args: Vec<_> = [OsString::from("/d"), "/s".into(), "/c".into(), "npx".into()]
        .into_iter()
        .chain(values.clone())
        .collect();
    assert_eq!(
        capture(
            configured_command(
                OsStr::new("cmd"),
                &args,
                root.path(),
                environment(root.path())
            )
            .unwrap()
        )
        .await["args"],
        raw(&values)
    );
    let source = format!("\"\"{}\" argv authored\"", executable.display());
    let spec = configured_command(
        OsStr::new("cmd.exe"),
        &["/d".into(), "/s".into(), "/c".into(), source.into()],
        root.path(),
        environment(root.path()),
    )
    .unwrap();
    assert_eq!(capture(spec).await["args"], raw(&["authored".into()]));
    let spec = configured_command(
        OsStr::new("npx"),
        &["line\nbreak".into()],
        root.path(),
        environment(root.path()),
    )
    .unwrap();
    assert!(spec.spawn().await.is_err());
    let spec = configured_command(
        OsStr::new("npx"),
        &["x".repeat(8192).into()],
        root.path(),
        environment(root.path()),
    )
    .unwrap();
    assert!(spec.spawn().await.is_err());
}

#[tokio::test]
async fn configured_mcp_cmd_npx_and_owned_descendant_shutdown_are_real() {
    let root = tempfile::tempdir().unwrap();
    let executable = fixture(root.path());
    std::fs::write(
        root.path().join("npx.cmd"),
        format!("@\"{}\" mcp-tree\r\n", executable.display()),
    )
    .unwrap();
    let config = Config {
        mcp: BTreeMap::from([(
            "native".into(),
            McpConfig {
                command: Some("cmd".into()),
                args: vec!["/c".into(), "npx".into(), "-y".into()],
                env: BTreeMap::from([("Path".into(), root.path().to_str().unwrap().into())]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let host = ToolHost::new(root.path(), &config).unwrap();
    let specs = host.specs().await.unwrap();
    let name = &specs
        .iter()
        .find(|spec| spec.name.starts_with("mcp_"))
        .unwrap()
        .name;
    let result: Value = serde_json::from_str(
        &host
            .execute(name, json!({"value":"native MCP"}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("native MCP")
    );
    tokio::time::timeout(Duration::from_secs(10), host.shutdown())
        .await
        .unwrap()
        .unwrap();
    // Successful close includes active-zero Job accounting, despite the real
    // parked descendant retaining stdout for 120 seconds.
}

#[tokio::test]
async fn file_alias_hardlink_and_reparse_guards_preserve_external_data() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".env"), "secret").unwrap();
    std::fs::write(root.path().join("ordinary"), "outside").unwrap();
    std::fs::hard_link(root.path().join("ordinary"), root.path().join("alias")).unwrap();
    std::fs::create_dir(root.path().join(".kuru")).unwrap();
    std::fs::write(root.path().join(".kuru/memory"), "private memory").unwrap();
    let mut junction = configured_command(
        OsStr::new("cmd"),
        &["/d".into(), "/c".into(), "mklink /J junction .kuru".into()],
        root.path(),
        environment(root.path()),
    )
    .unwrap()
    .spawn()
    .await
    .unwrap();
    assert!(
        junction
            .wait(Duration::from_secs(10))
            .await
            .unwrap()
            .success()
    );
    let host = ToolHost::new(
        root.path(),
        &Config {
            allow_write: true,
            ..Default::default()
        },
    )
    .unwrap();
    for path in [
        ".env.",
        ".env ",
        "ordinary:stream",
        "NUL",
        "CON.txt",
        "COM¹",
        "alias",
        "junction/memory",
    ] {
        assert!(
            host.execute("file_read", json!({"path":path}))
                .await
                .is_err(),
            "{path}"
        );
        assert!(
            host.execute("file_write", json!({"path":path,"content":"changed"}))
                .await
                .is_err(),
            "{path}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("ordinary")).unwrap(),
        "outside"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join(".env")).unwrap(),
        "secret"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join(".kuru/memory")).unwrap(),
        "private memory"
    );
}

#[tokio::test]
async fn stock_powershell_unicode_and_terminating_errors_are_observed() {
    let root = tempfile::tempdir().unwrap();
    let working = root.path().join("shell cwd café 東京");
    std::fs::create_dir(&working).unwrap();
    let host = ToolHost::new(
        &working,
        &Config {
            allow_shell: true,
            ..Default::default()
        },
    )
    .unwrap();
    let result: Value = serde_json::from_str(&host.execute("shell", json!({"command":"Write-Progress -Activity 'native progress' -Status 'working' -PercentComplete 50; [IO.File]::WriteAllText([IO.Path]::Combine((Get-Location).Path,'shell-created.txt'),'日本語 🦀'); [Console]::Out.Write('日本語 🦀'); [Console]::Error.Write('échec'); exit 0"})).await.unwrap()).unwrap();
    assert_eq!(result["stdout"], "日本語 🦀");
    assert_eq!(result["stderr"], "échec");
    assert_eq!(result["success"], true);
    assert_eq!(
        std::fs::read_to_string(working.join("shell-created.txt")).unwrap(),
        "日本語 🦀"
    );
    let literal: Value = serde_json::from_str(&host.execute("shell", json!({"command":"[Console]::Error.Write('#< CLIXML <Objs>literal diagnostic</Objs>')"})).await.unwrap()).unwrap();
    assert_eq!(
        literal["stderr"],
        "#< CLIXML <Objs>literal diagnostic</Objs>"
    );
    let diagnostics: Value = serde_json::from_str(&host.execute("shell", json!({"command":"Write-Warning 'retained warning'; Write-Error 'retained error'; exit 9"})).await.unwrap()).unwrap();
    assert_eq!(diagnostics["exit_code"], 9);
    assert_eq!(diagnostics["success"], false);
    let streams = format!("{}{}", diagnostics["stdout"], diagnostics["stderr"]);
    assert!(streams.contains("retained warning"), "{diagnostics}");
    assert!(
        diagnostics["stderr"]
            .as_str()
            .unwrap()
            .contains("retained error"),
        "{diagnostics}"
    );
    let failed: Value = serde_json::from_str(
        &host
            .execute("shell", json!({"command":"throw 'native failure'"}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(failed["success"], false);
    assert!(
        failed["stderr"]
            .as_str()
            .unwrap()
            .contains("native failure")
    );
}
