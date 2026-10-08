#![cfg(all(windows, feature = "test-support"))]

use kuru_platform::windows::{
    pipe::{self, Pipe, PrivateListener, PrivateServiceListener},
    process::{
        Console, Lifetime, NativeChild, NativeSpawnSpec, ProcessStamp, RootObservation, Stdio,
        current_process_handle, duplicate_inherited_process_handle, process_object_retained,
        process_stamp, sample_process, wait_process_handle,
    },
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, TryLockError},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// The only wall-clock bound the platform owns that these tests reach: the
/// stdio pipe accept in `kuru_platform::windows::pipe::stdio`
/// (`accept_connection(Duration::from_secs(5))`), which every `Stdio::Pipe`
/// spawn awaits. The fixture child's own connect, accept, close and wait
/// bounds are the same 5 s (`tests/fixtures/process.rs`), so a fixture's own
/// error is observed before the parent's wait on it ends.
const NATIVE_BOUND: Duration = Duration::from_secs(5);

/// Allowance for creating a fixture child process and its runtime before its
/// own work begins, restated from
/// `kuru_memory::test_support::CHILD_START_MARGIN` (`cfg(test)` and
/// crate-private there).
const CHILD_START: Duration = Duration::from_secs(5);

/// One wait's failure bound in this file: [`NATIVE_BOUND`] plus
/// [`CHILD_START`], 10 s. [`budget_arg`] adds one to each budgeted fixture's
/// self-timeout.
///
/// No product bound applies to these waits. The platform's `Pipe::close`,
/// `accept_connection` (through both listeners' `accept`), `pipe::connect`,
/// `NativeChild::wait` and `wait_process_handle` take their bound from the
/// caller, and `NativeChild::terminate` is an unbounded request proved by a
/// later wait: the test is the caller, and `LIMIT` is its bound for the native
/// operation plus a child started before it. Readiness and handshake reads
/// from a just-started child, the in-process service-pipe I/O, polls and reads
/// of fixture work and the 8 MiB duplex join are pure fixture work; for them
/// `LIMIT` is the fixture's stated child-start-plus-work allowance. Each wait
/// ends on its event, so the bound decides only a failure, and it outlives
/// the fixture's own 5 s bounds. The fixture's 20 s release deadlines are not
/// derived from it.
const LIMIT: Duration = NATIVE_BOUND.saturating_add(CHILD_START);
const SHORT: Duration = Duration::from_millis(80);

