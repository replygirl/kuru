use super::*;

#[tokio::test]
async fn lifecycle_namespace_is_explicit_external_and_bound_to_full_native_identity() -> Result<()>
{
    let root = crate::test_support::tempdir()?;
    let stage = root.path().join("stage 東京");
    private_directory(&stage)?;
    assert!(
        Server::quiescence_at(&stage, None, Duration::from_secs(1))
            .await
            .is_err()
    );
    assert!(
        Server::quiescence_at(&stage, Some(&stage), Duration::from_secs(1))
            .await
            .is_err()
    );
    assert!(
        Server::quiescence_at(&stage, Some(&stage.join("leases")), Duration::from_secs(1))
            .await
            .is_err()
    );
    // The deliberately rejected namespace is private and empty, never a lock.
    std::fs::remove_dir(stage.join("leases"))?;
    let namespace = root.path().join("lifecycles");
    let mut owner = Server::quiescence_at(&stage, Some(&namespace), Duration::from_secs(1)).await?;
    let identity = owner.directory.identity();
    let lock_identity = kuru_platform::fs::regular_file_info(&owner.lock)?.identity;
    assert!(!stage.join("lifecycle.lock").exists());
    assert!(
        Server::quiescence_at(&stage, Some(&namespace), Duration::from_millis(10))
            .await
            .is_err()
    );
    let active = root.path().join("active");
    owner.move_to(&active)?;
    assert_eq!(owner.directory.identity(), identity);
    assert_eq!(
        kuru_platform::fs::regular_file_info(&owner.lock)?.identity,
        lock_identity
    );
    assert!(
        Server::quiescence_at(&active, Some(&namespace), Duration::from_millis(10))
            .await
            .is_err()
    );
    // Recreated source means a new physical store, not the moved store's lease.
    private_directory(&stage)?;
    let recreated = Server::quiescence_at(&stage, Some(&namespace), Duration::from_secs(1)).await?;
    assert_ne!(recreated.directory.identity(), identity);
    drop(recreated);
    let interrupted_parent = root.path().join("interrupted café 東京");
    private_directory(&interrupted_parent)?;
    let interrupted = interrupted_parent.join("preserved-stage");
    owner.move_to(&interrupted)?;
    assert!(!active.exists());
    assert_eq!(owner.directory.identity(), identity);
    assert_eq!(
        kuru_platform::fs::regular_file_info(&owner.lock)?.identity,
        lock_identity
    );
    assert!(
        Server::quiescence_at(&interrupted, Some(&namespace), Duration::from_millis(10))
            .await
            .is_err()
    );
    drop(owner);
    let recovered =
        Server::quiescence_at(&interrupted, Some(&namespace), Duration::from_secs(1)).await?;
    assert_eq!(recovered.directory.identity(), identity);
    assert_eq!(
        kuru_platform::fs::regular_file_info(&recovered.lock)?.identity,
        lock_identity
    );
    Ok(())
}

#[tokio::test]
async fn stopped_stage_closes_descendant_file_handles_before_moving_under_lease() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let stage = root.path().join("stage");
    let namespace = root.path().join("leases");
    private_directory(&stage)?;
    let mut lease = Server::quiescence_at(&stage, Some(&namespace), Duration::from_secs(1)).await?;
    write_private(&stage.join("ready.json"), b"retained candidate")?;
    let held = lease.directory.read(std::ffi::OsStr::new("ready.json"))?;
    let active = root.path().join("active");
    assert!(lease.move_to(&active).is_err());
    assert!(!active.exists());
    assert_eq!(
        std::fs::read(stage.join("ready.json"))?,
        b"retained candidate"
    );
    drop(held);
    lease.move_to(&active)?;
    assert_eq!(
        std::fs::read(active.join("ready.json"))?,
        b"retained candidate"
    );
    Ok(())
}

#[tokio::test]
async fn bounded_frames_and_tail_logs_preserve_shared_protocol_on_windows() -> Result<()> {
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
    assert!(
        write_frame(&mut tokio::io::sink(), &"x".repeat(RECORD_LIMIT))
            .await
            .is_err()
    );
    let mut bytes = vec![b'a'; LOG_LIMIT];
    bytes.extend(vec![b'b'; LOG_LIMIT]);
    let log = Arc::new(Mutex::new(Vec::new()));
    drain(bytes.as_slice(), log.clone()).await;
    assert_eq!(*log.lock().await, vec![b'b'; LOG_LIMIT]);
    Ok(())
}
