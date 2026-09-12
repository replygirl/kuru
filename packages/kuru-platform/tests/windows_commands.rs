#![cfg(all(windows, feature = "test-support"))]

use kuru_platform::windows::pipe::Pipe;
use kuru_platform::windows::process::{
    NativeChild, NativeSpawnSpec, StandardStream, Stdio, configured_command,
    current_process_handle, duplicate_inherited_process_handle, environment_key_eq,
    inherited_stdio, merge_environment, resolve_executable, system_directory, wait_process_handle,
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
async fn configured_stock_powershell_starts_like_direct_spawn_with_the_same_isolated_environment() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("native shell 日本語");
    std::fs::create_dir(&cwd).unwrap();
    let system = system_directory().unwrap();
    let shell = system.join("WindowsPowerShell/v1.0/powershell.exe");
    assert!(shell.is_file(), "stock PowerShell 5.1 is required");
    let marker = cwd.join("script-reached");
    let mut env: Vec<(OsString, OsString)> = vec![
        ("SystemRoot".into(), system.parent().unwrap().into()),
        ("PROCESSOR_ARCHITECTURE".into(), "AMD64".into()),
        ("PATH".into(), "".into()),
        ("USERPROFILE".into(), cwd.clone().into()),
        ("LOCALAPPDATA".into(), cwd.clone().into()),
        ("TMP".into(), cwd.clone().into()),
        ("TEMP".into(), cwd.clone().into()),
        ("KURU_PROBE".into(), marker.clone().into()),
    ];
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        env.push(("LLVM_PROFILE_FILE".into(), profile));
    }
    let args: Vec<OsString> = ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
        "$ErrorActionPreference='Stop'; if ($PSVersionTable.PSVersion.Major -ne 5 -or $PSVersionTable.PSVersion.Minor -ne 1) { throw 'requires 5.1' }; $null=[Net.ServicePointManager]::SecurityProtocol; [IO.File]::WriteAllText($env:KURU_PROBE,'reached'); [Console]::WriteLine('ready:5.1')"]
        .map(Into::into).into();
    let mut direct = NativeSpawnSpec::new(shell.clone(), cwd.clone());
    direct.args = args.clone();
    direct.environment = env.clone();
    let configured =
        configured_command(shell.as_os_str(), &args, &cwd.canonicalize().unwrap(), env).unwrap();
    assert_eq!(
        configured.executable.canonicalize().unwrap(),
        shell.canonicalize().unwrap()
    );
    assert_eq!(
        configured.cwd.canonicalize().unwrap(),
        cwd.canonicalize().unwrap()
    );
    for (label, mut spec) in [("direct", direct), ("configured", configured)] {
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        let mut child = spec.spawn().await.unwrap();
        let mut stdout = child.take_stdout().unwrap();
        let mut stderr = child.take_stderr().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        capture_native_output(
            &mut child,
            &mut stdout,
            &mut stderr,
            &mut out,
            &mut err,
            Duration::from_secs(30),
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "{label}: {error}; progress={:?}",
                std::fs::read_to_string(&marker)
            )
        });
        stdout.close(Duration::from_secs(5)).await.unwrap();
        stderr.close(Duration::from_secs(5)).await.unwrap();
        let status = child.wait(Duration::from_secs(5)).await.unwrap();
        assert!(
            status.success(),
            "{label}: {status}; stdout={}; stderr={}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        assert_eq!(
            String::from_utf8(out).unwrap().trim(),
            "ready:5.1",
            "{label}"
        );
        assert_eq!(std::fs::read(&marker).unwrap(), b"reached", "{label}");
        std::fs::remove_file(&marker).unwrap();
    }
}

const OUTPUT_LIMIT: usize = 65536;

async fn read_bounded_output(
    stream: &mut Pipe,
    bytes: &mut Vec<u8>,
    label: &str,
) -> std::io::Result<()> {
    stream
        .take((OUTPUT_LIMIT + 1).saturating_sub(bytes.len()) as u64)
        .read_to_end(bytes)
        .await?;
    let _ = label;
    Ok(())
}

fn check_output_limit(bytes: &[u8], label: &str) -> std::io::Result<()> {
    if bytes.len() > OUTPUT_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{label} exceeded {OUTPUT_LIMIT} bytes"),
        ));
    }
    Ok(())
}

