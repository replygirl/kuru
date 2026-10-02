//! Pooled SQL sessions are reused, not re-authenticated, by sequential work.
//!
//! Every new connection runs TCP, MySQL authentication and the identity
//! callback inside the pool's ordinary acquire window. These tests count those
//! authentications and hold any new one at an authentication gate, so a
//! regression fails at the moment a new connection starts authenticating,
//! never after a timer. A release gate likewise holds a connection's return
//! before SQLx's release ping. Precondition kept by each test: nothing but the
//! test's own sequential work (and the store's write workers it awaits)
//! touches the gated pool while either gate is armed; SQLx's spawned
//! drop-releases also pass the release gate.

use super::*;
use crate::server::{ConnectionGate, ConnectionObservation};
use futures::FutureExt;
use kuru_core::{InvocationOutcome, InvocationStart, Usage, UsageObservation, UsagePhase};

async fn observation(store: &MemoryStore, branch: &str) -> Result<ConnectionObservation> {
    store
        .shared
        .server
        .pool_observation(branch)
        .await
        .with_context(|| format!("no live pool retained for {branch}"))
}

/// A fresh pool whose sequential open work reused one session authenticated
/// exactly one connection, plus verification's own when the pool attempt's
/// deadline cut the first connection's release (the bounded close the
/// product allows, never churn).
fn ensure_one_working_session(observed: &ConnectionObservation, work: &str) -> Result<()> {
    let authenticated = observed.authenticated();
    let cut = observed.first_release_cut();
    ensure!(
        authenticated == 1 + u64::from(cut),
        "{work} authenticated {authenticated} connections (first release cut: {cut}), \
         not one working session"
    );
    Ok(())
}

/// Run one step of sequential work; fail as soon as any new connection of the
/// gated pool enters authentication instead of reusing the pooled session.
async fn reusing<T>(
    gate: &ConnectionGate,
    step: &str,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        result = work => result.with_context(|| format!("{step} failed")),
        () = gate.entered() => {
            bail!("{step} opened a new pool connection instead of reusing the pooled session")
        }
    }
}

/// The open sequence runs its statements one after another on each pool, so
/// each pool authenticates exactly one working connection.
#[tokio::test]
async fn fresh_open_authenticates_one_session_per_pool() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    for branch in ["main", usage_ledger::BRANCH] {
        ensure_one_working_session(
            &observation(&store, branch).await?,
            &format!("the open sequence on {branch}"),
        )?;
    }
    store.close().await?;
    Ok(())
}

/// A fresh branch pool authenticates once for its creation and identity
/// verification; later sequential statements and a read transaction reuse
/// that session.
#[tokio::test]
async fn sequential_statements_authenticate_one_session() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let branch = "pool_session_probe";
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?)")
            .bind(branch)
            .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("probe branch creation deadline exceeded")??;
    let pool = store.shared.server.pool(branch).await?;
    let observed = observation(&store, branch).await?;
    // Load-bearing before the gate: creation and its two identity queries
    // must have used one working session.
    ensure_one_working_session(&observed, "creating and verifying the pool")?;
    let gate = observed.gate_new_authentications();
    let one: i64 = reusing(&gate, "fetch_one", async {
        Ok(sqlx::query_scalar("SELECT 1")
            .fetch_one(pool.as_ref())
            .await?)
    })
    .await?;
    ensure!(one == 1);
    let present: Option<i64> = reusing(&gate, "fetch_optional", async {
        Ok(sqlx::query_scalar("SELECT 2")
            .fetch_optional(pool.as_ref())
            .await?)
    })
    .await?;
    ensure!(present == Some(2));
    let all: Vec<i64> = reusing(&gate, "fetch_all", async {
        Ok(sqlx::query_scalar("SELECT 3 UNION ALL SELECT 4")
            .fetch_all(pool.as_ref())
            .await?)
    })
    .await?;
    ensure!(all == [3, 4]);
    reusing(&gate, "execute", async {
        Ok(sqlx::query("SELECT 5").execute(pool.as_ref()).await?)
    })
    .await?;
    let streamed: Vec<i64> = reusing(&gate, "fetch stream", async {
        Ok(sqlx::query_scalar::<_, i64>("SELECT 6 UNION ALL SELECT 7")
            .fetch(pool.as_ref())
            .try_collect::<Vec<_>>()
            .await?)
    })
    .await?;
    ensure!(streamed == [6, 7]);
    let counted: i64 = reusing(&gate, "read transaction", async {
        let mut transaction = pool.begin().await?;
        let counted = sqlx::query_scalar("SELECT COUNT(*) FROM kuru_instance")
            .fetch_one(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(counted)
    })
    .await?;
    ensure!(counted == 1);
    let after: i64 = reusing(&gate, "statement after the transaction", async {
        Ok(sqlx::query_scalar("SELECT 8")
            .fetch_one(pool.as_ref())
            .await?)
    })
    .await?;
    ensure!(after == 8);
    ensure!(!gate.was_entered());
    ensure_one_working_session(&observed, "the pool's sequential statements")?;
    drop(gate);
    drop(pool);
    store.close().await?;
    Ok(())
}

