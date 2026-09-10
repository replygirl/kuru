#![cfg(all(windows, feature = "test-support"))]

use kuru_platform::windows::process::{
    StandardStream, Stdio, configured_command, current_process_handle,
    duplicate_inherited_process_handle, environment_key_eq, inherited_stdio, merge_environment,
    resolve_executable, system_directory, wait_process_handle,
};
use std::{
    ffi::{OsStr, OsString},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncReadExt;

fn environment(root: &Path) -> Vec<(OsString, OsString)> {
    let mut env = vec![
        ("PATH".into(), root.into()),
        ("PATHEXT".into(), ".CMD;.BAT;.EXE;.COM;.cmd;.vbs".into()),
    ];
    for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(key) {
            env.push((key.into(), value));
        }
    }
    env
}

#[tokio::test]
async fn batch_transport_runs_the_native_capture_fixture_with_literal_data() {
    let root = tempfile::tempdir().unwrap();
    let peer = root.path().join("capture peer.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_kuru-platform-process-fixture"), &peer).unwrap();
    std::fs::write(
        root.path().join("shim.cmd"),
        format!("@\"{}\" capture %*\r\n", peer.display()),
    )
    .unwrap();
    let args: Vec<OsString> = [
        "",
        "two words",
        "quotes\"here",
        "%PATH%",
        "a&b|c<d>e^f(g)!",
        "日本語",
        "last\\",
    ]
    .map(Into::into)
    .into();
    for (program, args) in [
        (OsStr::new("shim"), args.clone()),
        (
            OsStr::new("cmd.exe"),
            [
                OsString::from("/d"),
                "/s".into(),
                "/c".into(),
                "shim".into(),
            ]
            .into_iter()
            .chain(args.clone())
            .collect(),
        ),
    ] {
        let mut spec =
            configured_command(program, &args, root.path(), environment(root.path())).unwrap();
        spec.stdout = Stdio::Pipe;
        let mut child = spec.spawn().await.unwrap();
        let mut output = child.take_stdout().unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            (&mut output).take(1 << 20).read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        let status = child.wait(Duration::from_secs(5)).await.unwrap();
        output.close(Duration::from_secs(5)).await.unwrap();
        assert!(
            status.success(),
            "{status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["args"],
            serde_json::json!([
                "",
                "two words",
                "quotes\"here",
                "%PATH%",
                "a&b|c<d>e^f(g)!",
                "日本語",
                "last\\"
            ])
        );
    }
}

#[tokio::test]
async fn command_resolution_and_representability_fail_before_execution() {
    let root = tempfile::tempdir().unwrap();
    for name in ["tool.exe", "tool.com", "shim.cmd", "bad%name.cmd"] {
        std::fs::write(root.path().join(name), b"not executable").unwrap();
    }
    let env = environment(root.path());
    assert_eq!(
        resolve_executable(OsStr::new("tool"), root.path(), &env).unwrap(),
        root.path().join("tool.exe").canonicalize().unwrap()
    );
    assert_eq!(
        resolve_executable(OsStr::new(".\\tool.com"), root.path(), &env).unwrap(),
        root.path().join("tool.com").canonicalize().unwrap()
    );
    for name in ["", "C:tool", "\\tool", "absent", "bad\0name"] {
        assert!(
            resolve_executable(OsStr::new(name), root.path(), &env).is_err(),
            "{name}"
        );
    }
    assert!(resolve_executable(OsStr::new("tool"), Path::new("relative"), &env).is_err());
    assert!(resolve_executable(OsStr::new("tool"), root.path(), &[]).is_err());
    for args in [
        vec![],
        vec!["/c".into()],
        vec!["/k".into(), "echo".into()],
        vec!["/bogus".into(), "/c".into(), "source".into()],
        vec!["/v:on".into(), "/c".into(), "tool".into(), "literal".into()],
    ] {
        assert!(configured_command(OsStr::new("cmd"), &args, root.path(), env.clone()).is_err());
    }
    for arg in [
        "line\nbreak".into(),
        "carriage\rreturn".into(),
        "nul\0".into(),
        OsString::from("x".repeat(8200)),
    ] {
        let spec =
            configured_command(OsStr::new("shim.cmd"), &[arg], root.path(), env.clone()).unwrap();
        assert!(spec.spawn().await.is_err());
    }
    let spec =
        configured_command(OsStr::new("bad%name.cmd"), &[], root.path(), env.clone()).unwrap();
    assert!(spec.spawn().await.is_err());
    let mut oversized = env.clone();
    oversized.push(("BIG".into(), "x".repeat(8192).into()));
    let spec = configured_command(
        OsStr::new("cmd"),
        &[
            "/e:off".into(),
            "/v:on".into(),
            "/c".into(),
            "echo source".into(),
        ],
        root.path(),
        oversized,
    )
    .unwrap();
    assert!(spec.spawn().await.is_err());
    let mut source = configured_command(
        system_directory().unwrap().join("cmd.exe").as_os_str(),
        &["/c".into(), "exit 0".into()],
        root.path(),
        env,
    )
    .unwrap();
    source.args.push("ambiguous".into());
    assert!(source.spawn().await.is_err());
}

