use super::*;
use std::fs::OpenOptions;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink};
use std::{future::Future, task::Poll};

fn identity() -> Identity {
    Identity {
        version: 1,
        instance: Uuid::new_v4().to_string(),
        project_scope: "project/test".into(),
        password: secret(),
        reader_password: secret(),
        initialized: false,
    }
}

fn fixture() -> Result<tempfile::TempDir> {
    Ok(tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()?)
}

#[test]
fn public_lifecycle_directory_refuses_with_the_shared_safe_remedy() -> Result<()> {
    let root = fixture()?;
    let directory = root.path().join("memory");
    fs::create_dir(&directory)?;
    let sentinel = directory.join("sentinel");
    fs::write(&sentinel, b"leave server directory unchanged")?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;

    let error = LifecycleLease::new(&directory, None).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("memory data directory"), "{text}");
    assert!(text.contains("mode 0700"), "{text}");
    assert!(text.contains(&directory.display().to_string()), "{text}");
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::PermissionDenied),
        "{text}"
    );
    assert_eq!(
        fs::metadata(&directory)?.permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read(sentinel)?, b"leave server directory unchanged");
    assert!(!directory.join("lifecycle.lock").exists());
    Ok(())
}

#[test]
fn cleanup_observer_retains_real_child_and_directory_after_query_error_and_deadline() -> Result<()>
{
    use std::io::Write;
    let root = Arc::new(fixture()?);
    let path = root.path().to_owned();
    fs::write(path.join("accepted"), b"preserved until exit")?;
    let weak = Arc::downgrade(&root);
    let mut child = Command::new("/bin/sh")
        .args(["-c", "IFS= read -r token; exit 0"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().context("controlled child stdin")?;
    let (observed, observation) = std::sync::mpsc::channel();
    let (finished, completion) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut calls = 0;
        observe_supervisor(child, Some(root), Duration::ZERO, |child| {
            calls += 1;
            if calls == 1 {
                return Err(std::io::Error::other("controlled status-query failure"));
            }
            let status = child.try_wait();
            if calls == 2 {
                observed
                    .send(status.as_ref().is_ok_and(|status| status.is_none()))
                    .unwrap();
            }
            status
        });
        finished.send(()).unwrap();
    });
    assert!(observation.recv_timeout(Duration::from_secs(3))?);
    assert!(matches!(
        completion.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(
        weak.upgrade().is_some(),
        "observer lost its actual retained directory"
    );
    assert_eq!(fs::read(path.join("accepted"))?, b"preserved until exit");
    writeln!(input, "finish")?;
    drop(input);
    completion.recv_timeout(Duration::from_secs(3))?;
    worker.join().unwrap();
    assert!(
        weak.upgrade().is_none(),
        "completed observer leaked its owner"
    );
    assert!(
        !path.exists(),
        "directory must be deleted only after observed exit"
    );
    Ok(())
}

#[test]
fn owner_drop_transfers_installed_reap_guard_until_real_child_reaps() -> Result<()> {
    use std::io::Write;
    let root = Arc::new(fixture()?);
    let locks = root.path().join("locks");
    private_directory(&locks)?;
    let locks = Directory::open(&locks, Privacy::OwnerOnly, NameRetention::Pinned)?;
    let guard = locks.lock_file(std::ffi::OsStr::new("startup"))?;
    guard.lock()?;
    let contender = locks.lock_file(std::ffi::OsStr::new("startup"))?;
    let mut child = Command::new("/bin/sh")
        .args(["-c", "IFS= read -r token; exit 0"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut release = child.stdin.take().context("controlled child stdin")?;
    let reap_guard = Arc::new(StdMutex::new(Some(guard)));
    drop(Owner {
        child: Some(child),
        lifetime: None,
        retained: Some(root.clone()),
        reap_guard,
    });
    let held_before_release =
        matches!(contender.try_lock(), Err(std::fs::TryLockError::WouldBlock));
    let release_result = writeln!(release, "finish").map_err(anyhow::Error::from);
    drop(release);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let reaped = loop {
        match contender.try_lock() {
            Ok(()) => break Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                break Err(anyhow!("observer did not release guard after child reap"));
            }
            Err(error) => break Err(error.into()),
        }
    };
    match (held_before_release, release_result, reaped) {
        (true, Ok(()), Ok(())) => Ok(()),
        (false, Ok(()), Ok(())) => {
            bail!("owner drop released the installed reap guard before child exit")
        }
        (_, Err(release), Ok(())) => Err(release),
        (_, Ok(()), Err(reap)) => Err(reap),
        (_, Err(release), Err(reap)) => Err(release.context(format!(
            "observer cleanup also failed after control-release failure: {reap:#}"
        ))),
    }
}

#[tokio::test]
async fn cleanup_observation_error_keeps_actual_lifecycle_lease_until_child_exit() -> Result<()> {
    let root = fixture()?;
    let store = root.path().join("store");
    private_directory(&store)?;
    let lease = Server::quiescence(&store, Duration::from_secs(1)).await?;
    let home = root.path().join("home");
    crate::provision::prepare_private_home(&home)?;
    let script = root.path().join("controlled-child");
    fs::write(
        &script,
        b"#!/bin/sh\nIFS= read -r token < \"$TMPDIR/release\"\n",
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let fifo = home.join("tmp/release");
    nix::unistd::mkfifo(
        &fifo,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )?;
    let release = nix::fcntl::open(
        &fifo,
        nix::fcntl::OFlag::O_RDWR | nix::fcntl::OFlag::O_NONBLOCK,
        nix::sys::stat::Mode::empty(),
    )?;
    let mut child =
        crate::engine::spawn(&script, &home, root.path(), Vec::new(), Vec::new(), true).await?;
    let (observed, observation) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut observed = Some(observed);
        observe_dolt(&mut child, |child| {
            if let Some(observed) = observed.take() {
                observed.send(()).unwrap();
                return Err(std::io::Error::other(
                    "controlled retained-child query failure",
                ));
            }
            child.try_wait()
        })
        .await;
        drop(lease);
    });
    tokio::time::timeout(Duration::from_secs(3), observation).await??;
    assert!(
        Server::quiescence(&store, Duration::from_millis(30))
            .await
            .is_err(),
        "query failure released a live child's actual lease"
    );
    nix::unistd::write(&release, b"finish\n")?;
    tokio::time::timeout(Duration::from_secs(3), task).await??;
    drop(release);
    drop(Server::quiescence(&store, Duration::from_secs(1)).await?);
    Ok(())
}

#[test]
fn private_records_validate_identity_and_reject_links_and_unknown_files() -> Result<()> {
    let root = fixture()?;
    let directory = root.path().join("private");
    prepare_directory(&directory, false)?;
    assert!(load_identity(&directory, "project/test")?.is_none());
    let mut record = identity();
    let path = directory.join("identity.json");
    write_record(&path, &record)?;
    assert!(load_identity(&directory, "project/other").is_err());
    assert!(load_identity(&directory, "project/test")?.is_some());
    record.password = "short".into();
    write_record(&path, &record)?;
    assert!(load_identity(&directory, "project/test").is_err());
    record = identity();
    record.instance = "not-a-uuid".into();
    write_record(&path, &record)?;
    assert!(load_identity(&directory, "project/test").is_err());
    write_private(&path, b"{broken")?;
    assert!(read_record::<Identity>(&path).is_err());
    write_private(&path, &vec![b'x'; RECORD_LIMIT + 1])?;
    assert!(read_record::<Identity>(&path).is_err());
    assert!(write_record(&path, &"x".repeat(RECORD_LIMIT)).is_err());
    fs::remove_file(&path)?;
    let other = root.path().join("outside");
    write_private(&other, b"untouched")?;
    symlink(&other, &path)?;
    assert!(read_record::<Identity>(&path).is_err());
    assert!(write_private(&path, b"bad").is_err());
    assert!(prepare_directory(&directory, false).is_err());
    fs::remove_file(&path)?;
    fs::hard_link(&other, &path)?;
    assert!(read_record::<Identity>(&path).is_err());
    fs::remove_file(&path)?;
    assert_eq!(fs::read(&other)?, b"untouched");
    fs::write(directory.join("unknown"), b"keep")?;
    assert!(prepare_directory(&directory, false).is_err());
    assert_eq!(fs::read(directory.join("unknown"))?, b"keep");
    Ok(())
}

#[test]
fn private_layout_and_branch_validation_are_strict() -> Result<()> {
    let root = fixture()?;
    let missing = root.path().join("missing");
    assert!(prepare_directory(&missing, true).is_err());
    private_directory(&missing)?;
    fs::set_permissions(&missing, fs::Permissions::from_mode(0o755))?;
    assert!(private_directory(&missing).is_err());
    let linked = root.path().join("linked");
    symlink(root.path(), &linked)?;
    assert!(private_directory(&linked).is_err());
    for invalid in [
        "",
        "main/other",
        "../main",
        "main;DROP DATABASE kuru",
        "space name",
        "é",
    ] {
        assert!(validate_branch(invalid).is_err(), "{invalid}");
    }
    assert!(validate_branch(&"a".repeat(129)).is_err());
    for valid in ["main", "dream_012345", "candidate-a"] {
        validate_branch(valid)?;
    }
    let yaml = server_yaml(
        &root.path().join("space café \"quoted\""),
        55000,
        Duration::from_secs(20),
    )?;
    assert!(yaml.contains("host: 127.0.0.1"));
    assert!(yaml.contains("event_scheduler: \"OFF\""));
    assert!(yaml.contains("auto_gc_behavior:\n    enable: true"));
    assert!(yaml.contains("\\\"quoted\\\""));
    assert!(!yaml.contains("password:"));
    assert!(yaml.contains("read_timeout_millis: 30000"));
    let longest = server_yaml(root.path(), 55000, Duration::from_secs(300))?;
    assert!(longest.contains("read_timeout_millis: 300000"));
    Ok(())
}

#[tokio::test]
async fn framing_is_bounded_and_diagnostics_keep_only_a_tail() -> Result<()> {
    let (mut write, mut read) = tokio::io::duplex(1024);
    let writer =
        tokio::spawn(
            async move { write_frame(&mut write, &Response::Failed("sample".into())).await },
        );
    assert!(
        matches!(read_frame::<_, Response>(&mut read).await?, Response::Failed(message) if message == "sample")
    );
    writer.await??;
    for bytes in [
        0_u32.to_be_bytes().to_vec(),
        ((RECORD_LIMIT + 1) as u32).to_be_bytes().to_vec(),
        vec![0, 0, 0, 1, b'{'],
        vec![0, 0, 0, 2, b'{'],
    ] {
        assert!(
            read_frame::<_, Response>(&mut bytes.as_slice())
                .await
                .is_err()
        );
    }
    let mut sink = tokio::io::sink();
    assert!(
        write_frame(&mut sink, &"x".repeat(RECORD_LIMIT))
            .await
            .is_err()
    );
    let mut data = vec![b'a'; LOG_LIMIT];
    data.extend(vec![b'b'; LOG_LIMIT]);
    let log = Arc::new(Mutex::new(Vec::new()));
    drain(data.as_slice(), log.clone()).await;
    assert_eq!(*log.lock().await, vec![b'b'; LOG_LIMIT]);
    Ok(())
}

#[tokio::test]
async fn supervisor_rejects_bad_configuration_and_parent_eof_without_spawning() -> Result<()> {
    let root = fixture()?;
    let request = || Request {
        binary: "/unexecuted".into(),
        directory: fs::canonicalize(root.path()).expect("fixture canonical path"),
        project_scope: "project/test".into(),
        timeout_millis: 100,
        read_only: false,
        lifecycle_root: None,
    };
    let mut input = tokio::io::empty();
    let mut output = Vec::new();
    let mut invalid = request();
    invalid.timeout_millis = 0;
    assert!(supervise(invalid, &mut input, &mut output).await.is_err());
    let lock = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.path().join("lifecycle.lock"))?;
    lock.lock()?;
    let error = supervise(request(), &mut input, &mut output)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("parent closed"), "{error:#}");
    assert!(output.is_empty());
    drop(lock);
    let mut invalid = request();
    invalid.read_only = true;
    assert!(
        supervise(invalid, &mut input, &mut output)
            .await
            .unwrap_err()
            .to_string()
            .contains("not been initialized")
    );
    Ok(())
}

