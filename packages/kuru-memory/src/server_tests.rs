use super::*;
use std::os::unix::fs::symlink;
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
    let yaml = server_yaml(&root.path().join("space café \"quoted\""), 55000)?;
    assert!(yaml.contains("host: 127.0.0.1"));
    assert!(yaml.contains("event_scheduler: \"OFF\""));
    assert!(yaml.contains("\\\"quoted\\\""));
    assert!(!yaml.contains("password:"));
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