#[tokio::test]
async fn process_capabilities_and_case_equivalent_environment_are_checked() {
    assert!(environment_key_eq(OsStr::new("Path"), OsStr::new("PATH")));
    assert!(!environment_key_eq(
        OsStr::new("PATH"),
        OsStr::new("PATHEXT")
    ));
    let env = merge_environment(
        [
            ("Path".into(), "old".into()),
            ("OTHER".into(), "keep".into()),
        ],
        [("PATH".into(), "new".into())],
    )
    .unwrap();
    assert_eq!(env.len(), 2);
    assert!(env.contains(&("PATH".into(), "new".into())));
    assert!(
        merge_environment(
            [],
            [("PATH".into(), "one".into()), ("Path".into(), "two".into())]
        )
        .is_err()
    );
    assert!(merge_environment([("bad=key".into(), "no".into())], []).is_err());
    assert!(merge_environment([("=C:".into(), "C:\\fixture".into())], []).is_ok());
    assert!(
        merge_environment([(OsString::from_wide(&[b'A' as u16, 0]), "no".into())], []).is_err()
    );
    let handle = current_process_handle().unwrap();
    let duplicate =
        duplicate_inherited_process_handle(handle.as_raw_handle() as usize, std::process::id())
            .unwrap();
    assert!(
        wait_process_handle(&duplicate, Duration::from_millis(10))
            .await
            .is_err()
    );
    for (value, pid) in [
        (0, std::process::id()),
        (usize::MAX, std::process::id()),
        (handle.as_raw_handle() as usize, 0),
        (handle.as_raw_handle() as usize, std::process::id() + 1),
    ] {
        assert!(duplicate_inherited_process_handle(value, pid).is_err());
    }
    for stream in [
        StandardStream::Input,
        StandardStream::Output,
        StandardStream::Error,
    ] {
        drop(inherited_stdio(stream).unwrap());
    }
    let root = tempfile::tempdir().unwrap();
    let mut spec = configured_command(
        PathBuf::from(env!("CARGO_BIN_EXE_kuru-platform-process-fixture")).as_os_str(),
        &["capture".into()],
        root.path(),
        environment(root.path()),
    )
    .unwrap();
    spec.stdout = Stdio::Null;
    let mut child = spec.spawn().await.unwrap();
    let handle = child.duplicate_process_handle().unwrap();
    child.wait(Duration::from_secs(10)).await.unwrap();
    wait_process_handle(&handle, Duration::from_secs(5))
        .await
        .unwrap();
}

#[tokio::test]
async fn owned_private_console_restores_real_modes_and_native_focus_records() {
    use kuru_platform::windows::process::{Console, NativeSpawnSpec};
    let root = tempfile::tempdir().unwrap();
    let mut spec = NativeSpawnSpec::new(
        PathBuf::from(env!("CARGO_BIN_EXE_kuru-platform-process-fixture")),
        root.path().into(),
    );
    spec.args.push("console-check".into());
    spec.environment = environment(root.path());
    spec.console = Console::PrivateHidden;
    let mut child = spec.spawn().await.unwrap();
    assert!(child.wait(Duration::from_secs(10)).await.unwrap().success());
}