#[tokio::test]
async fn open_rejects_invalid_options_before_executable_lookup() -> Result<()> {
    let root = fixture()?;
    let options = || ServerOptions {
        binary: "/unexecuted".into(),
        directory: root.path().to_path_buf(),
        project_scope: "project/test".into(),
        supervisor: "/unexecuted".into(),
        timeout: Duration::from_secs(1),
        read_only: false,
        retained: None,
        lifecycle_root: None,
    };
    let mut invalid = options();
    invalid.timeout = Duration::ZERO;
    assert!(Server::open(invalid).await.is_err());
    let mut invalid = options();
    invalid.project_scope.clear();
    assert!(Server::open(invalid).await.is_err());
    let mut invalid = options();
    invalid.read_only = true;
    assert!(Server::open(invalid).await.is_err());
    assert!(
        Server::open(options())
            .await
            .unwrap_err()
            .to_string()
            .contains("supervisor")
    );
    Ok(())
}

#[tokio::test]
async fn quiescence_waits_for_the_actual_lease_and_holds_it_through_rename() -> Result<()> {
    let root = fixture()?;
    let stage = root.path().join("stage");
    private_directory(&stage)?;
    assert!(Server::quiescence(&stage, Duration::ZERO).await.is_err());
    let owner = Server::quiescence(&stage, Duration::from_secs(1)).await?;
    assert!(
        Server::quiescence(&stage, Duration::from_millis(10))
            .await
            .is_err()
    );
    let original = owner.metadata()?;
    let destination = root.path().join("active");
    fs::rename(&stage, &destination)?;
    File::open(root.path())?.sync_all()?;
    let moved = fs::metadata(destination.join("lifecycle.lock"))?;
    assert_eq!((original.dev(), original.ino()), (moved.dev(), moved.ino()));
    assert!(
        Server::quiescence(&destination, Duration::from_millis(10))
            .await
            .is_err()
    );
    drop(owner);
    drop(Server::quiescence(&destination, Duration::from_secs(1)).await?);
    assert!(destination.join("lifecycle.lock").exists());
    Ok(())
}