/// Receipted writes return their session to the pool: after open, sequential
/// writes through every receipted main-pool writer and the usage ledger never
/// authenticate a new connection.
#[tokio::test]
async fn receipted_writes_reuse_their_pooled_session() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let main = observation(&store, "main").await?;
    let usage = observation(&store, usage_ledger::BRANCH).await?;
    let main_before = main.authenticated();
    let usage_before = usage.authenticated();
    let main_gate = main.gate_new_authentications();
    let usage_gate = usage.gate_new_authentications();
    let reasoning = |index: u64| ReasoningSummaryRecord {
        session_id: "pool-session".into(),
        turn_id: Some("turn".into()),
        operation_id: None,
        actor_id: "actor".into(),
        invocation_id: "invocation".into(),
        item_id: Some("item".into()),
        output_index: Some(0),
        summary_index: index,
        text: format!("summary {index}"),
    };
    let writes = async {
        for round in 0..2 {
            reusing(
                &main_gate,
                "put",
                store.put(&format!("pool.session.{round}"), &serde_json::json!(round)),
            )
            .await?;
            reusing(
                &main_gate,
                "append_message",
                store.append_message("pool-session", &Message::text("user", "question")),
            )
            .await?;
            reusing(
                &main_gate,
                "append_session_message",
                store.append_session_message(
                    "pool-session",
                    "pool-session",
                    &Message::text("assistant", "answer"),
                ),
            )
            .await?;
            reusing(
                &main_gate,
                "put_reasoning_summaries",
                store.put_reasoning_summaries(&[reasoning(round)]),
            )
            .await?;
        }
        reusing(
            &main_gate,
            "create_session_catalog",
            store.create_session_catalog("pool-session", Mode::Ifs, "pool session"),
        )
        .await?;
        let ledger = store.usage_ledger()?;
        reusing(
            &usage_gate,
            "usage mark_new_session",
            ledger.mark_new_session("pool-session"),
        )
        .await?;
        for index in 0..2 {
            let invocation = format!("pool-invocation-{index}");
            reusing(
                &usage_gate,
                "usage admit",
                ledger.admit(InvocationStart {
                    session_id: "pool-session".into(),
                    invocation_id: invocation.clone(),
                    operation_id: "turn-1".into(),
                    phase: UsagePhase::Speak,
                    actor_id: "speaker".into(),
                    route: "responses".into(),
                    model: "model".into(),
                    price_at_invocation: None,
                }),
            )
            .await?;
            reusing(
                &usage_gate,
                "usage observe",
                ledger.observe(
                    &invocation,
                    UsageObservation {
                        sequence: 1,
                        terminal: true,
                        usage: Usage {
                            input_tokens: Some(3),
                            ..Usage::default()
                        },
                    },
                ),
            )
            .await?;
            reusing(
                &usage_gate,
                "usage settle",
                ledger.settle(&invocation, InvocationOutcome::Succeeded),
            )
            .await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    // Either gate entering fails the sequence through `reusing`; a gate on
    // the other pool than the current step is checked here as well.
    tokio::select! {
        biased;
        result = writes => result?,
        () = main_gate.entered() => bail!("a write opened a new main-pool connection"),
        () = usage_gate.entered() => bail!("a write opened a new usage-pool connection"),
    }
    ensure!(main.authenticated() == main_before);
    ensure!(usage.authenticated() == usage_before);
    drop(main_gate);
    drop(usage_gate);
    store.close().await?;
    Ok(())
}

/// The dream's pool: candidate-view writes reuse the candidate pool's session.
#[tokio::test]
async fn candidate_view_writes_reuse_candidate_pool_sessions() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let candidate = store.begin_candidate("pool session probe").await?;
    let view = candidate.view();
    let observed = observation(&store, candidate.branch()).await?;
    let before = observed.authenticated();
    let gate = observed.gate_new_authentications();
    for round in 0..4 {
        reusing(
            &gate,
            "candidate put",
            view.put(&format!("dream.{round}"), &serde_json::json!(round)),
        )
        .await?;
        reusing(
            &gate,
            "candidate append_session_message",
            view.append_session_message(
                "dream",
                "dream-session",
                &Message::text("assistant", "reflection"),
            ),
        )
        .await?;
    }
    ensure!(observed.authenticated() == before);
    drop(gate);
    drop(view);
    candidate.abandon().await?;
    store.close().await?;
    Ok(())
}

