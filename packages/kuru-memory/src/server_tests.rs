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
        template: None,
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
            trace_directory: PathBuf::new(),
            ledger: None,
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
    drain(data.as_slice(), log.clone(), None).await;
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
        ticks: None,
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
        ticks: None,
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
        Response::Failed(message) | Response::TemplateRejected(message) => {
            bail!("actual Dolt did not recover: {message}")
        }
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
    .await?
    .0;
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

/// A real Dolt serving its own directory on a chosen port with a given root
/// password: the shape of a sibling template copy (copies share their
/// credentials) whose Dolt bound this store's freshly selected port first.
struct ForeignDolt {
    child: Child,
    port: u16,
    password: String,
    log: Arc<Mutex<Vec<u8>>>,
    drains: Vec<tokio::task::JoinHandle<()>>,
}

impl ForeignDolt {
    async fn start(binary: &Path, directory: &Path, port: u16, password: String) -> Result<Self> {
        for name in ["", "data", "config"] {
            private_directory(&directory.join(name))?;
        }
        crate::provision::prepare_private_home(&directory.join("home"))?;
        let config = directory.join("server.yaml");
        write_private(
            &config,
            server_yaml(directory, port, Duration::from_secs(20))?.as_bytes(),
        )?;
        let mut child = crate::engine::spawn(
            binary,
            &directory.join("home"),
            directory,
            vec!["sql-server".into(), "--config".into(), config.into()],
            vec![
                ("DOLT_ROOT_PASSWORD".into(), password.clone().into()),
                ("DOLT_ROOT_HOST".into(), "localhost".into()),
            ],
            true,
        )
        .await?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let drains = vec![
            tokio::spawn(drain(
                child.stdout().context("foreign Dolt stdout missing")?,
                log.clone(),
                None,
            )),
            tokio::spawn(drain(
                child.stderr().context("foreign Dolt stderr missing")?,
                log.clone(),
                None,
            )),
        ];
        let mut foreign = Self {
            child,
            port,
            password,
            log,
            drains,
        };
        // Bounded: serving its own directory to the shared credential, or a
        // failure naming the foreign server's own output.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = foreign.child.try_wait()? {
                bail!(
                    "foreign Dolt exited ({status}) before serving port {port}: {}",
                    foreign.log_text().await
                );
            }
            if let Ok(Ok(pool)) = timeout(Duration::from_millis(400), foreign.connect()).await {
                let datadir: String = sqlx::query_scalar("SELECT @@datadir")
                    .fetch_one(&pool)
                    .await?;
                pool.close().await;
                ensure!(
                    same_directory(Path::new(&datadir), &directory.join("data"))?,
                    "the server on port {port} is not the foreign Dolt"
                );
                return Ok(foreign);
            }
            if Instant::now() >= deadline {
                bail!(
                    "foreign Dolt did not serve port {port} within 30 s: {}",
                    foreign.log_text().await
                );
            }
            sleep(Duration::from_millis(25)).await;
        }
    }

    async fn connect(&self) -> sqlx::Result<MySqlPool> {
        MySqlPoolOptions::new()
            .max_connections(1)
            .connect_with(
                MySqlConnectOptions::new()
                    .host("127.0.0.1")
                    .port(self.port)
                    .username("root")
                    .password(&self.password)
                    .ssl_mode(MySqlSslMode::Disabled),
            )
            .await
    }

    async fn log_text(&self) -> String {
        String::from_utf8_lossy(&self.log.lock().await).replace(&self.password, "[redacted]")
    }

    /// Still serving, and holding nothing a store bootstrap would write.
    async fn assert_untouched(&self) -> Result<()> {
        let pool = timeout(Duration::from_secs(5), self.connect())
            .await
            .context("foreign Dolt stopped answering")??;
        let databases: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name = 'kuru'",
        )
        .fetch_one(&pool)
        .await?;
        let readers: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM mysql.user WHERE user = 'kuru_reader'")
                .fetch_one(&pool)
                .await?;
        pool.close().await;
        ensure!(
            databases == 0 && readers == 0,
            "the supervisor wrote to the foreign Dolt ({databases} kuru databases, \
             {readers} reader accounts)"
        );
        Ok(())
    }

    async fn stop(mut self) -> Result<()> {
        self.child.stop(CLOSE_GRACE, KILL_GRACE).await?;
        for drain in self.drains {
            timeout(Duration::from_secs(5), drain).await??;
        }
        Ok(())
    }
}

