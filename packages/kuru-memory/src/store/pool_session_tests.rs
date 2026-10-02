//! Pooled SQL sessions are reused, not re-authenticated, by sequential work.
//!
//! Every new connection runs TCP, MySQL authentication and the identity
//! callback inside its acquisition's budget. These tests count those
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
/// exactly one connection, plus one for each identity callback whose attempt
/// was abandoned (an opening-phase first acquire retried after its window, or
/// SQLx reconnecting after a callback error) and verification's own when the
/// pool attempt's deadline cut the first connection's release. Each of those
/// is a bounded attempt the product allows, never churn of a working session.
fn ensure_one_working_session(observed: &ConnectionObservation, work: &str) -> Result<()> {
    let authenticated = observed.authenticated();
    let abandoned = observed.abandoned_authentications();
    let cut = observed.first_release_cut();
    ensure!(
        authenticated == 1 + abandoned + u64::from(cut),
        "{work} authenticated {authenticated} connections (abandoned attempts: {abandoned}, \
         first release cut: {cut}), not one working session"
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

/// The statement-budget expiry of an acquisition that `gate` holds in
/// authentication. The scope ends as soon as a new connection has entered the
/// gate, an event standing in for the budget's deadline, so reaching the gate
/// before any budget is not a runner-speed assumption.
async fn budget_ended_in_authentication<T>(
    gate: &ConnectionGate,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<anyhow::Error> {
    match crate::pool::within_or(QUERY_TIMEOUT, work, gate.entered()).await {
        Err(elapsed) => {
            ensure!(elapsed.budget == QUERY_TIMEOUT, "{elapsed}");
            Ok(anyhow::Error::from(elapsed))
        }
        Ok(Ok(_)) => bail!("a stalled authentication handed out a session"),
        Ok(Err(error)) => bail!("a stalled acquisition failed before its budget ended: {error:#}"),
    }
}

/// With spare capacity and the only session held, a new connection that
/// stalls in authentication ends the acquisition at its statement budget,
/// naming that wait, both through the funnel's own acquire and through a
/// statement. The gates involve no time.
#[tokio::test]
async fn stalled_authentication_names_the_new_connection_phase() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let held = pool.acquire().await?;
    let gate = pool.observation().gate_new_authentications();
    let authenticated = pool.authenticated();

    let error = budget_ended_in_authentication(&gate, pool.acquire()).await?;
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
    ensure!(
        diagnostic.bound == crate::pool::AcquireBound::StatementBudget,
        "{diagnostic}"
    );
    ensure!(diagnostic.budget == QUERY_TIMEOUT, "{diagnostic}");
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
    let text = format!("{error:#}");
    ensure!(
        text.contains(
            "for a new connection in Kuru's identity callback (connection phase: authentication gate entered)"
        ),
        "{text}"
    );
    // The ended acquisition dropped its held callback; a fresh gate gives the
    // statement its own entry event.
    drop(gate);

    // A statement through the `Executor` reports the same wait in its chain.
    let gate = pool.observation().gate_new_authentications();
    let error = budget_ended_in_authentication(&gate, async {
        Ok(sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(pool.as_ref())
            .await?)
    })
    .await?
    .context("memory read failed");
    let diagnostic = crate::pool::pool_acquire_timeout(&error)
        .with_context(|| format!("statement carried no typed diagnostic: {error:#}"))?;
    ensure!(
        diagnostic.wait == crate::pool::PoolWait::NewConnection,
        "{diagnostic}"
    );
    ensure!(diagnostic.authenticated_during_wait == 1, "{diagnostic}");
    ensure!(
        format!("{error:#}").contains("memory pool acquire on kuru/main timed out"),
        "{error:#}"
    );
    ensure!(
        pool.pending_acquires() == 0,
        "an ended acquisition is still counted as pending"
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
/// time, and the acquisition's budget ends once its new connection has
/// entered the authentication gate.
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
    let error = budget_ended_in_authentication(&gate, pool.acquire()).await?;
    let diagnostic = crate::pool::pool_acquire_timeout(&error)
        .with_context(|| format!("stalled acquire carried no typed diagnostic: {error:#}"))?;
    ensure!(
        diagnostic.wait == crate::pool::PoolWait::NewConnection,
        "{diagnostic}"
    );
    ensure!(
        diagnostic.bound == crate::pool::AcquireBound::StatementBudget,
        "{diagnostic}"
    );
    ensure!(
        (diagnostic.checked_out, diagnostic.idle) == (holding, 0),
        "{diagnostic}"
    );
    // The budget's diagnostic is taken while the acquisition is still alive,
    // so the pool's size counts its new connection in authentication: the
    // permit the cancelled release freed, not a held one.
    ensure!(diagnostic.size == holding + 1, "{diagnostic}");

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
    let (session, id) = write_session(&pool, write_deadline()).await?;
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

async fn create_probe_branch(store: &MemoryStore, branch: &str) -> Result<()> {
    crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?)")
            .bind(branch)
            .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("probe branch creation deadline exceeded")??;
    Ok(())
}

/// Once memory is open, a new branch pool's first acquire, first release and
/// identity verification share one creation budget. The take-once gate holds
/// the first release, and the creation's scope ends once that release has
/// reached the pool's return, an event standing in for the budget's
/// deadline, so no new connection's handshake has to beat a budget. The
/// creation fails with its budget elapsed and retains no pool; nothing is
/// retried, and a later creation opens one working session.
#[tokio::test]
async fn first_connection_release_held_past_the_creation_budget_ends_creation_at_its_budget()
-> Result<()> {
    let store = MemoryStore::temporary().await?;
    let server = &store.shared.server;
    let branch = "first_release_probe";
    create_probe_branch(&store, branch).await?;
    let hold = server.hold_next_pool_first_release();
    // The gate stays held until the scope has ended, so only the scope can
    // end the held release.
    let kept = std::sync::Mutex::new(None::<ConnectionGate>);
    let expire = async {
        // A creation that fails before its first release drops the sender;
        // then only its own outcome ends the scope.
        let Ok(gate) = hold.await else {
            return std::future::pending().await;
        };
        gate.entered().await;
        *kept.lock().expect("kept gate lock") = Some(gate);
    };
    let outcome = crate::pool::within_or(QUERY_TIMEOUT, server.pool(branch), expire).await;
    let gate = kept.lock().expect("kept gate lock").take();
    let elapsed = match outcome {
        Err(elapsed) => elapsed,
        Ok(Ok(_)) => bail!("the branch pool opened while its first release was held"),
        Ok(Err(error)) => bail!("creation failed before its first release was held: {error:#}"),
    };
    let gate = gate.context("the creation budget ended without the first release held")?;
    ensure!(elapsed.budget == QUERY_TIMEOUT, "{elapsed}");
    ensure!(
        elapsed.acquire.is_none(),
        "the creation's own acquisition is not a funnel acquisition: {elapsed}"
    );
    ensure!(
        server.pool_observation(branch).await.is_none(),
        "a creation that ran out of budget retained a pool"
    );
    drop(gate);

    let pool = server.pool(branch).await?;
    let observed = pool.observation();
    ensure!(
        !observed.first_release_cut(),
        "a creation within its budget recorded a cut first release"
    );
    ensure_one_working_session(observed, "the pool created after an expired creation")?;
    ensure!(
        observed.abandoned_authentications() == 0,
        "the pool created after an expired creation abandoned a callback"
    );
    ensure!(
        (pool.size(), pool.num_idle()) == (1, 1),
        "the pool holds {} connections, {} idle",
        pool.size(),
        pool.num_idle()
    );
    drop(pool);
    store.close().await?;
    Ok(())
}

/// An opening-phase first acquire whose identity callback outlasts the
/// opening floor, inside the remaining startup deadline, opens the pool on
/// its first attempt: the pool ceiling is the statement budget, so nothing
/// cuts the callback and no connection is abandoned. The stall is fixed at
/// one instant past the floor from the first callback.
#[tokio::test]
async fn opening_first_acquire_outlasting_the_floor_is_one_working_session() -> Result<()> {
    let root = crate::test_support::tempdir()?;
    let config = kuru_core::MemoryConfig {
        offline: true,
        ..Default::default()
    };
    let binary = {
        let _gate = crate::spawn_gate::spawning().await;
        crate::provision::provision(&config, &test_cache()).await?
    };
    let server = {
        let _gate = crate::spawn_gate::spawning().await;
        crate::server::Server::open(crate::server::ServerOptions {
            binary,
            directory: root.path().join("opening-first-acquire"),
            project_scope: "project/opening-first-acquire".into(),
            supervisor: test_supervisor()?,
            timeout: Duration::from_secs(config.startup_timeout_secs),
            read_only: false,
            retained: None,
            lifecycle_root: cfg!(windows).then(|| root.path().join("leases")),
        })
        .await?
    };
    // The server closes on every outcome, so a failed expectation reports
    // itself rather than an unreaped fixture.
    let outcome = async {
        ensure!(
            server.opening_deadline().is_some(),
            "an owned start did not enter its opening phase"
        );
        let entered = Arc::new(AtomicBool::new(false));
        server.delay_next_pool_authentication(
            crate::server::OPENING_POOL_FLOOR * 3 / 2,
            entered.clone(),
        );
        let pool = server
            .pool("main")
            .await
            .context("the opening pool did not open past the floor")?;
        ensure!(
            entered.load(Ordering::SeqCst),
            "the first authentication callback was not delayed"
        );
        let observed = pool.observation();
        ensure!(
            observed.abandoned_authentications() == 0,
            "the first callback was cut and abandoned ({} connections authenticated)",
            observed.authenticated()
        );
        ensure!(
            observed.authenticated() == 1,
            "{} connections authenticated; the first attempt's one was expected",
            observed.authenticated()
        );
        ensure_one_working_session(observed, "the opening pool past the floor")?;
        ensure!(
            (pool.size(), pool.num_idle()) == (1, 1),
            "the pool holds {} connections, {} idle, after its first acquire",
            pool.size(),
            pool.num_idle()
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    server.close().await?;
    outcome
}

/// A budget that elapses after its acquisition returned is no acquisition
/// timeout: the scope ends on an event once the session is in hand, so no
/// budget has to outlast the acquisition. The dropped session leaves the pool
/// usable for the next statement.
#[tokio::test]
async fn budget_elapsed_in_execution_is_not_an_acquire_timeout() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let acquired = tokio::sync::Notify::new();
    let outcome = crate::pool::within_or(
        QUERY_TIMEOUT,
        async {
            let _session = pool.acquire().await?;
            acquired.notify_one();
            std::future::pending::<Result<()>>().await
        },
        acquired.notified(),
    )
    .await;
    let elapsed = match outcome {
        Err(elapsed) => elapsed,
        Ok(outcome) => bail!(
            "the work ended before its budget: {:?}",
            outcome.map_err(|error| format!("{error:#}"))
        ),
    };
    ensure!(
        elapsed.acquire.is_none(),
        "an execution expiry carried an acquisition diagnostic: {elapsed}"
    );
    let error = anyhow::Error::from(elapsed).context("memory read deadline exceeded");
    ensure!(
        crate::pool::pool_acquire_timeout(&error).is_none(),
        "an execution expiry was reported as an acquisition timeout: {error:#}"
    );
    ensure!(
        format!("{error:#}")
            == "memory read deadline exceeded: memory statement budget of 30.000 s elapsed",
        "{error:#}"
    );
    ensure!(
        pool.pending_acquires() == 0,
        "a completed acquisition is still counted as pending"
    );
    let one: i64 = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT 1").fetch_one(pool.as_ref()),
    )
    .await??;
    ensure!(one == 1, "the next statement read {one}");
    drop(pool);
    store.close().await?;
    Ok(())
}

/// The uncertain-write fence: a write's acquisition precedes its pending
/// record, so an acquisition that fails leaves no uncertain write. A plain
/// store has no logical receipt and nothing pending, so the first acquisition
/// `put` makes is its write session's. Every permit is held; once that
/// acquisition is queued, the pool is marked closed (`MemoryPool::close`, a
/// synchronous mark; not `MemoryStore::close`, which waits for the write
/// guard the queued worker holds), which wakes it with SQLx's `PoolClosed`.
#[tokio::test]
async fn acquire_failure_before_a_write_is_never_uncertain() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let pool = store.pool.clone();
    let max = pool.options().get_max_connections();
    let mut held = Vec::new();
    for _ in 0..max {
        held.push(pool.acquire().await?);
    }
    let writer = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .put("pool.session.fenced", &serde_json::json!(1))
                .await
        }
    });
    tokio::pin!(writer);
    tokio::select! {
        biased;
        outcome = &mut writer => bail!(
            "the write ended before its acquisition queued: {:?}",
            outcome.map(|result| result.map_err(|error| format!("{error:#}")))
        ),
        () = pool.observation().pending_acquires_reach(1) => {}
    }
    let closing = pool.close();
    let error = match writer.await.context("write worker panicked")? {
        Ok(()) => bail!("a write on a closed pool succeeded"),
        Err(error) => error,
    };
    ensure!(
        error.chain().any(|cause| matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::PoolClosed)
        )),
        "the write's failure is not its acquisition's PoolClosed: {error:#}"
    );
    ensure!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none(),
        "a write whose acquisition failed left an uncertain record: {error:#}"
    );
    ensure!(
        pool.pending_acquires() == 0,
        "the failed acquisition is still counted as pending"
    );
    drop(held);
    closing.await;
    drop(pool);
    store.close().await?;
    Ok(())
}