/// With spare capacity and the only session held, a new connection that
/// stalls in authentication fails the acquisition naming that wait, both
/// through the funnel's own acquire and through a statement. SQLx's own
/// window is the bound under test; the gate involves no time.
#[tokio::test]
async fn stalled_authentication_names_the_new_connection_phase() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let held = pool.acquire().await?;
    let gate = pool.observation().gate_new_authentications();
    let authenticated = pool.authenticated();

    let error = pool
        .acquire()
        .await
        .err()
        .context("a stalled authentication handed out a session")?;
    ensure!(
        gate.was_entered(),
        "the acquisition did not open a connection"
    );
    let diagnostic = crate::pool::pool_acquire_timeout(&error)
        .with_context(|| format!("stalled acquire carried no typed diagnostic: {error:#}"))?;
    ensure!(
        diagnostic.wait == crate::pool::PoolWait::NewConnection,
        "{diagnostic}"
    );
    ensure!(
        diagnostic.phase == Some(crate::server::AUTHENTICATION_GATE_PHASE),
        "{diagnostic}"
    );
    ensure!(diagnostic.authenticated_during_wait == 1, "{diagnostic}");
    ensure!(
        diagnostic.authenticated_total == authenticated + 1,
        "{diagnostic}"
    );
    ensure!(
        (diagnostic.checked_out, diagnostic.idle) == (1, 0),
        "{diagnostic}"
    );
    ensure!(diagnostic.size < diagnostic.max, "{diagnostic}");
    ensure!(diagnostic.window == crate::server::ORDINARY_POOL_WINDOW);
    ensure!(diagnostic.waited >= diagnostic.window, "{diagnostic}");
    let text = format!("{error:#}");
    ensure!(
        text.contains(
            "for a new connection in Kuru's identity callback (connection phase: authentication gate entered)"
        ),
        "{text}"
    );

    // A statement through the `Executor` reports the same wait in its chain.
    let error = anyhow::Error::from(
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(pool.as_ref())
            .await
            .err()
            .context("a stalled authentication ran a statement")?,
    )
    .context("memory read failed");
    let diagnostic = crate::pool::pool_acquire_timeout(&error)
        .with_context(|| format!("statement carried no typed diagnostic: {error:#}"))?;
    ensure!(diagnostic.wait == crate::pool::PoolWait::NewConnection);
    ensure!(diagnostic.authenticated_during_wait == 1, "{diagnostic}");
    ensure!(
        format!("{error:#}").contains("memory pool acquire on kuru/main timed out"),
        "{error:#}"
    );

    drop(gate);
    held.release().await;
    drop(pool);
    store.close().await?;
    Ok(())
}

/// A release cancelled after SQLx has taken its connection (a statement
/// timeout or a `select!` firing during the release ping) stops counting the
/// session as held: SQLx closes the floating connection, and a later timed-out
/// acquire still names the wait it actually had. With every other permit
/// held, an inflated count would report held connections. The release gate
/// holds the return before its ping, so the cancellation point involves no
/// time.
#[tokio::test]
async fn cancelled_release_is_not_counted_as_a_held_connection() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let max = pool.options().get_max_connections();
    ensure!(max >= 2, "the main pool admits {max} connections");
    let mut held = Vec::new();
    for _ in 0..max {
        held.push(pool.acquire().await?);
    }
    ensure!(pool.checked_out() == max);

    let release_gate = pool.observation().gate_releases();
    let releasing = held.pop().context("no session to release")?;
    ensure!(
        releasing.release().now_or_never().is_none(),
        "the gated release completed"
    );
    ensure!(
        release_gate.was_entered(),
        "the release did not reach the pool's return"
    );
    drop(release_gate);
    let holding = u32::try_from(held.len())?;
    ensure!(
        pool.checked_out() == holding,
        "a cancelled release left {} sessions counted as held while {holding} are",
        pool.checked_out()
    );

    let gate = pool.observation().gate_new_authentications();
    let error = pool
        .acquire()
        .await
        .err()
        .context("a stalled authentication handed out a session")?;
    ensure!(
        gate.was_entered(),
        "the acquisition did not open a connection"
    );
    let diagnostic = crate::pool::pool_acquire_timeout(&error)
        .with_context(|| format!("stalled acquire carried no typed diagnostic: {error:#}"))?;
    ensure!(
        diagnostic.wait == crate::pool::PoolWait::NewConnection,
        "{diagnostic}"
    );
    ensure!(
        (diagnostic.checked_out, diagnostic.idle) == (holding, 0),
        "{diagnostic}"
    );
    ensure!(diagnostic.size < diagnostic.max, "{diagnostic}");

    drop(gate);
    for session in held {
        session.release().await;
    }
    drop(pool);
    store.close().await?;
    Ok(())
}

