use super::*;
use crate::server::ConnectionObservation;
use std::pin::Pin;

fn pool_timed_out(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolTimedOut)
        )
    })
}

/// Work a test keeps pending across several waits and polls again later.
type Work<'a> = Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>>;

/// Named work that must stay pending while a test waits for an event.
type Watched<'w> = (
    &'w str,
    &'w mut (dyn std::future::Future<Output = Result<()>> + Unpin + 'w),
);

fn ended(outcome: &Result<()>) -> String {
    match outcome {
        Ok(()) => "Ok".into(),
        Err(error) => format!("Err({error:#})"),
    }
}

/// Complete once the held pool's acquisitions have left `records`
/// slow-acquire records, failing at once if any of `pending` ends first: a
/// shorter bound fails the test when it fires instead of parking it on the
/// harness timeout.
async fn until_slow_records(
    observation: &ConnectionObservation,
    records: u64,
    pending: &mut [Watched<'_>],
) -> Result<()> {
    let first_ended = futures::future::select_all(
        pending
            .iter_mut()
            .map(|(name, work)| Box::pin(async move { (*name, work.await) })),
    );
    tokio::select! {
        biased;
        ((name, outcome), _, _) = first_ended => bail!(
            "{name} ended before slow-acquire record {records} of the held pool \
             ({} recorded): {}",
            observation.slow_acquire_records(),
            ended(&outcome)
        ),
        () = observation.slow_acquire_records_reach(records) => Ok(()),
    }
}

/// With every permit of `pool` held: A (a statement) waits past its
/// slow-acquire record, then B (an acquisition) starts and waits past its own,
/// at least the slow-acquire threshold after A began. Every wait is an event
/// raced against A, B and `others`, which must all stay pending.
async fn two_slow_records<'p>(
    pool: &'p MemoryPool,
    others: &mut [Watched<'_>],
) -> Result<(Work<'p>, Work<'p>)> {
    let observation = pool.observation().clone();
    let base = observation.slow_acquire_records();
    let mut a: Work<'p> = Box::pin(async move {
        let one: i64 = crate::pool::within(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT 1").fetch_one(pool),
        )
        .await??;
        ensure!(one == 1, "A read {one}");
        Ok(())
    });
    {
        let mut pending: Vec<Watched<'_>> = Vec::new();
        for (name, work) in others.iter_mut() {
            pending.push((*name, &mut **work));
        }
        pending.push(("A, a contended statement,", &mut a));
        until_slow_records(&observation, base + 1, &mut pending).await?;
    }
    let mut b: Work<'p> = Box::pin(async move {
        crate::pool::within(QUERY_TIMEOUT, pool.acquire())
            .await??
            .release()
            .await;
        Ok(())
    });
    {
        let mut pending: Vec<Watched<'_>> = Vec::new();
        for (name, work) in others.iter_mut() {
            pending.push((*name, &mut **work));
        }
        pending.push(("A, a contended statement,", &mut a));
        pending.push(("B, a contended acquisition,", &mut b));
        until_slow_records(&observation, base + 2, &mut pending).await?;
    }
    Ok((a, b))
}

/// One released session goes to A, first in SQLx's fair queue; B, dropped
/// while still queued, leaves no acquisition pending.
async fn release_goes_to_the_first_waiter(
    pool: &MemoryPool,
    held: &mut Vec<crate::pool::PooledSession>,
    mut a: Work<'_>,
    b: Work<'_>,
) -> Result<()> {
    let mut b = b;
    held.pop()
        .context("no held session to release")?
        .release()
        .await;
    tokio::select! {
        biased;
        outcome = &mut a => outcome.context("A failed once a session was released")?,
        outcome = &mut b => bail!(
            "B received the released session before A, which queued first: {}",
            ended(&outcome)
        ),
    }
    drop(b);
    ensure!(
        pool.pending_acquires() == 0,
        "{} acquisitions still pending after the queued one was dropped",
        pool.pending_acquires()
    );
    Ok(())
}

async fn hold_every_session(pool: &MemoryPool) -> Result<Vec<crate::pool::PooledSession>> {
    let mut held = Vec::new();
    for _ in 0..pool.options().get_max_connections() {
        held.push(pool.acquire().await?);
    }
    Ok(held)
}

/// Once memory is open, a contended acquisition is bounded by the budget of
/// the statement it serves, not by a shorter window: with every permit held,
/// a statement and then an acquisition each outlive their slow-acquire record
/// (so the first waits at least twice the threshold), the first released
/// session goes to the statement, and the dropped acquisition leaves nothing
/// pending. Every wait is an event; the test adds no timer.
#[tokio::test]
async fn contended_acquire_outlives_the_slow_threshold_within_its_statement_budget() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let mut held = hold_every_session(&pool).await?;
    let outcome = async {
        let (a, b) = two_slow_records(&pool, &mut []).await?;
        release_goes_to_the_first_waiter(&pool, &mut held, a, b).await
    }
    .await;
    for session in held {
        session.release().await;
    }
    drop(pool);
    store.close().await?;
    outcome
}

