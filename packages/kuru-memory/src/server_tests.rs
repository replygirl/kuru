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
    let mut child = {
        // Held across the spawn; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning_blocking();
        Command::new("/bin/sh")
            .args(["-c", "IFS= read -r token; exit 0"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?
    };
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
    {
        // Held across the actual flock acquisition; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking();
        guard.lock()?;
    }
    let contender = locks.lock_file(std::ffi::OsStr::new("startup"))?;
    let mut child = {
        // Held across the spawn; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning_blocking();
        Command::new("/bin/sh")
            .args(["-c", "IFS= read -r token; exit 0"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?
    };
    let mut release = child.stdin.take().context("controlled child stdin")?;
    let reap_guard = Arc::new(StdMutex::new(Some(guard)));
    {
        // Held across the guard's drop (the actual flock release this test
        // is exercising); see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking();
        drop(Owner {
            child: Some(child),
            lifetime: None,
            retained: Some(root.clone()),
            reap_guard,
            reaped_observer: None,
        });
    }
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
    let lease = {
        // Held across the actual flock acquisition; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        Server::quiescence(&store, Duration::from_secs(1)).await?
    };
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
    let mut child = {
        // Held across the spawn; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        crate::engine::spawn(&script, &home, root.path(), Vec::new(), Vec::new(), true).await?
    };
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
        // Held across the actual flock release; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
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
    drop({
        // Held across the actual flock acquisition; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        Server::quiescence(&store, Duration::from_secs(1)).await?
    });
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
    let lock = {
        // Held across the actual flock acquisition; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        let lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(root.path().join("lifecycle.lock"))?;
        lock.lock()?;
        lock
    };
    let error = supervise(request(), &mut input, &mut output)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("parent closed"), "{error:#}");
    assert!(output.is_empty());
    {
        // Held across the actual flock release; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        drop(lock);
    }
    let mut invalid = request();
    invalid.read_only = true;
    let error = supervise(invalid, &mut input, &mut output)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("not been initialized"),
        "{error:#}"
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
    let error = {
        // Held across the attempted (failing) supervisor spawn; see
        // `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        Server::open(options()).await.unwrap_err()
    };
    assert!(error.to_string().contains("supervisor"), "{error:#}");
    Ok(())
}

#[tokio::test]
async fn quiescence_waits_for_the_actual_lease_and_holds_it_through_rename() -> Result<()> {
    // Held for the whole test: every `Server::quiescence` call here either
    // observes contention or actually acquires/releases the real flock, and
    // this test never itself spawns; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::locking_async().await;
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
    // Held for the whole test; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::locking_async().await;
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
    // Held for the whole test: its `supervise` call fails before reaching any
    // spawn (the directory-moved check runs first), and the rest is real
    // flock acquisition/release; see `crate::spawn_gate`.
    let _gate = crate::spawn_gate::locking_async().await;
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
    let binary = {
        // Held across the version-check spawn; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        crate::provision::provision(&config, &crate::store::test_cache()).await?
    };
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
    let (owner, attached) = {
        // Held across both owned-supervisor spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        let owner = Server::open(options.clone()).await?;
        let attached = Server::open(ServerOptions {
            read_only: true,
            ..options
        })
        .await?;
        (owner, attached)
    };
    attached.close().await?;
    let premature = {
        // Held across the actual flock acquisition attempt; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        Server::quiescence(&directory, Duration::from_millis(20)).await
    };
    owner.close().await?;
    assert!(
        premature.is_err(),
        "attached close incorrectly allowed directory activation"
    );
    let lease = {
        // Held across the actual flock acquisition; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        Server::quiescence(&directory, Duration::from_secs(1)).await?
    };
    assert!(!directory.join("endpoint.json").exists());
    {
        // Held across the actual flock release; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        drop(lease);
    }
    Ok(())
}

fn collision_request(root: &Path, binary: PathBuf) -> Result<Request> {
    Ok(Request {
        binary,
        directory: fs::canonicalize(root)?.join("collision-memory"),
        project_scope: "project/selected-port-collision".into(),
        timeout_millis: 20_000,
        read_only: false,
        lifecycle_root: None,
    })
}

async fn actual_dolt_for_collision() -> Result<PathBuf> {
    // Held across the version-check spawn; see `crate::spawn_gate`. A single
    // choke point for every collision test below.
    let _gate = crate::spawn_gate::spawning().await;
    crate::provision::provision(
        &kuru_core::MemoryConfig {
            offline: true,
            ..Default::default()
        },
        &crate::store::test_cache(),
    )
    .await
}

#[tokio::test]
async fn selected_port_takeover_retries_actual_dolt_without_touching_holder() -> Result<()> {
    let root = fixture()?;
    let request = collision_request(root.path(), actual_dolt_for_collision().await?)?;
    let directory = request.directory.clone();
    let holder = Arc::new(StdMutex::new(None::<std::net::TcpListener>));
    let chosen = Arc::new(StdMutex::new(Vec::<u16>::new()));
    let observed_holder = holder.clone();
    let observed_chosen = chosen.clone();
    let (parent, mut input) = tokio::io::duplex(1024);
    let (mut output, mut response) = tokio::io::duplex(4096);
    let supervisor = tokio::spawn(async move {
        // Held across the real Dolt spawn(s); see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        supervise_with_port_hook(request, &mut input, &mut output, move |port| {
            let mut chosen = observed_chosen.lock().unwrap();
            chosen.push(port);
            if chosen.len() == 1 {
                let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
                *observed_holder.lock().unwrap() = Some(listener);
            }
            Ok(())
        })
        .await
    });
    let ready = tokio::time::timeout(
        Duration::from_secs(25),
        read_frame::<_, Response>(&mut response),
    )
    .await??;
    let endpoint = match ready {
        Response::Ready { endpoint, owned } => {
            assert!(owned, "collision recovery borrowed an unrelated lifetime");
            endpoint
        }
        Response::Failed(message) => bail!("actual Dolt did not recover: {message}"),
    };
    let ports = chosen.lock().unwrap().clone();
    assert_eq!(
        ports.len(),
        2,
        "exactly one selected-port collision should retry"
    );
    assert_ne!(ports[0], ports[1]);
    assert_eq!(endpoint.port, ports[1]);
    {
        let held = holder.lock().unwrap();
        assert_eq!(
            held.as_ref()
                .context("unrelated listener lost")?
                .local_addr()?
                .port(),
            ports[0]
        );
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", ports[0])).is_err(),
            "Kuru closed the unrelated port holder"
        );
    }
    let identity = load_identity(&directory, "project/selected-port-collision")?
        .context("ready Dolt did not publish its identity")?;
    let pool = connect_pool_with_timeout(
        &identity,
        &endpoint,
        &directory,
        "main",
        false,
        1,
        PoolAttemptOptions::ordinary(),
    )
    .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kuru_instance")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    pool.close().await;
    drop(parent);
    tokio::time::timeout(Duration::from_secs(10), supervisor).await???;
    assert!(!directory.join("endpoint.json").exists());
    assert!(holder.lock().unwrap().is_some());
    Ok(())
}