async fn session_is_listed(store: &MemoryStore, id: u64) -> Result<bool> {
    let active: i64 = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?")
            .bind(id)
            .fetch_one(store.pool.as_ref()),
    )
    .await
    .context("processlist read deadline exceeded")??;
    Ok(active != 0)
}

/// Guard: a write whose outcome is uncertain never returns its session to the
/// pool. The injected failure after `DOLT_COMMIT` settles by its receipt only
/// after the original session has ended.
#[tokio::test]
async fn uncertain_write_still_ends_its_session_before_reconciliation() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pause = Arc::new(ApplyPause::default());
    pause.fail_after_commit.store(true, Ordering::SeqCst);
    // Only the receipt point holds; the commit point passes straight through.
    pause.dolt_committed.resume.notify_one();
    *store.shared.apply_pause.lock().expect("apply pause lock") = Some(pause.clone());
    let writer = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .put("pool.session.uncertain", &serde_json::json!(1))
                .await
        }
    });
    tokio::time::timeout(QUERY_TIMEOUT, pause.receipt_inserted.reached.notified())
        .await
        .context("write did not reach its receipt insert")?;
    let id = store
        .shared
        .uncertain
        .lock()
        .expect("uncertain lock")
        .as_ref()
        .map(|pending| pending.connection)
        .context("paused write recorded no pending session")?;
    pause.receipt_inserted.resume.notify_one();
    writer.await.context("write worker panicked")??;
    ensure!(
        !pause.fail_after_commit.load(Ordering::SeqCst),
        "write did not take the injected post-commit failure"
    );
    ensure!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none()
    );
    ensure!(
        !session_is_listed(&store, id).await?,
        "the uncertain write's session {id} is still open"
    );
    ensure!(
        store.get("pool.session.uncertain").await? == Some(serde_json::json!(1)),
        "the reconciled write is not durable"
    );
    store.close().await?;
    Ok(())
}

/// Guard: a rejected write ends its session; only a receipted success may
/// return one to the pool.
#[tokio::test]
async fn rejected_write_closes_its_session() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let record = ReasoningSummaryRecord {
        session_id: "pool-session".into(),
        turn_id: Some("turn".into()),
        operation_id: None,
        actor_id: "actor".into(),
        invocation_id: "invocation".into(),
        item_id: Some("item".into()),
        output_index: Some(0),
        summary_index: 0,
        text: "first".into(),
    };
    store
        .put_reasoning_summaries(std::slice::from_ref(&record))
        .await?;
    // The pool now holds exactly one session; the next write must use it.
    ensure!(
        (store.pool.size(), store.pool.num_idle()) == (1, 1),
        "the pool does not hold exactly one idle session"
    );
    let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(store.pool.as_ref())
        .await?;
    let authenticated = store.pool.authenticated();
    let mut conflicting = record;
    conflicting.text = "conflicting".into();
    let error = store
        .put_reasoning_summaries(&[conflicting])
        .await
        .err()
        .context("a conflicting reasoning summary was accepted")?;
    ensure!(
        error.downcast_ref::<ReasoningSummaryConflict>().is_some(),
        "the rejection lost its typed conflict: {error:#}"
    );
    ensure!(
        !session_is_listed(&store, id).await?,
        "the rejected write's session {id} is still open"
    );
    ensure!(
        store.pool.authenticated() > authenticated,
        "reconciliation reused a session the rejected write should have ended"
    );
    store.close().await?;
    Ok(())
}