async fn capture_native_output(
    child: &mut NativeChild,
    stdout: &mut Pipe,
    stderr: &mut Pipe,
    out: &mut Vec<u8>,
    err: &mut Vec<u8>,
    timeout: Duration,
) -> Result<(), String> {
    let capture = tokio::time::timeout(timeout, async {
        // Negative control: preserve the old post-join limit check ordering.
        tokio::try_join!(
            read_bounded_output(stdout, out, "stdout"),
            read_bounded_output(stderr, err, "stderr")
        )?;
        check_output_limit(out, "stdout")?;
        check_output_limit(err, "stderr")
    })
    .await;
    let failure = match capture {
        Ok(Ok(_)) => return Ok(()),
        Ok(Err(error)) => format!("capture failed: {error}"),
        Err(_) => "capture timed out".to_owned(),
    };
    // A live descendant can retain output after the root exits. Keep those
    // states distinct, and never let a diagnostic error prevent cleanup.
    let root_exit = match child.duplicate_process_handle() {
        Ok(root) => wait_process_handle(&root, Duration::ZERO).await,
        Err(error) => Err(error),
    };
    let tree_exit = child.try_wait();
    let termination = child.terminate();
    let reaped = child.wait(Duration::from_secs(5)).await;
    let closed = tokio::join!(
        stdout.close(Duration::from_secs(5)),
        stderr.close(Duration::from_secs(5))
    );
    Err(format!(
        "{failure}; root_exit={root_exit:?}; tree_exit={tree_exit:?}; stdout={}; stderr={}; termination={termination:?}; reaped={reaped:?}; closed={closed:?}",
        String::from_utf8_lossy(out),
        String::from_utf8_lossy(err)
    ))
}

#[tokio::test]
async fn overflowing_capture_rejects_each_stream_and_reaps_the_blocked_writer() {
    for stream in ["stdout", "stderr"] {
        let root = tempfile::tempdir().unwrap();
        let mut spec = NativeSpawnSpec::new(
            env!("CARGO_BIN_EXE_kuru-platform-process-fixture").into(),
            root.path().into(),
        );
        spec.args = vec!["capture-flood".into(), stream.into()];
        spec.environment = environment(root.path());
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        let mut child = spec.spawn().await.unwrap();
        let process = child.duplicate_process_handle().unwrap();
        let mut stdout = child.take_stdout().unwrap();
        let mut stderr = child.take_stderr().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let error = capture_native_output(
            &mut child,
            &mut stdout,
            &mut stderr,
            &mut out,
            &mut err,
            Duration::from_secs(10),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains(&format!("{stream} exceeded {OUTPUT_LIMIT} bytes")),
            "{error}"
        );
        let bytes = if stream == "stdout" { &out } else { &err };
        assert_eq!(bytes.len(), OUTPUT_LIMIT + 1);
        assert!(bytes.iter().all(|byte| *byte == b'x'));
        wait_process_handle(&process, Duration::ZERO).await.unwrap();
        assert!(child.try_wait().unwrap().is_some());
        assert_eq!(
            stdout.read(&mut [0]).await.unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            stderr.read(&mut [0]).await.unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
}

#[tokio::test]
async fn stalled_capture_preserves_partial_output_and_reaps_before_returning() {
    let root = tempfile::tempdir().unwrap();
    let mut spec = NativeSpawnSpec::new(
        env!("CARGO_BIN_EXE_kuru-platform-process-fixture").into(),
        root.path().into(),
    );
    spec.args = vec!["capture-stall".into()];
    spec.environment = environment(root.path());
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec.spawn().await.unwrap();
    let process = child.duplicate_process_handle().unwrap();
    let mut stdout = child.take_stdout().unwrap();
    let mut stderr = child.take_stderr().unwrap();
    let mut out = vec![0; b"fixture stdout\n".len()];
    let mut err = vec![0; b"fixture stderr\n".len()];
    // Synchronize with real output so the short capture deadline cannot be
    // satisfied by a failure to start the fixture in the first place.
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::try_join!(stdout.read_exact(&mut out), stderr.read_exact(&mut err))
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(out, b"fixture stdout\n");
    assert_eq!(err, b"fixture stderr\n");
    let error = capture_native_output(
        &mut child,
        &mut stdout,
        &mut stderr,
        &mut out,
        &mut err,
        Duration::from_millis(50),
    )
    .await
    .unwrap_err();
    assert!(error.contains("capture timed out"), "{error}");
    assert!(
        error.contains("fixture stdout") && error.contains("fixture stderr"),
        "{error}"
    );
    wait_process_handle(&process, Duration::ZERO).await.unwrap();
    assert!(child.try_wait().unwrap().is_some());
    assert_eq!(
        stdout.read(&mut [0]).await.unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        stderr.read(&mut [0]).await.unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn configured_launch_paths_do_not_normalize_a_distinct_verbatim_target() {
    let root = tempfile::tempdir().unwrap();
    let ordinary = root.path().join("different");
    std::fs::create_dir(&ordinary).unwrap();
    std::fs::write(ordinary.join("peer.exe"), b"ordinary alias").unwrap();
    let exact = root.path().canonicalize().unwrap().join("different.");
    std::fs::create_dir(&exact).unwrap();
    std::fs::write(exact.join("peer.exe"), b"verbatim target").unwrap();
    let executable = exact.join("peer.exe").canonicalize().unwrap();
    let spec = configured_command(executable.as_os_str(), &[], &exact, vec![]).unwrap();
    assert_eq!(spec.executable, executable);
    assert_eq!(spec.cwd, exact.canonicalize().unwrap());
    assert_eq!(std::fs::read(spec.executable).unwrap(), b"verbatim target");
    assert_eq!(
        std::fs::read(ordinary.join("peer.exe")).unwrap(),
        b"ordinary alias"
    );
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