#[tokio::test]
async fn persistent_selected_port_takeovers_exhaust_three_owned_attempts() -> Result<()> {
    let root = fixture()?;
    let request = collision_request(root.path(), actual_dolt_for_collision().await?)?;
    let directory = request.directory.clone();
    let holders = Arc::new(StdMutex::new(Vec::<std::net::TcpListener>::new()));
    let retained = holders.clone();
    let (_parent, mut input) = tokio::io::duplex(1024);
    let mut output = Vec::new();
    let error = {
        // Held across the real Dolt spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        supervise_with_port_hook(request, &mut input, &mut output, move |port| {
            retained
                .lock()
                .unwrap()
                .push(std::net::TcpListener::bind(("127.0.0.1", port))?);
            Ok(())
        })
        .await
        .unwrap_err()
    };
    let held = holders.lock().unwrap();
    assert_eq!(
        held.len(),
        3,
        "exact collision retry exceeded its finite attempt bound"
    );
    for listener in held.iter() {
        assert!(std::net::TcpListener::bind(listener.local_addr()?).is_err());
    }
    assert!(
        format!("{error:#}").contains("Dolt exited before readiness"),
        "{error:#}"
    );
    assert!(fs::read_to_string(directory.join("server.log"))?.contains("already in use."));
    assert!(
        output.is_empty(),
        "failed collision published a Ready response"
    );
    assert!(!directory.join("endpoint.json").exists());
    Ok(())
}

#[tokio::test]
async fn unrelated_premature_exit_does_not_retry_or_publish() -> Result<()> {
    let root = fixture()?;
    let binary = root.path().join("controlled-noncollision-exit");
    fs::write(
        &binary,
        b"#!/bin/sh\nprintf 'controlled noncollision exit\\n' >&2\nexit 7\n",
    )?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))?;
    let request = collision_request(root.path(), binary)?;
    let directory = request.directory.clone();
    let mut attempts = 0;
    let (_parent, mut input) = tokio::io::duplex(1024);
    let mut output = Vec::new();
    let error = {
        // Held across the spawn of the controlled noncollision-exit binary;
        // see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        supervise_with_port_hook(request, &mut input, &mut output, |_| {
            attempts += 1;
            Ok(())
        })
        .await
        .unwrap_err()
    };
    assert_eq!(attempts, 1);
    assert!(
        format!("{error:#}").contains("Dolt exited before readiness"),
        "{error:#}"
    );
    assert!(
        fs::read_to_string(directory.join("server.log"))?.contains("controlled noncollision exit")
    );
    assert!(output.is_empty());
    assert!(!directory.join("endpoint.json").exists());
    Ok(())
}

#[tokio::test]
async fn parent_close_during_selected_port_takeover_never_retries() -> Result<()> {
    let root = fixture()?;
    let request = collision_request(root.path(), actual_dolt_for_collision().await?)?;
    let directory = request.directory.clone();
    let holder = Arc::new(StdMutex::new(None::<std::net::TcpListener>));
    let observed = holder.clone();
    let attempts = Arc::new(StdMutex::new(0usize));
    let counted = attempts.clone();
    let (selected, chosen) = tokio::sync::oneshot::channel();
    let mut selected = Some(selected);
    let (parent, mut input) = tokio::io::duplex(1024);
    let mut output = Vec::new();
    let supervisor = tokio::spawn(async move {
        // Held across the real Dolt spawn; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        supervise_with_port_hook(request, &mut input, &mut output, move |port| {
            *counted.lock().unwrap() += 1;
            *observed.lock().unwrap() = Some(std::net::TcpListener::bind(("127.0.0.1", port))?);
            selected
                .take()
                .context("port selected more than once after parent close")?
                .send(())
                .map_err(|_| anyhow!("port-selection observer closed"))?;
            Ok(())
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), chosen).await??;
    drop(parent);
    let error = tokio::time::timeout(Duration::from_secs(10), supervisor)
        .await??
        .unwrap_err();
    assert!(format!("{error:#}").contains("parent closed"), "{error:#}");
    assert_eq!(*attempts.lock().unwrap(), 1);
    assert!(holder.lock().unwrap().is_some());
    assert!(!directory.join("endpoint.json").exists());
    Ok(())
}