#[tokio::test]
async fn quiescence_rejects_replaced_lock_instead_of_locking_an_orphan_inode() -> Result<()> {
    let root = fixture()?;
    let stage = root.path().join("stage");
    private_directory(&stage)?;
    let owner = Server::quiescence(&stage, Duration::from_secs(1)).await?;
    let mut waiter = Box::pin(Server::quiescence(&stage, Duration::from_secs(1)));
    std::future::poll_fn(|context| {
        assert!(waiter.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    fs::rename(stage.join("lifecycle.lock"), root.path().join("old-lock"))?;
    write_private(&stage.join("lifecycle.lock"), b"replacement untouched")?;
    drop(owner);
    let error = waiter.await.unwrap_err();
    assert!(
        error.to_string().contains("lock was moved or replaced"),
        "{error:#}"
    );
    assert_eq!(
        fs::read(stage.join("lifecycle.lock"))?,
        b"replacement untouched"
    );
    Ok(())
}

#[tokio::test]
async fn waiting_supervisor_refuses_recreated_stage_after_original_lock_moves() -> Result<()> {
    let root = fixture()?;
    let stage = fs::canonicalize(root.path())?.join("stage");
    private_directory(&stage)?;
    let owner = Server::quiescence(&stage, Duration::from_secs(1)).await?;
    let request = Request {
        binary: "/unexecuted".into(),
        directory: stage.clone(),
        project_scope: "project/test".into(),
        timeout_millis: 1000,
        read_only: false,
        lifecycle_root: None,
    };
    let (_parent, mut input) = tokio::io::duplex(1024);
    let mut output = Vec::new();
    let mut waiting = Box::pin(supervise(request, &mut input, &mut output));
    std::future::poll_fn(|context| {
        assert!(waiting.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    let moved = root.path().join("active");
    fs::rename(&stage, &moved)?;
    private_directory(&stage)?;
    drop(owner);
    let error = waiting.await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("directory was moved or replaced"),
        "{error:#}"
    );
    assert!(
        fs::read_dir(&stage)?.next().is_none(),
        "stale supervisor wrote into replacement directory"
    );
    assert!(!moved.join("identity.json").exists());
    assert!(output.is_empty());
    Ok(())
}

#[tokio::test]
async fn closing_an_attached_handle_does_not_establish_quiescence() -> Result<()> {
    let root = fixture()?;
    let directory = root.path().join("server");
    let config = kuru_core::MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let binary = crate::provision::provision(&config, &crate::store::test_cache()).await?;
    let options = ServerOptions {
        binary,
        directory: directory.clone(),
        project_scope: "project/test".into(),
        supervisor: crate::store::test_supervisor()?,
        timeout: Duration::from_secs(20),
        read_only: false,
        retained: None,
        lifecycle_root: None,
    };
    let owner = Server::open(options.clone()).await?;
    let attached = Server::open(ServerOptions {
        read_only: true,
        ..options
    })
    .await?;
    attached.close().await?;
    let premature = Server::quiescence(&directory, Duration::from_millis(20)).await;
    owner.close().await?;
    assert!(
        premature.is_err(),
        "attached close incorrectly allowed directory activation"
    );
    let lease = Server::quiescence(&directory, Duration::from_secs(1)).await?;
    assert!(!directory.join("endpoint.json").exists());
    drop(lease);
    Ok(())
}