/// The cold stage's one engine start is the first Dolt start of a fresh
/// open. Its main pool continues that start's deadline. A post-open pool's
/// creation is bounded by its creation budget, not a shorter window.
#[tokio::test]
async fn stage_pool_uses_remaining_startup_budget_and_post_open_pools_are_statement_bounded()
-> Result<()> {
    // Past the opening floor, well inside the startup budget.
    let beyond_ordinary = crate::server::ORDINARY_POOL_WINDOW * 3 / 2;

    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::warmed_open_options(
        root.path().join("within-budget"),
        format!("project/{}", "1".repeat(64)),
    )
    .await?;
    let entered = Arc::new(AtomicBool::new(false));
    // The delayed pool is the cold staged build's one stage start.
    options.creation = Creation::Cold;
    options.stage_pool_delay = Some((beyond_ordinary, entered.clone()));
    let store = crate::test_support::spawn_gated_open(options).await?;
    assert!(
        entered.load(Ordering::SeqCst),
        "staged main-pool authentication was not delayed"
    );

    // Once ready, a fresh branch pool's creation held at its new connection's
    // authentication stays pending while the held main pool leaves two
    // slow-acquire records, at least twice the threshold; released, it opens
    // with a statement-budget ceiling. The main pool is read through its own
    // handle: the server's pool map stays locked while the creation runs.
    let branch = "open_budget_probe";
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?)")
            .bind(branch)
            .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("probe branch creation deadline exceeded")??;
    let main = store.pool.clone();
    let server = &store.shared.server;
    let mut held = hold_every_session(&main).await?;
    let authentication = server.gate_next_pool_authentication();
    let outcome = async {
        let mut creation: Work = Box::pin(async {
            let pool = server.pool(branch).await?;
            ensure!(
                pool.options().get_acquire_timeout() == QUERY_TIMEOUT,
                "a post-open pool's acquisition ceiling is {:?}, not the statement budget",
                pool.options().get_acquire_timeout()
            );
            Ok(())
        });
        let gate = tokio::select! {
            biased;
            outcome = &mut creation => bail!(
                "post-open pool creation ended before arming its authentication gate: {}",
                ended(&outcome)
            ),
            gate = authentication => gate.context("the post-open pool attempt dropped its gate")?,
        };
        tokio::select! {
            biased;
            outcome = &mut creation => bail!(
                "post-open pool creation ended before a connection reached its gate: {}",
                ended(&outcome)
            ),
            () = gate.entered() => {}
        }
        let (a, b) =
            two_slow_records(&main, &mut [("post-open pool creation", &mut creation)]).await?;
        drop(gate);
        creation
            .await
            .context("post-open pool creation failed once its gate was released")?;
        release_goes_to_the_first_waiter(&main, &mut held, a, b).await
    }
    .await;
    for session in held {
        session.release().await;
    }
    drop(main);
    store.close().await?;
    outcome?;

    // Past the whole startup budget, the same pool fails with its own bound.
    let mut options = crate::test_support::warmed_open_options(
        root.path().join("past-budget"),
        format!("project/{}", "2".repeat(64)),
    )
    .await?;
    let startup_budget = crate::test_support::server_start_budget();
    let entered = Arc::new(AtomicBool::new(false));
    // The delayed pool is the cold staged build's one stage start.
    options.creation = Creation::Cold;
    options.stage_pool_delay = Some((startup_budget, entered.clone()));
    let error = crate::test_support::spawn_gated_open(options)
        .await
        .expect_err("an expired open-sequence pool cannot open memory");
    assert!(
        entered.load(Ordering::SeqCst),
        "staged main-pool authentication was not delayed"
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
    let options = crate::test_support::warmed_open_options(
        root.path().join("retained"),
        format!("project/{}", "3".repeat(64)),
    )
    .await?;
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
    // acquire must end at the ordinary window, not a startup-length one, and
    // name a wait for held connections. SQLx's own window is the bound under
    // test; the test adds no timer.
    let max = store.pool.options().get_max_connections();
    let mut held = Vec::new();
    for _ in 0..max {
        held.push(store.pool.acquire().await?);
    }
    let error = store
        .pool
        .acquire()
        .await
        .err()
        .context("a contended retained pool handed out a fifth session")?;
    assert!(
        pool_timed_out(&error),
        "contended retained pool lost SQLx's pool timeout: {error:#}"
    );
    let diagnostic = crate::pool::pool_acquire_timeout(&error)
        .with_context(|| format!("contended acquire carried no typed diagnostic: {error:#}"))?;
    assert_eq!(diagnostic.wait, crate::pool::PoolWait::HeldConnections);
    assert_eq!(diagnostic.branch, "main");
    assert_eq!((diagnostic.max, diagnostic.size), (max, max));
    assert_eq!((diagnostic.idle, diagnostic.checked_out), (0, max));
    assert_eq!(diagnostic.window, crate::server::ORDINARY_POOL_WINDOW);
    assert!(diagnostic.waited >= diagnostic.window, "{diagnostic}");
    assert_eq!(diagnostic.authenticated_during_wait, 0);
    assert_eq!(diagnostic.phase, None);
    let text = format!("{error:#}");
    assert!(
        text.contains("memory pool acquire on kuru/main timed out")
            && text.contains("every permit held")
            && text.contains(&format!(
                "pool size {max} of {max}, 0 idle, {max} checked out"
            )),
        "contended acquire diagnostic is incomplete: {text}"
    );
    drop(held);
    drop(usage);
    store.close().await?;
    Ok(())
}
