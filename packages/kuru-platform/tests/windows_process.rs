#![cfg(all(windows, feature = "test-support"))]

use kuru_platform::windows::{
    pipe::{self, Pipe, PrivateListener, PrivateServiceListener},
    process::{Console, Lifetime, NativeChild, NativeSpawnSpec, Stdio, sample_process},
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, TryLockError},
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const LIMIT: Duration = Duration::from_secs(8);
const SHORT: Duration = Duration::from_millis(80);

#[tokio::test]
async fn private_service_pipe_accepts_multiple_clients_after_cancelled_wait() {
    let mut listener = PrivateServiceListener::bind().unwrap();
    let address = listener.address().to_owned();
    assert!(PrivateServiceListener::bind_at(&address).is_err());

    assert_eq!(
        listener.accept(SHORT).await.err().unwrap().kind(),
        io::ErrorKind::TimedOut
    );

    let mut first_client = pipe::connect(&address, LIMIT).await.unwrap();
    let mut first_server = listener.accept(LIMIT).await.unwrap();
    tokio::time::timeout(LIMIT, first_client.write_all(b"first"))
        .await
        .expect("first service client write deadline")
        .unwrap();
    tokio::time::timeout(LIMIT, first_client.flush())
        .await
        .expect("first service client flush deadline")
        .unwrap();
    let mut first = [0; 5];
    tokio::time::timeout(LIMIT, first_server.read_exact(&mut first))
        .await
        .expect("first service client read deadline")
        .unwrap();
    assert_eq!(&first, b"first");

    let mut second_client = pipe::connect(&address, LIMIT).await.unwrap();
    let mut second_server = listener.accept(LIMIT).await.unwrap();
    tokio::time::timeout(LIMIT, second_client.write_all(b"second"))
        .await
        .expect("second service client write deadline")
        .unwrap();
    tokio::time::timeout(LIMIT, second_client.flush())
        .await
        .expect("second service client flush deadline")
        .unwrap();
    let mut second = [0; 6];
    tokio::time::timeout(LIMIT, second_server.read_exact(&mut second))
        .await
        .expect("second service client read deadline")
        .unwrap();
    assert_eq!(&second, b"second");

    first_client.close(LIMIT).await.unwrap();
    first_server.close(LIMIT).await.unwrap();
    second_client.close(LIMIT).await.unwrap();
    second_server.close(LIMIT).await.unwrap();
}

fn spec(root: &Path, args: &[&OsStr]) -> NativeSpawnSpec {
    let mut spec = NativeSpawnSpec::new(
        PathBuf::from(env!("CARGO_BIN_EXE_kuru-platform-process-fixture")),
        root.to_owned(),
    );
    spec.args = args.iter().map(|arg| (*arg).to_owned()).collect();
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        spec.environment.push(("SystemRoot".into(), system_root));
    }
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
    }
    spec
}

async fn ready(child: &mut NativeChild) -> BufReader<Pipe> {
    let mut reader = BufReader::new(child.take_stdout().expect("piped fixture stdout"));
    let mut line = String::new();
    tokio::time::timeout(LIMIT, reader.read_line(&mut line))
        .await
        .expect("fixture readiness deadline")
        .expect("fixture readiness IO");
    assert!(
        !line.is_empty(),
        "fixture {} exited before readiness",
        child.id()
    );
    reader
}

async fn idle(root: &Path) -> NativeChild {
    let mut spawn = spec(root, &[OsStr::new("idle")]);
    spawn.stdout = Stdio::Pipe;
    let mut child = spawn.spawn().await.unwrap();
    ready(&mut child).await;
    child
}

