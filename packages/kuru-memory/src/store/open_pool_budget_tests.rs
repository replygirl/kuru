use super::*;

fn pool_timed_out(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        )
    })
}

/// The migrated staged reopen is the second Dolt start of a fresh open. Its
/// main pool continues that start's deadline; a post-open pool is ordinary.
#[tokio::test]
async fn migrated_stage_pool_uses_remaining_startup_budget_and_post_open_pools_stay_ordinary()
-> Result<()> {
    // Past the ordinary per-attempt window, well inside the startup budget.
    let beyond_ordinary = crate::server::ORDINARY_POOL_WINDOW * 3 / 2;

    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(
        root.path().join("within-budget"),
        format!("project/{}", "1".repeat(64)),
    )?;
    let entered = Arc::new(AtomicBool::new(false));
    options.migrated_stage_pool_delay = Some((beyond_ordinary, entered.clone()));
    let store = crate::test_support::spawn_gated_open(options).await?;
    assert!(
        entered.load(Ordering::SeqCst),
        "migrated staged main-pool authentication was not delayed"
    );

    // Once ready, a fresh branch pool gets exactly the ordinary window.
    let branch = "open_budget_probe";
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?)")
            .bind(branch)
            .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("probe branch creation deadline exceeded")??;
    let entered = Arc::new(AtomicBool::new(false));
    store
        .shared
        .server
        .delay_next_pool_authentication(beyond_ordinary, entered.clone());
    let error = store
        .shared
        .server
        .pool(branch)
        .await
        .expect_err("a post-open pool must keep the ordinary window");
    assert!(
        entered.load(Ordering::SeqCst),
        "post-open pool authentication was not delayed"
    );
    assert!(
        pool_timed_out(&error),
        "post-open pool did not fail its ordinary acquisition window: {error:#}"
    );
    store.close().await?;

    // Past the whole startup budget, the same pool fails with its own bound.
    let mut options = crate::test_support::open_options(
        root.path().join("past-budget"),
        format!("project/{}", "2".repeat(64)),
    )?;
    let startup_budget = crate::test_support::server_start_budget();
    let entered = Arc::new(AtomicBool::new(false));
    options.migrated_stage_pool_delay = Some((startup_budget, entered.clone()));
    let error = crate::test_support::spawn_gated_open(options)
        .await
        .expect_err("an expired open-sequence pool cannot open memory");
    assert!(
        entered.load(Ordering::SeqCst),
        "migrated staged main-pool authentication was not delayed"
    );
    assert!(
        pool_timed_out(&error),
        "expired open-sequence pool did not fail its acquisition window: {error:#}"
    );
    Ok(())
}

/// SQLx keeps a pool's acquire timeout for its whole life. Pools created while
/// the store was opening must still be ordinary once it reports ready.
#[tokio::test]
async fn retained_open_pools_keep_the_ordinary_acquire_window() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        root.path().join("retained"),
        format!("project/{}", "3".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options).await?;
    let usage = store
        .shared
        .usage_pool
        .lock()
        .expect("usage pool lock")
        .clone()
        .context("writable store did not retain its usage-ledger pool")?;
    for (name, pool) in [("main", &store.pool), ("usage ledger", &usage)] {
        assert_eq!(
            pool.options().get_acquire_timeout(),
            crate::server::ORDINARY_POOL_WINDOW,
            "retained {name} pool kept an opening-phase acquire window"
        );
    }

    // Contend the retained main pool: with every connection held, the next
    // acquire must end at the ordinary window, not a startup-length one.
    let mut held = Vec::new();
    for _ in 0..store.pool.options().get_max_connections() {
        held.push(store.pool.acquire().await?);
    }
    let contended = tokio::time::timeout(
        crate::server::ORDINARY_POOL_WINDOW * 2,
        store.pool.acquire(),
    )
    .await;
    assert!(
        matches!(contended, Ok(Err(sqlx::Error::PoolTimedOut))),
        "contended retained pool did not fail at its ordinary acquire window"
    );
    drop(held);
    drop(usage);
    store.close().await?;
    Ok(())
}