/// The actual Dolt behind a one-second delay: the owned engine is reliably
/// still starting when the startup probe first reaches the selected port.
fn delayed_dolt(root: &Path, dolt: &Path) -> Result<PathBuf> {
    let dolt = dolt.to_str().context("Dolt path is not UTF-8")?;
    ensure!(!dolt.contains('\''), "Dolt path needs quoting");
    let wrapper = root.join("delayed-dolt");
    fs::write(
        &wrapper,
        format!("#!/bin/sh\nsleep 1\nexec '{dolt}' \"$@\"\n"),
    )?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    Ok(wrapper)
}

/// Start a foreign Dolt on `port` under this store's own root credential,
/// from the synchronous port hook, before the owned engine is spawned.
fn foreign_on_selected_port(
    dolt: &Path,
    store: &Path,
    foreign: &Path,
    port: u16,
) -> Result<ForeignDolt> {
    let password = load_identity(store, "project/selected-port-collision")?
        .context("the supervisor wrote no identity before selecting a port")?
        .password;
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(ForeignDolt::start(dolt, foreign, port, password))
    })
}

/// A sibling's Dolt that took the selected port and accepts this store's
/// credential answers the startup probe with its own data directory. That is
/// a taken port, not a failed bootstrap: nothing is written to it, the owned
/// Dolt reports the taken port itself, and the bounded retry reselects.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_foreign_dolt_on_selected_port_retries_without_touching_it() -> Result<()> {
    let root = fixture()?;
    let dolt = actual_dolt_for_collision().await?;
    let mut request = collision_request(root.path(), delayed_dolt(root.path(), &dolt)?)?;
    request.timeout_millis = 60_000;
    let directory = request.directory.clone();
    let foreign_directory = fs::canonicalize(root.path())?.join("foreign-memory");
    let foreign = Arc::new(StdMutex::new(None::<ForeignDolt>));
    let chosen = Arc::new(StdMutex::new(Vec::<u16>::new()));
    let (observed_foreign, observed_chosen) = (foreign.clone(), chosen.clone());
    let hook_directory = directory.clone();
    let (parent, mut input) = tokio::io::duplex(1024);
    let (mut output, mut response) = tokio::io::duplex(4096);
    let supervisor = tokio::spawn(async move {
        // Held across the real Dolt spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        supervise_with_port_hook(request, &mut input, &mut output, move |port| {
            let first = {
                let mut chosen = observed_chosen.lock().unwrap();
                chosen.push(port);
                chosen.len() == 1
            };
            if first {
                let started =
                    foreign_on_selected_port(&dolt, &hook_directory, &foreign_directory, port)?;
                *observed_foreign.lock().unwrap() = Some(started);
            }
            Ok(())
        })
        .await
    });
    let ready = tokio::time::timeout(
        Duration::from_secs(90),
        read_frame::<_, Response>(&mut response),
    )
    .await?;
    let endpoint = match ready {
        Ok(Response::Ready { endpoint, owned }) => {
            assert!(
                owned,
                "foreign-listener recovery borrowed an unrelated lifetime"
            );
            endpoint
        }
        Ok(Response::Failed(message) | Response::TemplateRejected(message)) => {
            bail!("actual Dolt did not recover: {message}")
        }
        Err(error) => {
            let outcome = tokio::time::timeout(Duration::from_secs(30), supervisor).await??;
            let foreign = foreign.lock().unwrap().take();
            if let Some(foreign) = foreign {
                foreign.stop().await?;
            }
            bail!("supervisor ended before Ready ({error:#}): {outcome:?}")
        }
    };
    let ports = chosen.lock().unwrap().clone();
    assert_eq!(
        ports.len(),
        2,
        "exactly one foreign-listener retry: {ports:?}"
    );
    assert_ne!(ports[0], ports[1]);
    assert_eq!(endpoint.port, ports[1]);
    let identity = load_identity(&directory, "project/selected-port-collision")?
        .context("ready Dolt did not publish its identity")?;
    assert!(identity.initialized);
    let pool = connect_pool_with_timeout(
        &identity,
        &endpoint,
        &directory,
        "main",
        false,
        1,
        PoolAttemptOptions::ordinary(),
    )
    .await?
    .0;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kuru_instance")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    pool.close().await;
    drop(parent);
    tokio::time::timeout(Duration::from_secs(30), supervisor).await???;
    assert!(!directory.join("endpoint.json").exists());
    let foreign = foreign
        .lock()
        .unwrap()
        .take()
        .context("the foreign Dolt was never started")?;
    assert_eq!(foreign.port, ports[0]);
    foreign.assert_untouched().await?;
    foreign.stop().await
}