/// A receipted write's session release is bounded by the write's own budget.
/// The release gate holds the return before SQLx's ping and never opens on
/// its own, and the deadline has already passed, so only the bound can end
/// the release: the connection is closed, not pooled, and stops counting.
#[tokio::test]
async fn receipted_release_past_its_deadline_closes_the_connection() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let (session, id) = write_session(&pool).await?;
    let (size, checked_out) = (pool.size(), pool.checked_out());
    ensure!(checked_out >= 1, "the write session is not counted as held");
    let gate = pool.observation().gate_releases();
    // The outer timeout only turns an unbounded regression into a failure.
    tokio::time::timeout(
        QUERY_TIMEOUT,
        session.settle_receipted(tokio::time::Instant::now()),
    )
    .await
    .context("a receipted release past its deadline was not cut")?;
    ensure!(
        gate.was_entered(),
        "the release did not reach the pool's return"
    );
    ensure!(
        pool.checked_out() == checked_out - 1,
        "an expired release left {} sessions counted as held, not {}",
        pool.checked_out(),
        checked_out - 1
    );
    ensure!(
        pool.size() == size - 1,
        "an expired release left its connection in the pool (size {} of {size})",
        pool.size()
    );
    drop(gate);
    await_session_end(&pool, id, QUERY_TIMEOUT).await?;
    drop(pool);
    store.close().await?;
    Ok(())
}

/// A receipted write whose session release never finishes still returns
/// `Ok` once its budget ends, and its session leaves the pool. The release
/// gate holds the return indefinitely, so the write's own `QUERY_TIMEOUT`
/// budget is the bound under test and this test waits it out; the outer
/// timeout only turns an unbounded regression into a failure.
#[tokio::test]
async fn receipted_write_succeeds_when_its_release_outlasts_its_budget() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    store
        .put("pool.session.warm", &serde_json::json!(0))
        .await?;
    ensure!(
        (pool.size(), pool.num_idle()) == (1, 1),
        "the pool does not hold exactly one idle session"
    );
    // The write reuses this idle session.
    let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(pool.as_ref())
        .await?;
    let gate = pool.observation().gate_releases();
    tokio::time::timeout(
        QUERY_TIMEOUT * 2,
        store.put("pool.session.held_release", &serde_json::json!(1)),
    )
    .await
    .context("a receipted write whose release was held did not end with its budget")?
    .context("a receipted write failed because its release outlasted its budget")?;
    ensure!(
        gate.was_entered(),
        "the write's release did not reach the pool's return"
    );
    ensure!(
        (pool.size(), pool.checked_out()) == (0, 0),
        "the held release left its session in the pool (size {}, {} checked out)",
        pool.size(),
        pool.checked_out()
    );
    ensure!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none()
    );
    // Later statements release through the pool; open the gate first.
    drop(gate);
    await_session_end(&pool, id, QUERY_TIMEOUT).await?;
    ensure!(
        store.get("pool.session.held_release").await? == Some(serde_json::json!(1)),
        "the receipted write is not durable"
    );
    drop(pool);
    store.close().await?;
    Ok(())
}

/// A new branch pool's first connection release is bounded by the attempt's
/// own deadline. The take-once gate holds that release indefinitely, so only
/// the deadline ends it: the connection is closed and identity verification
/// authenticates its own. The outer timeout only turns an unbounded
/// regression into a failure.
#[tokio::test]
async fn first_connection_release_is_bounded_by_its_attempt_deadline() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let branch = "first_release_probe";
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?)")
            .bind(branch)
            .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("probe branch creation deadline exceeded")??;
    let mut hold = store.shared.server.hold_next_pool_first_release();
    let pool = tokio::time::timeout(QUERY_TIMEOUT, store.shared.server.pool(branch))
        .await
        .context("a held first release kept the branch pool from opening")??;
    let gate = hold
        .try_recv()
        .context("the pool attempt did not arm the first release hold")?;
    ensure!(
        gate.was_entered(),
        "the first connection's release did not reach the pool's return"
    );
    ensure!(
        pool.observation().first_release_cut(),
        "the held first release was not recorded as cut at the attempt's deadline"
    );
    ensure!(
        pool.authenticated() == 2,
        "{} connections authenticated; the closed first connection and verification's own were expected",
        pool.authenticated()
    );
    ensure_one_working_session(pool.observation(), "the pool with its first release cut")?;
    ensure!(
        (pool.size(), pool.num_idle()) == (1, 1),
        "the pool holds {} connections, {} idle, after the first was closed",
        pool.size(),
        pool.num_idle()
    );
    drop(gate);
    drop(pool);
    store.close().await?;
    Ok(())
}
