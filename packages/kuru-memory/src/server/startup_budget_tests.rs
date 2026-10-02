use super::*;

#[tokio::test]
async fn initial_authentication_uses_remaining_startup_budget_and_reaps_on_expiry() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let config = kuru_core::MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let binary = {
        let _gate = crate::spawn_gate::spawning().await;
        crate::provision::provision(&config, &crate::store::test_cache()).await?
    };
    let supervisor = crate::store::test_supervisor()?;
    let lifecycle_root = cfg!(windows).then(|| root.path().join("leases"));
    let options = |directory: PathBuf, timeout: Duration| ServerOptions {
        binary: binary.clone(),
        directory,
        project_scope: "project/startup-budget".into(),
        supervisor: supervisor.clone(),
        timeout,
        read_only: false,
        retained: None,
        lifecycle_root: lifecycle_root.clone(),
    };

    // This delay runs inside SQLx acquisition after a real Dolt connection.
    // The old fixed two-second acquisition fails; one remaining startup
    // deadline permits this bounded authenticated probe and identity check.
    let success_dir = root.path().join("within-budget");
    let entered = Arc::new(AtomicBool::new(false));
    let success = Server::open_with_initial_probe_delay(
        options(success_dir, Duration::from_secs(30)),
        Duration::from_millis(2300),
        entered.clone(),
    )
    .await?;
    success.close().await?;
    assert!(
        entered.load(Ordering::SeqCst),
        "initial callback was not entered"
    );

    let expired_dir = root.path().join("past-deadline");
    let entered = Arc::new(AtomicBool::new(false));
    let error = Server::open_with_initial_probe_delay(
        options(expired_dir.clone(), Duration::from_secs(30)),
        Duration::from_secs(40),
        entered.clone(),
    )
    .await
    .expect_err("an expired initial authentication cannot return a server");
    assert!(
        entered.load(Ordering::SeqCst),
        "initial callback was not entered"
    );
    let acquire_deadline = error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        )
    });
    assert!(
        format!("{error:#}").contains("deadline exceeded") || acquire_deadline,
        "expired probe did not report the bounded startup failure"
    );
    assert!(
        !expired_dir.join("endpoint.json").exists(),
        "failed startup retained a published endpoint"
    );
    // The supervisor owns the lifecycle lock until Dolt is reaped. Acquiring
    // this exact directory lease proves that ownership has ended before retry.
    let lease = Server::quiescence_at(
        &expired_dir,
        lifecycle_root.as_deref(),
        Duration::from_secs(3),
    )
    .await?;
    drop(lease);
    let reopened = {
        let _gate = crate::spawn_gate::spawning().await;
        Server::open(options(expired_dir, Duration::from_secs(20))).await?
    };
    reopened.close().await?;
    Ok(())
}

#[tokio::test]
async fn opening_pool_identity_rejection_is_terminal() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let config = kuru_core::MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let binary = {
        let _gate = crate::spawn_gate::spawning().await;
        crate::provision::provision(&config, &crate::store::test_cache()).await?
    };
    let server = {
        let _gate = crate::spawn_gate::spawning().await;
        Server::open(ServerOptions {
            binary,
            directory: root.path().join("identity"),
            project_scope: "project/opening-identity".into(),
            supervisor: crate::store::test_supervisor()?,
            timeout: Duration::from_secs(config.startup_timeout_secs),
            read_only: false,
            retained: None,
            lifecycle_root: cfg!(windows).then(|| root.path().join("leases")),
        })
        .await?
    };
    assert!(
        server.opening_deadline().is_some(),
        "an owned start did not enter its opening phase"
    );

    // The SQL identity row no longer matches this server's private identity,
    // which a retry against the same endpoint cannot change.
    let main = server.pool("main").await?;
    timeout(
        crate::store::QUERY_TIMEOUT,
        sqlx::query("UPDATE kuru_instance SET instance_id = ? WHERE singleton = 1")
            .bind(Uuid::new_v4().to_string())
            .execute(main.as_ref()),
    )
    .await
    .context("identity row update deadline exceeded")??;
    main.close().await;
    drop(main);

    let error = server
        .pool("main")
        .await
        .expect_err("a mismatched SQL identity cannot authenticate a pool");
    let rejection = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<sqlx::Error>());
    assert!(
        matches!(rejection, Some(sqlx::Error::Protocol(_))),
        "opening identity rejection was waited out or misclassified: {error:#}"
    );
    server.close().await?;
    Ok(())
}

/// A Windows accept that fails with its own timeout at or after the shared
/// readiness deadline is the accept part of that deadline; before it, or
/// with another kind, it stays an ordinary accept failure.
#[test]
fn an_accept_timeout_at_the_deadline_is_the_accept_part() {
    let deadline = Instant::now() + Duration::from_secs(1);
    let timed_out = std::io::Error::new(std::io::ErrorKind::TimedOut, "accept timed out");
    for now in [deadline, deadline + Duration::from_millis(1)] {
        assert_eq!(
            readiness_deadline_part(&timed_out, now, deadline),
            Some(ReadinessPart::Accept)
        );
    }
    let early = deadline - Duration::from_millis(1);
    assert_eq!(readiness_deadline_part(&timed_out, early, deadline), None);
    for kind in [
        std::io::ErrorKind::BrokenPipe,
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::Other,
    ] {
        let error = std::io::Error::new(kind, "accept failed");
        assert_eq!(readiness_deadline_part(&error, deadline, deadline), None);
    }
    // Printed in the deadline's shape, under its unchanged outer cause.
    let error = readiness_deadline(timed_out.into(), ReadinessPart::Accept, deadline);
    let rendered = format!("{error:#}");
    assert!(
        rendered.starts_with(
            "memory supervisor readiness deadline exceeded: the supervisor's private channel accept had not completed "
        ),
        "{rendered}"
    );
    assert_eq!(
        error.to_string(),
        "memory supervisor readiness deadline exceeded"
    );
    for (part, text) in [
        (
            ReadinessPart::Accept,
            "the supervisor's private channel accept",
        ),
        (ReadinessPart::Write, "the startup request write"),
        (ReadinessPart::Read, "the supervisor's Ready frame"),
    ] {
        assert_eq!(part.to_string(), text);
        let step = ReadinessStep::new(ReadinessPart::Accept);
        step.set(part);
        assert_eq!(step.get(), part);
    }
}