async fn unlocked(path: &Path) {
    let deadline = tokio::time::Instant::now() + LIMIT;
    loop {
        let file = File::options().read(true).write(true).open(path).unwrap();
        match file.try_lock() {
            Ok(()) => return,
            Err(TryLockError::WouldBlock) => (),
            Err(error) => panic!("unexpected fixture lock failure: {error}"),
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "owned fixture retained {} after cleanup",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn independent_service_breaks_away_from_permitting_job_after_starter_exits() {
    let root = tempfile::tempdir().unwrap();
    let lock_path = root.path().join("independent.lock");
    let release = root.path().join("release");
    let mut starter = spec(
        root.path(),
        &[
            OsStr::new("independent-starter"),
            lock_path.as_os_str(),
            release.as_os_str(),
        ],
    );
    starter.lifetime = Lifetime::FixtureBreakawayJob;
    starter.stdout = Stdio::Pipe;
    let mut starter = starter.spawn().await.unwrap();
    let mut reader = BufReader::new(starter.take_stdout().unwrap());
    let mut line = String::new();
    tokio::time::timeout(LIMIT, reader.read_line(&mut line))
        .await
        .expect("independent starter readiness deadline")
        .unwrap();
    assert_eq!(line, "starter-exited\n");
    assert!(starter.wait(LIMIT).await.unwrap().success());
    drop(starter); // closes the containing kill-on-close Job
    let held = File::options()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(matches!(held.try_lock(), Err(TryLockError::WouldBlock)));
    drop(held);
    fs::write(&release, b"release").unwrap();
    unlocked(&lock_path).await;
}

#[tokio::test]
async fn independent_service_rejects_a_job_that_forbids_breakaway() {
    let root = tempfile::tempdir().unwrap();
    let lock_path = root.path().join("forbidden.lock");
    let release = root.path().join("release");
    let mut starter = spec(
        root.path(),
        &[
            OsStr::new("independent-starter"),
            lock_path.as_os_str(),
            release.as_os_str(),
        ],
    );
    starter.lifetime = Lifetime::OwnedJob;
    let mut starter = starter.spawn().await.unwrap();
    assert!(
        !starter.wait(LIMIT).await.unwrap().success(),
        "a denied breakaway must fail before the independent service starts"
    );
    drop(starter);
    assert!(
        !lock_path.exists(),
        "denied breakaway unexpectedly launched an independent leaf"
    );
}

#[tokio::test]
async fn native_arguments_environment_stdio_and_working_directory_round_trip() {
    let root = tempfile::Builder::new()
        .prefix("kuru native λ ")
        .tempdir()
        .unwrap();
    let args = [
        "",
        "two words",
        "quotes\"inside",
        "trailing \\",
        "\\\\\"",
        "λ雪🦀",
        "$HOME; & | > < %PATH% !",
        "a\tb",
    ];
    let mut spawn = spec(root.path(), &[OsStr::new("capture")]);
    spawn.args.extend(args.iter().map(OsString::from));
    let raw_argument = OsString::from_wide(&[0xD800, b' ' as u16, 0xDC00]);
    spawn.args.push(raw_argument.clone());
    spawn
        .environment
        .push(("KURU_VALUE".into(), "exact λ & value".into()));
    spawn.environment.push(("kuru_second".into(), "".into()));
    spawn
        .environment
        .push(("KURU_WIDE".into(), raw_argument.clone()));
    spawn.stdin = Stdio::Pipe;
    spawn.stdout = Stdio::Pipe;
    spawn.stderr = Stdio::Pipe;
    let mut child = spawn.spawn().await.unwrap();
    let mut input = child.take_stdin().unwrap();
    input.write_all(b"exact input\n").await.unwrap();
    input.flush().await.unwrap();
    drop(input);
    let mut output = child.take_stdout().unwrap();
    let mut error = child.take_stderr().unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    tokio::time::timeout(LIMIT, async {
        tokio::try_join!(
            output.read_to_end(&mut stdout),
            error.read_to_end(&mut stderr)
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert!(child.wait(LIMIT).await.unwrap().success());
    assert!(child.try_wait().unwrap().unwrap().success());
    child.terminate().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(
        report["args"].as_array().unwrap()[..args.len()],
        serde_json::json!(args).as_array().unwrap()[..]
    );
    assert_eq!(
        report["args_utf16"].as_array().unwrap().last().unwrap(),
        &serde_json::json!(raw_argument.encode_wide().collect::<Vec<_>>())
    );
    assert_eq!(
        report["wide_env"]["KURU_WIDE"],
        serde_json::json!(raw_argument.encode_wide().collect::<Vec<_>>())
    );
    assert_eq!(report["stdin"], "exact input\n");
    assert_eq!(report["cwd"], serde_json::json!(root.path()));
    assert_eq!(report["env"]["KURU_VALUE"], "exact λ & value");
    assert_eq!(report["env"]["kuru_second"], "");
    assert!(
        report["env"].get("PATH").is_none(),
        "ambient environment must not leak"
    );
    assert_eq!(stderr, b"fixture stderr\n");
}

#[tokio::test]
async fn invalid_creation_intent_fails_before_running_a_fixture() {
    let root = tempfile::tempdir().unwrap();
    for keys in [["Path", "PATH"], ["é", "É"]] {
        let mut spawn = spec(root.path(), &[OsStr::new("idle")]);
        spawn
            .environment
            .extend(keys.map(|key| (key.into(), "do-not-log-this".into())));
        let error = spawn.spawn().await.err().expect("duplicate env must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!error.to_string().contains("do-not-log-this"));
    }
    for (key, value) in [("", "value"), ("A=B", "value"), ("A", "x\0y")] {
        let mut spawn = spec(root.path(), &[OsStr::new("idle")]);
        spawn.environment.push((key.into(), value.into()));
        assert_eq!(
            spawn.spawn().await.err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    for path in [
        PathBuf::from("relative.exe"),
        root.path().join("hidden.cmd"),
        root.path().join("hidden.BAT"),
    ] {
        let mut spawn = spec(root.path(), &[]);
        spawn.executable = path;
        assert_eq!(
            spawn.spawn().await.err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let mut spawn = spec(root.path(), &[]);
    spawn.cwd = "relative".into();
    assert_eq!(
        spawn.spawn().await.err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
    for arg in ["bad\0arg".to_owned(), "x".repeat(33000)] {
        let mut spawn = spec(root.path(), &[]);
        spawn.args.push(arg.into());
        assert_eq!(
            spawn.spawn().await.err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let mut missing = spec(root.path(), &[]);
    missing.executable = root.path().join("missing.exe");
    missing.stdin = Stdio::Pipe;
    missing.stdout = Stdio::Pipe;
    missing.stderr = Stdio::Pipe;
    assert_eq!(
        missing.spawn().await.err().unwrap().kind(),
        io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn timeout_retains_tree_ownership_and_termination_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    let mut child = idle(root.path()).await;
    assert_eq!(
        child.wait(SHORT).await.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(child.try_wait().unwrap().is_none());
    assert_eq!(
        child.interrupt().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    child.terminate().unwrap();
    assert!(!child.wait(LIMIT).await.unwrap().success());
    child.terminate().unwrap();
}

#[tokio::test]
async fn root_exit_does_not_hide_a_descendant_or_its_open_output() {
    let root = tempfile::tempdir().unwrap();
    let root_lock = root.path().join("root.lock");
    let leaf_lock = root.path().join("leaf.lock");
    let release = root.path().join("release");
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("tree"),
            root_lock.as_os_str(),
            leaf_lock.as_os_str(),
            release.as_os_str(),
        ],
    );
    spawn.stdout = Stdio::Pipe;
    let mut child = spawn.spawn().await.unwrap();
    let mut output = BufReader::new(child.take_stdout().unwrap());
    let mut lines = String::new();
    for _ in 0..2 {
        tokio::time::timeout(LIMIT, output.read_line(&mut lines))
            .await
            .unwrap()
            .unwrap();
    }
    assert!(lines.contains("leaf-ready") && lines.contains("root-ready"));
    unlocked(&root_lock).await; // actual root-owned lock released, not a sleep
    assert!(child.try_wait().unwrap().is_none());
    assert_eq!(
        child.wait(SHORT).await.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    let mut remainder = String::new();
    assert!(
        tokio::time::timeout(SHORT, output.read_to_string(&mut remainder))
            .await
            .is_err()
    );
    fs::write(release, b"go").unwrap();
    assert!(child.wait(LIMIT).await.unwrap().success());
    tokio::time::timeout(LIMIT, output.read_to_string(&mut remainder))
        .await
        .unwrap()
        .unwrap();
    unlocked(&leaf_lock).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_handle_allowlists_keep_stdin_and_output_independent() {
    let root = tempfile::tempdir().unwrap();
    let make = || {
        let mut child = spec(root.path(), &[OsStr::new("capture")]);
        child.stdin = Stdio::Pipe;
        child.stdout = Stdio::Pipe;
        child
    };
    let (a, b) = tokio::join!(tokio::spawn(make().spawn()), tokio::spawn(make().spawn()));
    let (mut a, mut b) = (a.unwrap().unwrap(), b.unwrap().unwrap());
    drop(a.take_stdin());
    let mut output = Vec::new();
    tokio::time::timeout(LIMIT, a.take_stdout().unwrap().read_to_end(&mut output))
        .await
        .unwrap()
        .unwrap();
    assert!(a.wait(LIMIT).await.unwrap().success());
    assert!(
        b.try_wait().unwrap().is_none(),
        "other fixture still awaits its own input"
    );
    let mut input = b.take_stdin().unwrap();
    input.write_all(b"second").await.unwrap();
    input.flush().await.unwrap();
    drop(input);
    let mut output = Vec::new();
    tokio::time::timeout(LIMIT, b.take_stdout().unwrap().read_to_end(&mut output))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output).unwrap()["stdin"],
        "second"
    );
    assert!(b.wait(LIMIT).await.unwrap().success());
}

#[tokio::test]
async fn owner_death_closes_its_job_and_leaves_an_unrelated_child_usable() {
    let root = tempfile::tempdir().unwrap();
    let mut unrelated = idle(root.path()).await;
    let root_lock = root.path().join("root.lock");
    let leaf_lock = root.path().join("leaf.lock");
    let release = root.path().join("release");
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("owner"),
            root_lock.as_os_str(),
            leaf_lock.as_os_str(),
            release.as_os_str(),
        ],
    );
    // No outer Job may mask whether the killed owner's inner Job closes.
    spawn.lifetime = Lifetime::TrustedSupervisor;
    spawn.stdout = Stdio::Pipe;
    let mut owner = spawn.spawn().await.unwrap();
    ready(&mut owner).await;
    let lock = File::open(&leaf_lock).unwrap();
    assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
    owner.terminate().unwrap();
    owner.wait(LIMIT).await.unwrap();
    unlocked(&leaf_lock).await;
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.terminate().unwrap();
    unrelated.wait(LIMIT).await.unwrap();
}

#[tokio::test]
async fn owner_loss_at_child_startup_prevents_work_and_preserves_unrelated_process() {
    let root = tempfile::tempdir().unwrap();
    let mut unrelated = idle(root.path()).await;
    let started = root.path().join("startup.lock");
    let release = root.path().join("release-startup");
    let work = root.path().join("work");
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("owner-startup"),
            started.as_os_str(),
            release.as_os_str(),
            work.as_os_str(),
        ],
    );
    // An outer Job would mask whether the startup child's actual owner cleans it.
    spawn.lifetime = Lifetime::TrustedSupervisor;
    spawn.stdout = Stdio::Pipe;
    let mut owner = spawn.spawn().await.unwrap();
    ready(&mut owner).await;
    let lock = File::open(&started).unwrap();
    assert!(matches!(lock.try_lock(), Err(TryLockError::WouldBlock)));
    assert!(!work.exists());
    owner.terminate().unwrap();
    owner.wait(LIMIT).await.unwrap();
    unlocked(&started).await;
    assert!(
        !work.exists(),
        "startup child must not reach its work phase"
    );
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.terminate().unwrap();
    unrelated.wait(LIMIT).await.unwrap();
}

#[tokio::test]
async fn trusted_child_survives_its_creator_until_private_lifetime_eof() {
    let root = tempfile::tempdir().unwrap();
    let receipt = root.path().join("trusted-receipt");
    let mut spawn = spec(
        root.path(),
        &[OsStr::new("trusted-owner"), receipt.as_os_str()],
    );
    spawn.lifetime = Lifetime::TrustedSupervisor;
    spawn.stdout = Stdio::Pipe;
    let mut owner = spawn.spawn().await.unwrap();
    ready(&mut owner).await;
    owner.terminate().unwrap();
    owner.wait(LIMIT).await.unwrap();
    tokio::time::timeout(LIMIT, async {
        while !receipt.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("trusted child did not finish after creator EOF");
    assert_eq!(fs::read(receipt).unwrap(), b"parent-lifetime");
}

#[tokio::test]
async fn rendezvous_authenticates_the_connecting_child_and_eof_finishes_it() {
    let root = tempfile::tempdir().unwrap();
    let receipt = root.path().join("receipt");
    let listener = PrivateListener::bind().unwrap();
    assert_eq!(
        PrivateListener::bind_at(listener.address())
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("rendezvous"),
            listener.address(),
            receipt.as_os_str(),
        ],
    );
    spawn.stderr = Stdio::Pipe;
    let mut child = spawn.spawn().await.unwrap();
    let mut channel = tokio::spawn(listener.accept(&child, LIMIT))
        .await
        .unwrap()
        .unwrap();
    let mut hello = [0; 9];
    tokio::time::timeout(LIMIT, channel.read_exact(&mut hello))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&hello, b"connected");
    let mut byte = [0];
    assert!(
        tokio::time::timeout(SHORT, channel.read(&mut byte))
            .await
            .is_err()
    );
    channel.write_all(b"private-payload").await.unwrap();
    channel.flush().await.unwrap();
    drop(channel);
    assert!(child.wait(LIMIT).await.unwrap().success());
    assert_eq!(fs::read(receipt).unwrap(), b"private-payload");
}

#[tokio::test]
async fn split_duplex_tasks_keep_independent_read_and_write_wakeups() {
    let root = tempfile::tempdir().unwrap();
    let listener = PrivateListener::bind().unwrap();
    let mut child = spec(root.path(), &[OsStr::new("duplex"), listener.address()])
        .spawn()
        .await
        .unwrap();
    let channel = listener.accept(&child, LIMIT).await.unwrap();
    let (mut reader, mut writer) = tokio::io::split(channel);
    let pending_read = std::sync::Arc::new(tokio::sync::Notify::new());
    let reader_signal = pending_read.clone();
    let read = tokio::spawn(async move {
        let mut reply = [0; 9];
        {
            let operation = reader.read_exact(&mut reply);
            tokio::pin!(operation);
            std::future::poll_fn(|cx| {
                use std::future::Future;
                let result = operation.as_mut().poll(cx);
                if result.is_pending() {
                    reader_signal.notify_one();
                }
                result
            })
            .await
            .unwrap();
        }
        // Keep the reader alive until its response has been verified, without
        // polling from any third task that could hide a lost wake registration.
        assert_eq!(&reply, b"duplex-ok");
    });
    let write = tokio::spawn(async move {
        pending_read.notified().await;
        writer
            .write_all(&vec![b'x'; 8 * 1024 * 1024])
            .await
            .unwrap();
        writer.flush().await.unwrap();
    });
    let completed = tokio::time::timeout(LIMIT, async { tokio::try_join!(read, write) }).await;
    if completed.is_err() {
        child.terminate().unwrap();
        child.wait(LIMIT).await.unwrap();
        panic!("split pipe lost an independent read/write wakeup");
    }
    completed.unwrap().unwrap();
    assert!(child.wait(LIMIT).await.unwrap().success());
}

#[tokio::test]
async fn wrong_peer_and_nonlocal_names_are_rejected_without_payload() {
    let root = tempfile::tempdir().unwrap();
    let mut expected = idle(root.path()).await;
    let listener = PrivateListener::bind().unwrap();
    // The test process opens this connection, not the expected live fixture.
    let client = pipe::connect(listener.address(), LIMIT).await.unwrap();
    assert_eq!(
        listener
            .accept(&expected, LIMIT)
            .await
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    drop(client);
    for address in [
        r"\\remote\pipe\kuru-test",
        r"\\.\pipe\foreign",
        r"\\.\pipe\kuru-",
        r"\\.\pipe\kuru-bad\name",
    ] {
        assert_eq!(
            PrivateListener::bind_at(OsStr::new(address))
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            pipe::connect(OsStr::new(address), SHORT)
                .await
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let listener = PrivateListener::bind().unwrap();
    assert_eq!(
        listener
            .accept(&expected, SHORT)
            .await
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::TimedOut
    );
    expected.terminate().unwrap();
    expected.wait(LIMIT).await.unwrap();
}

#[test]
fn stalled_overlapped_write_closes_before_peer_exit_and_runtime_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let child = runtime.block_on(async {
            let listener = PrivateListener::bind().unwrap();
            let spawn = spec(
                &directory,
                &[OsStr::new("rendezvous-stall"), listener.address()],
            );
            let mut child = spawn.spawn().await.unwrap();
            let mut channel = listener.accept(&child, LIMIT).await.unwrap();
            let mut hello = [0; 9];
            tokio::time::timeout(LIMIT, channel.read_exact(&mut hello))
                .await
                .unwrap()
                .unwrap();
            let bytes = vec![b'x'; 8 * 1024 * 1024];
            // First write queues an owned buffer. The second write and flush
            // must wait for the stalled peer to consume it, not return early.
            assert_eq!(channel.write(&bytes).await.unwrap(), bytes.len());
            assert!(
                tokio::time::timeout(SHORT, channel.write_all(b"second"))
                    .await
                    .is_err()
            );
            assert!(tokio::time::timeout(SHORT, channel.flush()).await.is_err());
            assert!(child.try_wait().unwrap().is_none());
            channel.close(LIMIT).await.unwrap();
            assert!(channel.is_closed());
            channel.close(LIMIT).await.unwrap();
            assert!(
                child.try_wait().unwrap().is_none(),
                "close must not rely on peer exit"
            );
            assert_eq!(
                channel.write_all(b"closed").await.unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            drop(channel);
            child
        });
        drop(runtime);
        sender
            .send(child)
            .unwrap_or_else(|_| panic!("cleanup receiver disappeared"));
    });
    let mut child = receiver
        .recv_timeout(LIMIT * 2)
        .expect("native I/O retained runtime after close");
    thread.join().unwrap();
    assert!(
        child.try_wait().unwrap().is_none(),
        "peer must outlive runtime cleanup"
    );
    child.terminate().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(child.wait(LIMIT)).unwrap();
}

#[tokio::test]
async fn dropping_an_owned_job_releases_the_descendant_lock() {
    let root = tempfile::tempdir().unwrap();
    let root_lock = root.path().join("root.lock");
    let leaf_lock = root.path().join("leaf.lock");
    let release = root.path().join("release");
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("tree"),
            root_lock.as_os_str(),
            leaf_lock.as_os_str(),
            release.as_os_str(),
        ],
    );
    spawn.stdout = Stdio::Pipe;
    let mut child = spawn.spawn().await.unwrap();
    let mut output = BufReader::new(child.take_stdout().unwrap());
    let mut lines = String::new();
    for _ in 0..2 {
        tokio::time::timeout(LIMIT, output.read_line(&mut lines))
            .await
            .unwrap()
            .unwrap();
    }
    assert!(lines.contains("leaf-ready"));
    drop(child);
    unlocked(&leaf_lock).await;
    let mut rest = String::new();
    tokio::time::timeout(LIMIT, output.read_to_string(&mut rest))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn dropping_a_trusted_handle_does_not_terminate_its_lifetime_peer() {
    let root = tempfile::tempdir().unwrap();
    let receipt = root.path().join("receipt");
    let listener = PrivateListener::bind().unwrap();
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("rendezvous"),
            listener.address(),
            receipt.as_os_str(),
        ],
    );
    spawn.lifetime = Lifetime::TrustedSupervisor;
    let child = spawn.spawn().await.unwrap();
    let mut channel = listener.accept(&child, LIMIT).await.unwrap();
    let mut hello = [0; 9];
    tokio::time::timeout(LIMIT, channel.read_exact(&mut hello))
        .await
        .unwrap()
        .unwrap();
    drop(child);
    channel
        .write_all(b"still-owned-by-lifetime-channel")
        .await
        .unwrap();
    channel.flush().await.unwrap();
    drop(channel);
    tokio::time::timeout(LIMIT, async {
        while !receipt.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fs::read(receipt).unwrap(),
        b"still-owned-by-lifetime-channel"
    );
}

#[test]
fn cancelled_partial_frame_closes_pipe_and_reaps_peer_before_runtime_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = PrivateListener::bind().unwrap();
            let mut spawn = spec(
                &directory,
                &[OsStr::new("rendezvous-partial-frame"), listener.address()],
            );
            spawn.stdout = Stdio::Pipe;
            let mut child = spawn.spawn().await.unwrap();
            let mut channel = BufReader::new(listener.accept(&child, LIMIT).await.unwrap());
            let mut output = BufReader::new(child.take_stdout().unwrap());
            let mut sent = String::new();
            tokio::time::timeout(LIMIT, output.read_line(&mut sent))
                .await
                .expect("partial-frame peer did not acknowledge its completed write")
                .unwrap();
            assert_eq!(sent.trim(), "partial-frame-sent");
            let mut frame = Vec::new();
            // read_until retains the actual received prefix when cancelled.
            // The peer acknowledged flush and cannot supply a frame delimiter.
            assert!(
                tokio::time::timeout(SHORT, channel.read_until(b'\n', &mut frame))
                    .await
                    .is_err(),
                "an incomplete frame was reported as complete"
            );
            assert_eq!(frame, b"{\"frame\":");
            assert!(child.try_wait().unwrap().is_none());
            channel.get_mut().close(LIMIT).await.unwrap();
            assert!(channel.get_ref().is_closed());
            drop(channel);
            let mut remainder = Vec::new();
            tokio::time::timeout(LIMIT, (&mut output).take(4096).read_to_end(&mut remainder))
                .await
                .expect("partial-frame peer retained output after lifetime EOF")
                .unwrap();
            assert_eq!(remainder, b"partial-frame-eof\n");
            output.get_mut().close(LIMIT).await.unwrap();
            assert!(child.wait(LIMIT).await.unwrap().success());
        });
        drop(runtime);
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(LIMIT * 4)
        .expect("partial-frame cancellation stranded its child, pipe or Tokio runtime");
    thread.join().unwrap();
}

#[test]
fn missing_and_busy_connects_preserve_distinct_terminal_states() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let address = format!(r"\\.\pipe\kuru-{}", uuid::Uuid::new_v4());
            assert_eq!(
                pipe::connect(OsStr::new(&address), SHORT)
                    .await
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::NotFound
            );

            let listener = PrivateListener::bind().unwrap();
            let first = pipe::connect(listener.address(), LIMIT).await.unwrap();
            assert_eq!(
                pipe::connect(listener.address(), SHORT)
                    .await
                    .err()
                    .unwrap()
                    .kind(),
                io::ErrorKind::TimedOut
            );
            drop(first);
            drop(listener);
        });
        drop(runtime);
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(LIMIT)
        .expect("terminal native pipe connect states stranded the Tokio runtime");
    thread.join().unwrap();
}

#[tokio::test]
async fn diagnostic_sampling_reports_non_decreasing_resources() {
    let root = tempfile::tempdir().unwrap();
    let mut child = idle(root.path()).await;
    let diagnostic = child.duplicate_diagnostic_handle().unwrap();
    let first = sample_process(&diagnostic).unwrap();
    assert!(
        first.working_set_bytes > 0,
        "expected a positive working set"
    );
    let second = sample_process(&diagnostic).unwrap();
    assert!(second.kernel_time >= first.kernel_time);
    assert!(second.user_time >= first.user_time);
    assert!(
        second.working_set_bytes > 0,
        "expected a positive working set"
    );

    child.terminate().unwrap();
    assert!(!child.wait(LIMIT).await.unwrap().success());
}

#[tokio::test]
async fn explicit_file_stdio_and_console_group_interrupt_work() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("console-output");
    let mut spawn = spec(root.path(), &[OsStr::new("console-owner")]);
    spawn.console = Console::PrivateHidden;
    spawn.stdout = Stdio::Handle(File::create(&output).unwrap().into());
    spawn.inherited.push(
        File::create(root.path().join("explicit-extra"))
            .unwrap()
            .into(),
    );
    let mut child = spawn.spawn().await.unwrap();
    assert!(child.wait(LIMIT).await.unwrap().success());
    assert!(
        String::from_utf8(fs::read(output).unwrap())
            .unwrap()
            .contains("group-stopped")
    );
}