/// While the answering server stays foreign and the owned engine neither
/// becomes ready nor exits, the start fails closed at its deadline with the
/// mismatch in the error, publishing no Ready response or endpoint.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_foreign_dolt_fails_closed_with_the_mismatch() -> Result<()> {
    let root = fixture()?;
    let dolt = actual_dolt_for_collision().await?;
    // Never binds and never exits on its own: only the foreign server answers.
    let silent = root.path().join("silent-engine");
    fs::write(&silent, b"#!/bin/sh\nexec sleep 600\n")?;
    fs::set_permissions(&silent, fs::Permissions::from_mode(0o700))?;
    let mut request = collision_request(root.path(), silent)?;
    // The hook's foreign start runs inside this deadline.
    request.timeout_millis = 15_000;
    let directory = request.directory.clone();
    let foreign_directory = fs::canonicalize(root.path())?.join("foreign-memory");
    let foreign = Arc::new(StdMutex::new(None::<ForeignDolt>));
    let observed = foreign.clone();
    let hook_directory = directory.clone();
    let mut attempts = 0;
    let (_parent, mut input) = tokio::io::duplex(1024);
    let mut output = Vec::new();
    let result = {
        // Held across the real Dolt and stand-in spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::spawning().await;
        supervise_with_port_hook(request, &mut input, &mut output, |port| {
            attempts += 1;
            let started =
                foreign_on_selected_port(&dolt, &hook_directory, &foreign_directory, port)?;
            *observed.lock().unwrap() = Some(started);
            Ok(())
        })
        .await
    };
    let foreign = foreign
        .lock()
        .unwrap()
        .take()
        .context("the foreign Dolt was never started")?;
    let error = match result {
        Ok(()) => bail!("the supervisor became ready on a foreign server"),
        Err(error) => format!("{error:#}"),
    };
    assert_eq!(attempts, 1);
    assert!(
        error.contains("authenticated Dolt startup deadline exceeded")
            && error.contains("Dolt bootstrap data directory mismatch"),
        "{error}"
    );
    assert!(
        output.is_empty(),
        "a foreign server's start published Ready"
    );
    assert!(!directory.join("endpoint.json").exists());
    let identity = load_identity(&directory, "project/selected-port-collision")?
        .context("the supervisor's identity is missing")?;
    assert!(
        !identity.initialized,
        "a foreign server initialized this store"
    );
    foreign.assert_untouched().await?;
    foreign.stop().await
}

// A failure reason recorded for a starter never carries either connection
// secret of the store's identity.
#[test]
fn a_failure_reason_never_carries_a_connection_secret() {
    let identity = Identity {
        version: 1,
        instance: Uuid::new_v4().to_string(),
        project_scope: "project/redaction".into(),
        password: secret(),
        reader_password: secret(),
        initialized: true,
        template: None,
    };
    let text = format!(
        "pool refused {} and {} twice: {}",
        identity.password, identity.reader_password, identity.password
    );
    assert_eq!(
        redact_identity(text, &identity),
        "pool refused [redacted] and [redacted] twice: [redacted]"
    );
}

/// A supervisor that takes its startup request and never answers reaches the
/// one readiness deadline in its Ready-frame step: the outer cause is
/// unchanged, the step is named, and the supervisor was reaped before the
/// error returned. Real clock: the wait is the product's own deadline, the
/// smallest startup timeout plus the supervisor transport allowance, because
/// the reap after it polls on the clock that a paused test would race.
#[tokio::test]
async fn supervisor_readiness_deadline_names_its_part() -> Result<()> {
    let root = fixture()?;
    let script = root.path().join("silent-supervisor");
    fs::write(&script, b"#!/bin/sh\ncat >/dev/null\n")?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let directory = root.path().join("store");
    let options = ServerOptions {
        binary: "/unexecuted".into(),
        directory: directory.clone(),
        project_scope: "project/silent".into(),
        supervisor: script,
        timeout: Duration::from_millis(1),
        read_only: false,
        retained: None,
        lifecycle_root: None,
        ticks: None,
    };
    // Holds the spawn gate across the supervisor spawn only.
    let error = Server::open_with_initial_probe_delay(
        options,
        Duration::ZERO,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .err()
    .context("a silent supervisor was reported ready")?;
    let rendered = format!("{error:#}");
    assert!(
        error
            .chain()
            .any(|cause| cause.to_string() == "memory supervisor readiness deadline exceeded"),
        "the outer cause changed: {rendered}"
    );
    assert!(
        rendered.starts_with(
            "memory supervisor readiness deadline exceeded: the supervisor's Ready frame had not completed "
        ) && rendered.contains(" ms after the supervisor was spawned: deadline has elapsed"),
        "{rendered}"
    );
    assert!(
        error
            .chain()
            .any(|cause| cause.is::<tokio::time::error::Elapsed>()),
        "{rendered}"
    );
    // The reap after the deadline succeeded: no cleanup failure was added.
    assert!(
        !rendered.contains("memory startup cleanup also failed"),
        "{rendered}"
    );
    assert!(!directory.join("endpoint.json").exists());
    Ok(())
}