#[tokio::test]
async fn process_queries_refuse_file_handles_and_mismatched_identity_without_closing_sources() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("retained-file");
    fs::write(&path, b"retained file bytes").unwrap();
    let file: OwnedHandle = File::open(&path).unwrap().into();
    assert!(sample_process(&file).is_err());
    assert!(process_stamp(&file).is_err());
    assert!(wait_process_handle(&file, SHORT).await.is_err());
    assert!(
        duplicate_inherited_process_handle(file.as_raw_handle() as usize, std::process::id())
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"retained file bytes");

    let process = current_process_handle().unwrap();
    let stamp = process_stamp(&process).unwrap();
    for (value, expected_pid) in [
        (0, stamp.id),
        (usize::MAX, stamp.id),
        (process.as_raw_handle() as usize, 0),
        (process.as_raw_handle() as usize, stamp.id + 1),
    ] {
        assert_eq!(
            duplicate_inherited_process_handle(value, expected_pid)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
    let duplicate =
        duplicate_inherited_process_handle(process.as_raw_handle() as usize, stamp.id).unwrap();
    assert_eq!(process_stamp(&duplicate).unwrap(), stamp);
    drop(duplicate);
    assert_eq!(process_stamp(&process).unwrap(), stamp);
    assert_eq!(
        wait_process_handle(&process, Duration::ZERO)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

/// The bound of an outer wait over `limits` inner waits bounded by [`LIMIT`]
/// and `shorts` bounded by [`SHORT`], run in series, so each inner wait has its
/// own bound first. A caller whose span has a step no inner wait bounds (child
/// creation, runtime construction or shutdown) adds one `LIMIT` for it.
fn series(limits: u32, shorts: u32) -> Duration {
    LIMIT * limits + SHORT * shorts
}

/// A budgeted fixture mode's last argument: its orphan-reaping self-timeout,
/// which the fixture uses as is. `budget` is a [`series`] of the parent's waits
/// from the spawn until it no longer needs the fixture running. The added
/// [`LIMIT`] covers the parent's unbounded steps after its last counted wait
/// (a thread join, a synchronous lock check, `try_wait`) and the gap between
/// the parent's clock and the fixture's, which starts only once it runs. So
/// the parent's waits decide first, and the self-timeout only reaps an orphan.
fn budget_arg(budget: Duration) -> OsString {
    budget.saturating_add(LIMIT).as_millis().to_string().into()
}

#[tokio::test]
async fn pipe_direction_empty_io_and_closed_operations_preserve_child_ownership() {
    let root = tempfile::tempdir().unwrap();
    let mut spawn = spec(root.path(), &[OsStr::new("capture")]);
    spawn.stdin = Stdio::Pipe;
    spawn.stdout = Stdio::Pipe;
    let mut child = spawn.spawn().await.unwrap();
    let mut input = child.take_stdin().unwrap();
    let mut output = child.take_stdout().unwrap();
    assert_eq!(input.write(&[]).await.unwrap(), 0);
    assert_eq!(output.read(&mut []).await.unwrap(), 0);
    assert_eq!(
        input.read(&mut [0]).await.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        output.write(b"wrong direction").await.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(child.interrupt().is_err(), "no console group was requested");
    assert!(
        child.try_wait().unwrap().is_none(),
        "refused operations ended the child"
    );
    input.write_all(b"owned input").await.unwrap();
    tokio::time::timeout(LIMIT, input.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(input.is_closed());
    input.close(LIMIT).await.unwrap();
    assert_eq!(
        input.write(b"after close").await.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        input.flush().await.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    let mut bytes = Vec::new();
    tokio::time::timeout(LIMIT, output.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["stdin"], "owned input");
    assert!(child.wait(LIMIT).await.unwrap().success());
    child.terminate().unwrap();
    output.close(LIMIT).await.unwrap();
    assert_eq!(
        output.read(&mut [0]).await.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn process_diagnostics_retain_failed_observations_and_partial_member_details() {
    use kuru_platform::windows::process::{JobMember, JobSnapshot, TreeSnapshot};

    let snapshot = TreeSnapshot {
        root_id: 42,
        root: Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "root query denied",
        )),
        root_sample: Err(io::Error::other("root sample unavailable")),
        job: Some(Ok(JobSnapshot {
            active_processes: 2,
            total_processes: 3,
            terminated_processes: 1,
            assigned_processes: 2,
            members: vec![
                JobMember {
                    id: 43,
                    image: Ok("retained fixture.exe".into()),
                    sample: Err(io::Error::other("member sample unavailable")),
                },
                JobMember {
                    id: 44,
                    image: Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "member query denied",
                    )),
                    sample: Err(io::Error::other("second sample unavailable")),
                },
            ],
        })),
    };
    let text = snapshot.to_string();
    for expected in [
        "root pid=42",
        "state_error=root query denied",
        "sample_error=root sample unavailable",
        "active=2",
        "listed=2/2",
        "pid=43",
        "image=retained fixture.exe",
        "pid=44",
        "image_error=member query denied",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    let snapshot = TreeSnapshot {
        root_id: 42,
        root: Ok(RootObservation::Exited(7)),
        root_sample: Err(io::Error::other("unavailable")),
        job: Some(Err(io::Error::other("job query unavailable"))),
    };
    assert!(snapshot.to_string().contains("state=exited(7)"));
    assert!(
        snapshot
            .to_string()
            .contains("job_error=job query unavailable")
    );
}

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

/// A parked fixture that only termination ends. `budget` is the parent's wait
/// on it from its spawn until the parent no longer needs it running.
async fn idle(root: &Path, budget: Duration) -> NativeChild {
    let mut spawn = spec(root, &[OsStr::new("idle")]);
    spawn.args.push(budget_arg(budget));
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

async fn root_exited(child: &NativeChild) {
    let deadline = tokio::time::Instant::now() + LIMIT;
    while !child.fixture_root_has_exited().unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "fixture root did not exit"
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
async fn independent_service_remains_in_a_job_that_forbids_breakaway() {
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
    starter.stdout = Stdio::Pipe;
    let mut starter = starter.spawn().await.unwrap();
    let mut reader = BufReader::new(starter.take_stdout().unwrap());
    let mut line = String::new();
    tokio::time::timeout(LIMIT, reader.read_line(&mut line))
        .await
        .expect("contained starter readiness deadline")
        .unwrap();
    assert_eq!(line, "starter-exited\n");
    root_exited(&starter).await;
    let held = File::options()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(
        matches!(held.try_lock(), Err(TryLockError::WouldBlock)),
        "contained service exited with its starter"
    );
    drop(held);
    drop(starter);
    unlocked(&lock_path).await;
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
    // Readiness, then the probe that must time out on the running tree.
    let mut child = idle(root.path(), series(1, 1)).await;
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

#[tokio::test]
async fn diagnostic_snapshot_lists_job_members_without_changing_the_tree() {
    let root = tempfile::tempdir().unwrap();
    let release = root.path().join("release");
    let mut spawn = spec(
        root.path(),
        &[
            OsStr::new("tree"),
            root.path().join("root.lock").as_os_str(),
            root.path().join("leaf.lock").as_os_str(),
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

    let snapshot = child.diagnostic_snapshot();
    let text = snapshot.to_string();
    assert_eq!(snapshot.root_id, child.id());
    assert!(
        matches!(
            snapshot.root,
            Ok(RootObservation::Running | RootObservation::Exited(0))
        ),
        "{text}"
    );
    let job = snapshot.job.as_ref().unwrap().as_ref().unwrap();
    assert!(job.active_processes >= 1, "{text}");
    assert!(job.total_processes >= 2, "{text}");
    assert_eq!(job.members.len(), job.assigned_processes as usize, "{text}");
    // The leaf is still blocked on its release file, so it is a live member
    // distinct from the root, named by its image. Other members, such as a
    // console host, may also be listed.
    let leaf = job
        .members
        .iter()
        .find(|member| {
            member.id != child.id()
                && member
                    .image
                    .as_deref()
                    .ok()
                    .and_then(|image| Path::new(image).file_name())
                    == Some(OsStr::new("kuru-platform-process-fixture.exe"))
        })
        .unwrap_or_else(|| panic!("no descendant fixture member: {text}"));
    assert!(leaf.sample.is_ok(), "{text}");
    // The image is the member's full executable path, not only its file name.
    let image = Path::new(leaf.image.as_deref().unwrap());
    assert!(image.is_absolute(), "{text}");
    assert_eq!(
        fs::canonicalize(image).unwrap(),
        fs::canonicalize(env!("CARGO_BIN_EXE_kuru-platform-process-fixture")).unwrap(),
        "{text}"
    );
    assert!(
        text.contains(&format!("root pid={} ", child.id())),
        "{text}"
    );
    assert!(
        text.contains(&format!("pid={} image={} cpu=", leaf.id, image.display())),
        "{text}"
    );
    // Observation neither cached nor ended anything.
    assert!(child.try_wait().unwrap().is_none(), "{text}");

    fs::write(release, b"go").unwrap();
    assert!(child.wait(LIMIT).await.unwrap().success());
    let mut remainder = String::new();
    tokio::time::timeout(LIMIT, output.read_to_string(&mut remainder))
        .await
        .unwrap()
        .unwrap();
    let text = child.diagnostic_snapshot().to_string();
    assert!(text.contains("state=exited(0)"), "{text}");
    assert!(text.contains("job active=0 "), "{text}");
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
    // Its own readiness, the owner's creation (which no wait bounds), the
    // owner's readiness and reap, and the leaf's release before the check
    // that it still runs.
    let mut unrelated = idle(root.path(), series(5, 0)).await;
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
    // Its readiness is the only wait before termination.
    spawn.args.push(budget_arg(LIMIT));
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
    // Its own readiness, the owner's creation (which no wait bounds), the
    // owner's readiness and reap, and the startup lock's release before the
    // check that it still runs.
    let mut unrelated = idle(root.path(), series(5, 0)).await;
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
    // Its readiness is the only wait before termination.
    spawn.args.push(budget_arg(LIMIT));
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
    // Its readiness is the only wait before termination.
    spawn.args.push(budget_arg(LIMIT));
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
    // Its readiness, the test's connect and rejected accept, then four
    // rejected connects and the accept that must time out on it.
    let mut expected = idle(root.path(), series(3, 5)).await;
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

#[tokio::test]
async fn peer_exit_fails_a_queued_write_without_releasing_an_unrelated_child() {
    let root = tempfile::tempdir().unwrap();
    // Accept/read, the pending flush probe, and peer reap are the waits before
    // this parked peer is ended. Later error observations do not need it alive.
    // Its readiness, accept/read, peer reap, flush/EOF, two closes and final
    // peer wait precede unrelated termination; one LIMIT covers child creation.
    let mut unrelated = idle(root.path(), series(10, 1)).await;
    let listener = PrivateListener::bind().unwrap();
    let mut spawn = spec(
        root.path(),
        &[OsStr::new("rendezvous-stall"), listener.address()],
    );
    spawn.args.push(budget_arg(series(3, 1)));
    let mut child = spawn.spawn().await.unwrap();
    let mut channel = listener.accept(&child, LIMIT).await.unwrap();
    let observed: io::Result<_> = async {
        let mut hello = [0; 9];
        tokio::time::timeout(LIMIT, channel.read_exact(&mut hello))
            .await
            .map_err(io::Error::other)??;
        let queued = channel.write(&vec![b'x'; 8 * 1024 * 1024]).await?;
        let pending = tokio::time::timeout(SHORT, channel.flush()).await.is_err();
        child.terminate()?;
        let status = child.wait(LIMIT).await?;
        let flush = tokio::time::timeout(LIMIT, channel.flush())
            .await
            .map_err(io::Error::other)?;
        let mut byte = [0];
        let eof = tokio::time::timeout(LIMIT, channel.read(&mut byte))
            .await
            .map_err(io::Error::other)??;
        Ok((
            hello,
            queued,
            pending,
            status,
            flush,
            eof,
            unrelated.try_wait()?.is_none(),
        ))
    }
    .await;
    let closed = channel.close(LIMIT).await;
    let closed_again = channel.close(LIMIT).await;
    let child_settled = async {
        child.terminate()?;
        child.wait(LIMIT).await
    }
    .await;
    let unrelated_settled = async {
        unrelated.terminate()?;
        unrelated.wait(LIMIT).await
    }
    .await;
    closed.unwrap();
    closed_again.unwrap();
    child_settled.unwrap();
    unrelated_settled.unwrap();
    let (hello, queued, pending, status, flush, eof, unrelated_live) = observed.unwrap();
    assert_eq!(&hello, b"connected");
    assert_eq!(queued, 8 * 1024 * 1024);
    assert!(
        pending,
        "the live peer unexpectedly consumed the queued write"
    );
    assert!(!status.success());
    assert_eq!(flush.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(eof, 0);
    assert!(unrelated_live, "peer I/O failure changed an unrelated Job");
    assert!(channel.is_closed());
}

#[test]
fn dropping_pending_pipe_io_does_not_wait_for_peer_or_tokio_runtime() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_owned();
    // Accept/read, the two pending probes, and one LIMIT for unbounded child
    // creation and runtime construction/shutdown. budget_arg adds the gap
    // through thread join and the parent's subsequent owned termination.
    let budget = series(3, 2);
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (child, observed) = runtime.block_on(async {
            let listener = PrivateListener::bind().unwrap();
            let mut spawn = spec(
                &directory,
                &[OsStr::new("rendezvous-stall"), listener.address()],
            );
            spawn.args.push(budget_arg(budget));
            let mut child = spawn.spawn().await.unwrap();
            let mut channel = listener.accept(&child, LIMIT).await.unwrap();
            let observed: io::Result<_> = async {
                let mut hello = [0; 9];
                tokio::time::timeout(LIMIT, channel.read_exact(&mut hello))
                    .await
                    .map_err(io::Error::other)??;
                let queued = channel.write(&vec![b'x'; 8 * 1024 * 1024]).await?;
                let write_pending = tokio::time::timeout(SHORT, channel.flush()).await.is_err();
                let mut byte = [0];
                let read_pending = tokio::time::timeout(SHORT, channel.read(&mut byte))
                    .await
                    .is_err();
                Ok((
                    hello,
                    queued,
                    write_pending,
                    read_pending,
                    child.try_wait()?.is_none(),
                ))
            }
            .await;
            // Deliberately exercise Drop, not close: native resources must stay
            // owned until cancellation completes, independently of this runtime.
            drop(channel);
            (child, observed)
        });
        drop(runtime);
        sender
            .send((child, observed))
            .unwrap_or_else(|_| panic!("pending-I/O cleanup receiver disappeared"));
    });
    let (mut child, observed) = receiver
        .recv_timeout(budget)
        .expect("pending pipe Drop stranded Tokio runtime shutdown");
    thread.join().unwrap();
    let live_after_shutdown = child.try_wait();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    child.terminate().unwrap();
    let settled = runtime.block_on(child.wait(LIMIT));
    drop(runtime);
    settled.unwrap();
    let (hello, queued, write_pending, read_pending, live_before_drop) = observed.unwrap();
    assert_eq!(&hello, b"connected");
    assert_eq!(queued, 8 * 1024 * 1024);
    assert!(
        write_pending && read_pending,
        "both native I/O directions must be pending"
    );
    assert!(live_before_drop);
    assert!(
        live_after_shutdown.unwrap().is_none(),
        "Drop relied on peer exit"
    );
}

#[test]
fn stalled_overlapped_write_closes_before_peer_exit_and_runtime_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().to_owned();
    // The thread's accept, hello read and two closes, plus one LIMIT for the
    // child's creation and the runtime's construction and shutdown, which no
    // inner wait bounds; and its two probes that must time out. The stalled
    // peer is passed the same budget, so it is parked past this wait.
    let budget = series(5, 2);
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let child = runtime.block_on(async {
            let listener = PrivateListener::bind().unwrap();
            let mut spawn = spec(
                &directory,
                &[OsStr::new("rendezvous-stall"), listener.address()],
            );
            spawn.args.push(budget_arg(budget));
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
        .recv_timeout(budget)
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
    // The thread's accept, readiness read, two closes, remainder read and
    // reap, plus one LIMIT for the child's creation and the runtime's
    // construction and shutdown, which no inner wait bounds; and its one probe
    // that must time out. The peer is passed the same budget: it ends on the
    // EOF of this thread's close, inside this series, so its self-timeout
    // cannot end it first.
    let budget = series(7, 1);
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
            spawn.args.push(budget_arg(budget));
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
        .recv_timeout(budget)
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
            let missing = pipe::connect(OsStr::new(&address), SHORT)
                .await
                .err()
                .unwrap();
            assert_eq!(missing.kind(), io::ErrorKind::NotFound);
            assert!(!pipe::is_no_free_instance(&missing));

            // The only instance is connected, so every retry sees a busy pipe.
            let listener = PrivateListener::bind().unwrap();
            let first = pipe::connect(listener.address(), LIMIT).await.unwrap();
            let busy = pipe::connect(listener.address(), SHORT)
                .await
                .err()
                .unwrap();
            assert_eq!(busy.kind(), io::ErrorKind::TimedOut);
            assert_eq!(busy.to_string(), "private pipe connect timed out");
            assert!(pipe::is_no_free_instance(&busy));
            // The marker is the typed payload, never the kind or the text.
            assert!(!pipe::is_no_free_instance(&io::Error::new(
                io::ErrorKind::TimedOut,
                "private pipe connect timed out",
            )));
            drop(first);
            drop(listener);
        });
        drop(runtime);
        sender.send(()).unwrap();
    });
    // The thread's one bounded connect, plus one LIMIT for the runtime's
    // construction and shutdown, which no inner wait bounds; and its two
    // connects that must fail at once or time out.
    receiver
        .recv_timeout(series(2, 2))
        .expect("terminal native pipe connect states stranded the Tokio runtime");
    thread.join().unwrap();
}

#[tokio::test]
async fn diagnostic_sampling_reports_non_decreasing_resources() {
    let root = tempfile::tempdir().unwrap();
    // Its readiness is the only wait before termination.
    let mut child = idle(root.path(), LIMIT).await;
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

#[tokio::test]
async fn process_object_retained_matches_only_the_exact_identity() {
    let current = process_stamp(&current_process_handle().unwrap()).unwrap();
    assert_eq!(current.id, std::process::id());
    assert!(process_object_retained(current).unwrap());

    let root = tempfile::tempdir().unwrap();
    // Its readiness is the only wait before termination.
    let mut child = idle(root.path(), LIMIT).await;
    let stamp = child.stamp().unwrap();
    assert_eq!(stamp.id, child.id());
    assert_ne!(stamp.created, 0);
    // A duplicated handle keeps the exited child's process object alive after
    // the owner reaps and drops it: exactly the hold the measurement reports.
    let retained = child.duplicate_process_handle().unwrap();
    child.terminate().unwrap();
    assert!(!child.wait(LIMIT).await.unwrap().success());
    drop(child);
    assert!(
        process_object_retained(stamp).unwrap(),
        "an exited process object that a handle still holds is retained"
    );
    // The same id with another creation time is a reused id, never the child.
    assert!(
        !process_object_retained(ProcessStamp {
            id: stamp.id,
            created: stamp.created + 1,
        })
        .unwrap()
    );
    // No control asserts `false` once our duplicate closes: another process
    // may legitimately still hold the object, which is what the measurement
    // exists to report.
    drop(retained);
}