const IDENTITY_MISMATCH: &str = "memory SQL project/instance identity mismatch";

/// An authored identity rejection, not a timeout of any kind.
fn ensure_identity_rejection(error: &anyhow::Error, what: &str) -> Result<()> {
    ensure!(
        format!("{error:#}").contains(IDENTITY_MISMATCH),
        "{what} did not name the identity mismatch: {error:#}"
    );
    ensure!(
        crate::pool::pool_acquire_timeout(error).is_none(),
        "{what} was reported as an acquisition timeout: {error:#}"
    );
    ensure!(
        !error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::PoolTimedOut)
            ) || cause.is::<crate::pool::BudgetElapsed>()
        }),
        "{what} waited out a bound instead of ending at the rejection: {error:#}"
    );
    Ok(())
}

/// Once memory is open, an authored identity rejection ends an acquisition at
/// once with its cause, and every later acquisition on that pool fails with it
/// without authenticating again; a pool creation that meets one fails with it
/// and retains nothing. With a sticky rejection no timer can decide, so the
/// test orders nothing against one. The branch's identity is changed through
/// a detached session of its own pool, the same per-branch working-set
/// visibility the main-pool identity test relies on.
#[tokio::test]
async fn post_open_identity_rejection_ends_the_acquire_at_once() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let server = &store.shared.server;
    let branch = "identity_rejection_probe";
    create_probe_branch(&store, branch).await?;
    let pool = server.pool(branch).await?;
    let mut connection = pool.acquire().await?.detach();
    let project_scope: String =
        sqlx::query_scalar("SELECT project_scope FROM kuru_instance WHERE singleton = 1")
            .fetch_one(&mut connection)
            .await?;
    sqlx::query("UPDATE kuru_instance SET project_scope = 'foreign-sql-project'")
        .execute(&mut connection)
        .await?;
    let outcome = async {
        let before = pool.authenticated();
        let error = pool
            .acquire()
            .await
            .err()
            .context("a pool whose identity changed handed out a session")?;
        ensure_identity_rejection(&error, "the retained pool's acquisition")?;
        let rejected = pool.authenticated();
        ensure!(
            rejected > before,
            "the rejected acquisition authenticated no new connection"
        );
        let error = pool
            .acquire()
            .await
            .err()
            .context("a rejected pool handed out a session")?;
        ensure_identity_rejection(&error, "the second acquisition")?;
        let error = anyhow::Error::from(
            sqlx::query_scalar::<_, i64>("SELECT 1")
                .fetch_one(pool.as_ref())
                .await
                .err()
                .context("a rejected pool ran a statement")?,
        );
        ensure_identity_rejection(&error, "a statement on the rejected pool")?;
        ensure!(
            pool.authenticated() == rejected,
            "a later acquisition authenticated a connection instead of failing with the \
             pool's rejection"
        );
        ensure!(
            pool.pending_acquires() == 0,
            "a rejected acquisition is still counted as pending"
        );

        // With the retained pool gone, a creation meets the same rejection.
        ensure!(
            server.pool_observation(branch).await.is_some(),
            "the branch pool was not retained before its creation half"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    drop(pool);
    let outcome = match outcome {
        Ok(()) => {
            async {
                ensure!(
                    server.pool_observation(branch).await.is_none(),
                    "the dropped branch pool is still retained"
                );
                let error = server
                    .pool(branch)
                    .await
                    .err()
                    .context("a pool creation that met an identity rejection succeeded")?;
                ensure_identity_rejection(&error, "the pool creation")?;
                ensure!(
                    server.pool_observation(branch).await.is_none(),
                    "a rejected creation retained a pool"
                );
                Ok(())
            }
            .await
        }
        Err(error) => Err(error),
    };
    sqlx::query("UPDATE kuru_instance SET project_scope = ?")
        .bind(&project_scope)
        .execute(&mut connection)
        .await?;
    sqlx::Connection::close(connection).await?;
    outcome?;
    let pool = server
        .pool(branch)
        .await
        .context("pool creation after restoring the identity")?;
    ensure_one_working_session(pool.observation(), "the pool after the restored identity")?;
    drop(pool);
    store.close().await?;
    Ok(())
}
