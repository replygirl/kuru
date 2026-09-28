use super::*;
use serde_json::json;

const TEST_DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn candidate_failure_record_exposes_only_fixed_stage_and_sql_class() {
    let error = anyhow::Error::new(CandidateRefRejected(CandidateRefRefusal::Changed))
        .context(CandidateFailureStage::RefInspection)
        .context("private branch and SQL text must not appear");
    assert_eq!(
        candidate_failure_record(&error).as_deref(),
        Some(
            "candidate_owner stage=ref_inspection class=non_sql sqlstate=none vendor=0 reason=other"
        )
    );
}

#[test]
fn branch_rename_reason_requires_the_exact_pinned_dolt_error() {
    const MESSAGE: &str = "unsafe to delete or rename branches in use in other sessions; use --force to force the change";
    // A checked cleanup delete or exclusion probe meets the same Dolt check as
    // the status rename, so both stages name it; the message stays private.
    for stage in [
        CandidateFailureStage::BranchRename,
        CandidateFailureStage::Cleanup,
    ] {
        assert_eq!(
            candidate_branch_rename_reason(stage, "HY000", 1105, MESSAGE),
            "branch_in_use"
        );
    }
    for (stage, state, vendor, message) in [
        (
            CandidateFailureStage::BranchRename,
            "HY000",
            1105,
            "another Dolt error",
        ),
        (
            CandidateFailureStage::Cleanup,
            "HY000",
            1105,
            "another Dolt error",
        ),
        (CandidateFailureStage::Cleanup, "HY001", 1105, MESSAGE),
        (CandidateFailureStage::Cleanup, "HY000", 1106, MESSAGE),
        (CandidateFailureStage::RefInspection, "HY000", 1105, MESSAGE),
        (
            CandidateFailureStage::PoolRetirement,
            "HY000",
            1105,
            MESSAGE,
        ),
        (CandidateFailureStage::MainMerge, "HY000", 1105, MESSAGE),
        (CandidateFailureStage::BranchRename, "HY001", 1105, MESSAGE),
        (CandidateFailureStage::BranchRename, "HY000", 1106, MESSAGE),
        (
            CandidateFailureStage::BranchRename,
            "HY000",
            1105,
            "unsafe to delete or rename branches in use in other sessions; use --force to force the change: private branch",
        ),
    ] {
        assert_eq!(
            candidate_branch_rename_reason(stage, state, vendor, message),
            "other"
        );
    }
}

#[tokio::test]
async fn lost_receipt_reply_settles_and_remains_indexed_after_later_write() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    store.put("first", &json!(1)).await?;
    let first_revision = store.revision().await?;

    let operation = Uuid::new_v4().to_string();
    let (mut connection, id) = owned_connection(&store.pool).await?;
    apply(
        &mut connection,
        &operation,
        "lost acknowledgement",
        Mutation::State(vec![("second".into(), "2".into())]),
        None,
    )
    .await?;
    let second_revision = store.revision().await?;
    *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::Operation(operation.clone()),
    });

    let next_store = store.clone();
    let mut next = tokio::spawn(async move { next_store.put("third", &json!(3)).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut next)
            .await
            .is_err()
    );
    assert!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_some()
    );
    assert!(operation_exists(&store.pool, &operation).await?);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?",
        )
        .bind(id)
        .fetch_one(store.pool.as_ref())
        .await?,
        1
    );
    drop(connection);
    tokio::time::timeout(TEST_DEADLINE, next).await???;
    let third_revision = store.revision().await?;

    let receipts: Vec<String> = sqlx::query_scalar("SELECT id FROM operations")
        .fetch_all(store.pool.as_ref())
        .await?;
    assert_eq!(receipts.len(), 3);
    assert!(receipts.contains(&operation));
    assert_eq!(store.get("first").await?, Some(json!(1)));
    assert_eq!(store.get("second").await?, Some(json!(2)));
    assert_eq!(store.get("third").await?, Some(json!(3)));
    let revisions = store.revisions(20).await?;
    assert!(
        revisions
            .iter()
            .any(|revision| revision.hash == first_revision)
    );
    assert!(
        revisions
            .iter()
            .any(|revision| revision.hash == second_revision)
    );
    assert!(
        revisions
            .iter()
            .any(|revision| revision.hash == third_revision)
    );
    store.close().await
}

#[tokio::test]
async fn live_transition_session_blocks_abandon_until_exact_teardown() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let candidate = Arc::new(store.begin_candidate("session barrier").await?);
    candidate
        .view()
        .put("accepted candidate write", &json!(true))
        .await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    ensure_branch_clean(&store, &names.open).await?;

    let (mut connection, id) = owned_connection(&store.pool).await?;
    *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::CandidateTransition {
            source: names.open.clone(),
            status: names.promoting.clone(),
            expected: target.clone(),
        },
    });
    sqlx::query("CALL DOLT_BRANCH('-m', ?, ?)")
        .bind(&names.open)
        .bind(&names.promoting)
        .fetch_all(&mut connection)
        .await?;
    let mut abandoning = {
        let candidate = candidate.clone();
        tokio::spawn(async move { candidate.abandon().await })
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut abandoning)
            .await
            .is_err()
    );
    assert!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_some()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?",
        )
        .bind(id)
        .fetch_one(store.pool.as_ref())
        .await?,
        1
    );
    assert_eq!(
        candidate_heads(&store.pool, &names)
            .await?
            .get(&names.promoting),
        Some(&target)
    );

    drop(connection);
    tokio::time::timeout(TEST_DEADLINE, abandoning).await???;
    assert!(candidate_heads(&store.pool, &names).await?.is_empty());
    store.put("after abandonment", &json!(true)).await?;
    store.close().await
}

#[tokio::test]
async fn dirty_promoting_retry_publishes_the_committed_head_but_preserves_the_ref() -> Result<()> {
    let directory = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        directory.path().to_path_buf(),
        format!("project/{}", "9".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    let candidate = store.begin_candidate("dirty retry").await?;
    candidate
        .view()
        .put("committed candidate write", &json!(true))
        .await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    ensure_branch_clean(&store, &names.open).await?;
    transition_candidate(&store, &names.open, &names.promoting, &target).await?;
    let status_pool = store.shared.server.pool(&names.promoting).await?;
    sqlx::query("CREATE TABLE unresolved_candidate_working_set (id INT PRIMARY KEY)")
        .execute(status_pool.as_ref())
        .await?;

    for _ in 0..2 {
        let error = candidate
            .promote()
            .await
            .expect_err("dirty promoting ref must not be reported as cleaned up");
        assert!(
            format!("{error:#}").contains("candidate working set is not clean"),
            "unexpected cleanup refusal: {error:#}"
        );
        assert_eq!(store.revision().await?, target);
        assert_eq!(
            candidate_heads(&store.pool, &names)
                .await?
                .get(&names.promoting),
            Some(&target)
        );
    }
    let heads = candidate_heads(&store.pool, &names).await?;
    assert_eq!(heads.get(&names.promoting), Some(&target));
    let reopened = store.shared.server.pool(&names.promoting).await?;
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status")
            .fetch_one(reopened.as_ref())
            .await?
            > 0
    );
    drop(status_pool);
    drop(reopened);
    drop(candidate);
    store.close().await?;

    let reopened = crate::test_support::spawn_gated_open(options.clone()).await?;
    assert_eq!(reopened.revision().await?, target);
    assert_eq!(
        candidate_heads(&reopened.pool, &names)
            .await?
            .get(&names.promoting),
        Some(&target)
    );
    let preserved = reopened.shared.server.pool(&names.promoting).await?;
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status")
            .fetch_one(preserved.as_ref())
            .await?
            > 0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT value FROM state WHERE `key` = 'committed candidate write'",
        )
        .fetch_one(preserved.as_ref())
        .await?,
        "true"
    );
    reopened
        .put("ordinary main remains writable", &json!(true))
        .await?;
    assert_eq!(
        reopened.get("ordinary main remains writable").await?,
        Some(json!(true))
    );
    assert_eq!(
        candidate_heads(&reopened.pool, &names)
            .await?
            .get(&names.promoting),
        Some(&target)
    );
    sqlx::query("DROP TABLE unresolved_candidate_working_set")
        .execute(preserved.as_ref())
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status")
            .fetch_one(preserved.as_ref())
            .await?,
        0
    );
    drop(preserved);
    let main_before_cleanup = reopened.revision().await?;
    reopened.recover_candidates().await?;
    assert_eq!(reopened.revision().await?, main_before_cleanup);
    assert!(candidate_heads(&reopened.pool, &names).await?.is_empty());
    reopened.close().await?;

    let recovered = crate::test_support::spawn_gated_open(options).await?;
    assert!(candidate_heads(&recovered.pool, &names).await?.is_empty());
    assert_eq!(
        recovered.get("committed candidate write").await?,
        Some(json!(true))
    );
    assert_eq!(
        recovered.get("ordinary main remains writable").await?,
        Some(json!(true))
    );
    recovered.close().await
}

#[tokio::test]
async fn held_candidate_view_delays_explicit_abandonment_without_losing_history() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let candidate = Arc::new(store.begin_candidate("held view").await?);
    candidate
        .view()
        .append("candidate history", "assistant", "accepted")
        .await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    ensure_branch_clean(&store, &names.open).await?;
    let source_pool = store.shared.server.pool(&names.open).await?;
    let (mut held, held_id) = owned_connection(&source_pool).await?;
    let database: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&mut held)
        .await?;
    let exact_session: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ? AND BINARY DB = BINARY ?",
    )
    .bind(held_id)
    .bind(&database)
    .fetch_one(store.pool.as_ref())
    .await?;
    assert_eq!(
        exact_session, 1,
        "held source session was not server-visible"
    );
    let wait_started = store.shared.server.observe_next_candidate_wait().await;
    let waiting_store = store.clone();
    let source = names.open.clone();
    let status = names.abandoned.clone();
    let expected = target.clone();
    let waiting = tokio::spawn(async move {
        transition_candidate_with_retirement_deadline(
            &waiting_store,
            &source,
            &status,
            &expected,
            Duration::from_secs(2),
        )
        .await
    });
    tokio::time::timeout(TEST_DEADLINE, wait_started)
        .await
        .context("held source session was not observed by the retirement wait")??;
    let error = tokio::time::timeout(TEST_DEADLINE, waiting)
        .await??
        .expect_err("live candidate session permitted status rename");
    assert!(
        format!("{error:#}").contains("candidate source session retirement deadline exceeded"),
        "held source session did not cause bounded pre-rename refusal"
    );
    let record = candidate_failure_record(&error)
        .context("held-session refusal lost its fixed owner-stage diagnostic")?;
    assert_eq!(
        record,
        "candidate_owner stage=pool_retirement class=non_sql sqlstate=none vendor=0 reason=other",
        "held-session refusal was not classified before branch rename"
    );
    eprintln!("{record}");
    let heads = candidate_heads(&store.pool, &names).await?;
    assert_eq!(heads.get(&names.open), Some(&target));
    assert!(!heads.contains_key(&names.abandoned));
    assert!(store.history("candidate history", 10).await?.is_empty());
    assert!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none()
    );

    let exact_session: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ? AND BINARY DB = BINARY ?",
    )
    .bind(held_id)
    .bind(&database)
    .fetch_one(store.pool.as_ref())
    .await?;
    assert_eq!(exact_session, 1, "held source session ended before refusal");
    drop(held);
    await_session_end(&store.pool, held_id, TEST_DEADLINE).await?;
    drop(source_pool);
    tokio::time::timeout(TEST_DEADLINE, candidate.abandon()).await??;
    assert!(candidate_heads(&store.pool, &names).await?.is_empty());
    store.put("after held view", &json!(true)).await?;
    store.close().await
}

#[tokio::test]
async fn candidate_source_admission_waits_for_server_session_before_status_rename() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let candidate = store.begin_candidate("source admission fence").await?;
    candidate
        .view()
        .put("candidate effect", &json!(true))
        .await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    let pool = candidate.view.pool.clone();
    let (mut held, id) = owned_connection(&pool).await?;
    let source_database: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&mut held)
        .await?;
    assert!(
        source_database == format!("kuru/{}", names.open),
        "candidate session selected the wrong database"
    );
    let observed_database: Option<String> =
        sqlx::query_scalar("SELECT DB FROM information_schema.processlist WHERE ID = ?")
            .bind(id)
            .fetch_one(store.pool.as_ref())
            .await?;
    assert!(
        observed_database.as_deref() == Some(source_database.as_str()),
        "held candidate session has the wrong processlist database"
    );

    let transition_store = store.clone();
    let source = names.open.clone();
    let status = names.promoting.clone();
    let expected = target.clone();
    let wait_started = store.shared.server.observe_next_candidate_wait().await;
    let mut transition = tokio::spawn(async move {
        transition_candidate(&transition_store, &source, &status, &expected).await
    });
    tokio::time::timeout(TEST_DEADLINE, wait_started)
        .await
        .context("candidate pool was not retired before the server-session wait")??;
    assert!(
        pool.is_closed(),
        "candidate pool stayed open at the wait boundary"
    );
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.processlist WHERE BINARY DB = BINARY ?",
    )
    .bind(&source_database)
    .fetch_one(store.pool.as_ref())
    .await?;
    assert!(active > 0, "held candidate session was not server-visible");
    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut transition)
            .await
            .is_err(),
        "status rename completed while its source session remained active"
    );
    assert_eq!(
        candidate_heads(&store.pool, &names).await?.get(&names.open),
        Some(&target)
    );
    assert!(
        !candidate_heads(&store.pool, &names)
            .await?
            .contains_key(&names.promoting)
    );
    assert!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none()
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(80),
            store.shared.server.fence_pool(&names.open),
        )
        .await
        .is_err(),
        "source admission guard was not held during the server-session wait"
    );

    let server = store.shared.server.clone();
    let blocked_source = names.open.clone();
    let (attempted, attempt_started) = tokio::sync::oneshot::channel();
    let mut source_acquisition = tokio::spawn(async move {
        let _ = attempted.send(());
        server.pool(&blocked_source).await
    });
    tokio::time::timeout(TEST_DEADLINE, attempt_started).await??;
    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut source_acquisition)
            .await
            .is_err(),
        "a new source pool crossed the status-transition fence"
    );
    let unrelated =
        tokio::time::timeout(TEST_DEADLINE, store.shared.server.pool(&store.branch)).await??;
    drop(unrelated);

    drop(held);
    await_session_end(&store.pool, id, TEST_DEADLINE).await?;
    tokio::time::timeout(TEST_DEADLINE, transition).await???;
    assert!(
        tokio::time::timeout(TEST_DEADLINE, source_acquisition)
            .await??
            .is_err(),
        "the retired source branch became available again after status rename"
    );
    let heads = candidate_heads(&store.pool, &names).await?;
    assert!(heads.get(&names.promoting) == Some(&target));
    assert!(!heads.contains_key(&names.open));
    drop(candidate);
    store.close().await
}

#[tokio::test]
async fn candidate_pool_retirement_observes_exact_server_sessions_before_rename() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let observation = async {
        let candidate = store.begin_candidate("retirement observation").await?;
        candidate
            .view()
            .put("candidate write", &json!(true))
            .await?;
        let names = CandidateNames::from_open(&candidate.view.branch)?;
        let target = candidate.view().revision().await?;
        let pool = candidate.view.pool.clone();

        // Keep all four pool permits checked out together. Repeated queries on one
        // idle connection would not identify a different session in the same pool.
        let mut connections = Vec::new();
        let mut ids = BTreeSet::new();
        for _ in 0..4 {
            let mut connection = pool.acquire().await?;
            let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *connection)
                .await?;
            let database: String = sqlx::query_scalar("SELECT DATABASE()")
                .fetch_one(&mut *connection)
                .await?;
            assert!(
                database == format!("kuru/{}", names.open),
                "candidate session database mismatch"
            );
            assert!(
                ids.insert(id),
                "candidate pool reused a checked-out session"
            );
            let observed: Option<String> =
                sqlx::query_scalar("SELECT DB FROM information_schema.processlist WHERE ID = ?")
                    .bind(id)
                    .fetch_one(store.pool.as_ref())
                    .await?;
            assert!(
                observed.as_deref() == Some(database.as_str()),
                "candidate processlist database mismatch"
            );
            connections.push(connection);
        }
        assert_eq!(ids.len(), 4);
        drop(connections);

        // The product's own retirement step before every candidate rename and
        // delete; its session wait makes the count below a guarantee. Its
        // admission fence ends here, before the transition below fences again.
        {
            let admission = store.shared.server.fence_pool(&names.open).await?;
            let _sessions = retire_branch_sessions(&store, &admission, QUERY_TIMEOUT).await?;
        }
        let mut active_after_close = 0;
        for id in ids {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?",
            )
            .bind(id)
            .fetch_one(store.pool.as_ref())
            .await?;
            assert!(active <= 1);
            active_after_close += active;
        }
        eprintln!(
            "candidate_retirement stage=after_pool_close active_sessions={active_after_close}"
        );

        if let Err(error) =
            transition_candidate(&store, &names.open, &names.promoting, &target).await
        {
            let mut sqlstate = "none";
            let mut vendor = 0;
            for cause in error.chain() {
                if let Some(sqlx::Error::Database(database)) = cause.downcast_ref::<sqlx::Error>() {
                    if let Some(mysql) =
                        database.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                    {
                        sqlstate = mysql
                            .code()
                            .filter(|code| {
                                code.len() == 5
                                    && code.bytes().all(|byte| {
                                        byte.is_ascii_uppercase() || byte.is_ascii_digit()
                                    })
                            })
                            .unwrap_or("other");
                        vendor = mysql.number();
                    }
                    break;
                }
            }
            eprintln!(
                "candidate_retirement stage=rename_failed sqlstate={sqlstate} vendor={vendor}"
            );
            let heads = candidate_heads(&store.pool, &names).await?;
            assert!(
                heads.get(&names.open) == Some(&target),
                "failed candidate rename changed the source head"
            );
            assert!(!heads.contains_key(&names.promoting));
            bail!("candidate retirement stage=rename_failed sqlstate={sqlstate} vendor={vendor}");
        }
        eprintln!("candidate_retirement stage=rename_succeeded");
        let heads = candidate_heads(&store.pool, &names).await?;
        assert!(
            heads.get(&names.promoting) == Some(&target),
            "successful candidate rename lost the status head"
        );
        assert!(!heads.contains_key(&names.open));
        ensure!(
            active_after_close == 0,
            "candidate retirement left an owned server session active before rename"
        );
        drop(pool);
        drop(candidate);
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let closed = store.close().await;
    observation?;
    closed
}

#[tokio::test]
async fn startup_reclaims_only_resolved_status_and_never_finishes_a_pending_promotion() -> Result<()>
{
    let directory = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        directory.path().to_path_buf(),
        format!("project/{}", "7".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;

    let unmerged = store.begin_candidate("unmerged").await?;
    unmerged.view().put("unmerged", &json!(true)).await?;
    let unmerged_names = CandidateNames::from_open(&unmerged.view.branch)?;
    let unmerged_target = unmerged.view().revision().await?;
    ensure_branch_clean(&store, &unmerged_names.open).await?;
    transition_candidate(
        &store,
        &unmerged_names.open,
        &unmerged_names.promoting,
        &unmerged_target,
    )
    .await?;

    let ordinary = store.begin_candidate("ordinary").await?;
    ordinary.view().put("ordinary", &json!(true)).await?;
    let ordinary_names = CandidateNames::from_open(&ordinary.view.branch)?;
    let ordinary_target = ordinary.view().revision().await?;

    let merged = store.begin_candidate("merged").await?;
    merged.view().put("merged", &json!(true)).await?;
    let merged_names = CandidateNames::from_open(&merged.view.branch)?;
    let merged_target = merged.view().revision().await?;
    ensure_branch_clean(&store, &merged_names.open).await?;
    transition_candidate(
        &store,
        &merged_names.open,
        &merged_names.promoting,
        &merged_target,
    )
    .await?;
    sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
        .bind(&merged_names.promoting)
        .fetch_all(store.pool.as_ref())
        .await?;
    assert_eq!(store.revision().await?, merged_target);
    store.close().await?;

    let reopened = crate::test_support::spawn_gated_open(options).await?;
    assert_eq!(reopened.revision().await?, merged_target);
    let unmerged_heads = candidate_heads(&reopened.pool, &unmerged_names).await?;
    assert_eq!(
        unmerged_heads.get(&unmerged_names.promoting),
        Some(&unmerged_target)
    );
    assert!(
        candidate_heads(&reopened.pool, &merged_names)
            .await?
            .is_empty()
    );
    let ordinary_heads = candidate_heads(&reopened.pool, &ordinary_names).await?;
    assert_eq!(
        ordinary_heads.get(&ordinary_names.open),
        Some(&ordinary_target)
    );
    let preserved = reopened
        .shared
        .server
        .pool(&unmerged_names.promoting)
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM state WHERE `key` = 'unmerged'")
            .fetch_one(preserved.as_ref())
            .await?,
        "true"
    );
    reopened.close().await
}

#[tokio::test]
async fn startup_reconciles_equal_dual_refs_and_preserves_mismatched_refs() -> Result<()> {
    let directory = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        directory.path().to_path_buf(),
        format!("project/{}", "6".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;

    let equal = store.begin_candidate("equal dual refs").await?;
    equal.view().put("equal dual refs", &json!(true)).await?;
    let equal_names = CandidateNames::from_open(&equal.view.branch)?;
    let equal_target = equal.view().revision().await?;
    ensure_branch_clean(&store, &equal_names.open).await?;
    sqlx::query("CALL DOLT_BRANCH(?, ?)")
        .bind(&equal_names.promoting)
        .bind(&equal_target)
        .fetch_all(store.pool.as_ref())
        .await?;
    let equal_heads = candidate_heads(&store.pool, &equal_names).await?;
    assert_eq!(equal_heads.get(&equal_names.open), Some(&equal_target));
    assert_eq!(equal_heads.get(&equal_names.promoting), Some(&equal_target));
    sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
        .bind(&equal_names.promoting)
        .fetch_all(store.pool.as_ref())
        .await?;
    assert_eq!(store.revision().await?, equal_target);

    let mismatch = store.begin_candidate("mismatched dual refs").await?;
    mismatch.view().put("mismatched", &json!(true)).await?;
    let mismatch_names = CandidateNames::from_open(&mismatch.view.branch)?;
    let mismatch_target = mismatch.view().revision().await?;
    sqlx::query("CALL DOLT_BRANCH(?, ?)")
        .bind(&mismatch_names.promoting)
        .bind(&equal_target)
        .fetch_all(store.pool.as_ref())
        .await?;
    drop(equal);
    drop(mismatch);
    store.close().await?;

    let reopened = crate::test_support::spawn_gated_open(options).await?;
    assert!(
        candidate_heads(&reopened.pool, &equal_names)
            .await?
            .is_empty()
    );
    let mismatch_heads = candidate_heads(&reopened.pool, &mismatch_names).await?;
    assert_eq!(
        mismatch_heads.get(&mismatch_names.open),
        Some(&mismatch_target)
    );
    assert_eq!(
        mismatch_heads.get(&mismatch_names.promoting),
        Some(&equal_target)
    );
    reopened
        .put("main after mismatched refs", &json!(true))
        .await?;
    assert_eq!(
        candidate_heads(&reopened.pool, &mismatch_names).await?,
        mismatch_heads
    );
    reopened.close().await
}

#[tokio::test]
async fn cancelled_startup_recovery_reaps_before_handoff_and_preserves_pending_intent() -> Result<()>
{
    let directory = crate::test_support::tempdir()?;
    let options = crate::test_support::open_options(
        directory.path().to_path_buf(),
        format!("project/{}", "8".repeat(64)),
    )?;
    let store = crate::test_support::spawn_gated_open(options.clone()).await?;
    let base = store.revision().await?;
    let candidate = store.begin_candidate("cancelled recovery").await?;
    candidate.view().put("pending", &json!(true)).await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    ensure_branch_clean(&store, &names.open).await?;
    transition_candidate(&store, &names.open, &names.promoting, &target).await?;
    store.close().await?;

    let reached = Arc::new(Semaphore::new(0));
    let resume = Arc::new(Semaphore::new(0));
    let mut interrupted = options.clone();
    interrupted.candidate_recovery_pause = Some(Arc::new(CandidateRecoveryPause {
        reached: reached.clone(),
        resume: resume.clone(),
    }));
    let opening = tokio::spawn(crate::test_support::spawn_gated_open(interrupted));
    let _permit = tokio::time::timeout(TEST_DEADLINE, reached.acquire()).await??;
    opening.abort();
    assert!(
        opening
            .await
            .expect_err("cancelled open completed")
            .is_cancelled()
    );
    resume.add_permits(1);

    let reopened = tokio::time::timeout(
        TEST_DEADLINE,
        crate::test_support::spawn_gated_open(options),
    )
    .await??;
    assert_eq!(reopened.revision().await?, base);
    let heads = candidate_heads(&reopened.pool, &names).await?;
    assert_eq!(heads.get(&names.promoting), Some(&target));
    let pending = reopened.shared.server.pool(&names.promoting).await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM state WHERE `key` = 'pending'")
            .fetch_one(pending.as_ref())
            .await?,
        "true"
    );
    reopened.close().await
}

async fn export_records(snapshot: &ActiveExportSnapshot) -> Result<Vec<StorageRecord>> {
    let mut cursor = None;
    let mut records = Vec::new();
    loop {
        let page = snapshot.page(cursor).await?;
        records.extend(page.records);
        let Some(next) = page.next else {
            return Ok(records);
        };
        cursor = Some(next);
    }
}

#[tokio::test]
async fn full_gc_preserves_live_candidate_historical_and_export_views() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    store.append("conversation", "user", "before gc").await?;
    store.put("retained", &json!("first")).await?;
    let historical_revision = store.revision().await?;
    let historical = store.shared.server.pool(&historical_revision).await?;
    let snapshot = store.begin_active_export().await?;
    let before_export = export_records(&snapshot).await?;

    store
        .append("conversation", "assistant", "after snapshot")
        .await?;
    let live_revision = store.revision().await?;
    let candidate = store.begin_candidate("gc candidate").await?;
    candidate
        .view()
        .put("candidate retained", &json!(true))
        .await?;
    let candidate_revision = candidate.view().revision().await?;
    let auto_gc_enabled: i64 = sqlx::query_scalar("SELECT @@GLOBAL.dolt_auto_gc_enabled")
        .fetch_one(store.pool.as_ref())
        .await?;
    assert_eq!(auto_gc_enabled, 1);

    tokio::time::timeout(
        TEST_DEADLINE,
        sqlx::query("CALL DOLT_GC('--full')").fetch_all(store.pool.as_ref()),
    )
    .await??;
    assert_eq!(store.revision().await?, live_revision);
    assert_eq!(candidate.view().revision().await?, candidate_revision);
    assert_eq!(export_records(&snapshot).await?, before_export);
    assert_eq!(revision(&historical).await?, historical_revision);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
            .fetch_one(historical.as_ref())
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(store.pool.as_ref())
            .await?,
        1
    );
    candidate.abandon().await?;
    store.close().await
}

/// One real server session on `branch` that Kuru's pool bookkeeping no longer
/// owns: it stands in for a session the server still counts after the client
/// released it. It ends only when the test sends on the returned channel.
struct LingeringSession {
    id: u64,
    database: String,
    release: tokio::sync::oneshot::Sender<()>,
    closed: tokio::task::JoinHandle<Result<()>>,
}

async fn lingering_session(store: &MemoryStore, branch: &str) -> Result<LingeringSession> {
    let pool = store.shared.server.pool(branch).await?;
    let mut connection = pool.acquire().await?.detach();
    drop(pool);
    let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(&mut connection)
        .await?;
    let database: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(database, format!("kuru/{branch}"));
    assert_eq!(session_count(store, id, &database).await?, 1);
    let (release, released) = tokio::sync::oneshot::channel();
    let closed = tokio::spawn(async move {
        let _ = released.await;
        sqlx::Connection::close(connection).await?;
        Ok(())
    });
    Ok(LingeringSession {
        id,
        database,
        release,
        closed,
    })
}

impl LingeringSession {
    async fn end(self, store: &MemoryStore) -> Result<()> {
        let _ = self.release.send(());
        tokio::time::timeout(TEST_DEADLINE, self.closed).await???;
        await_session_end(&store.pool, self.id, TEST_DEADLINE).await
    }
}

async fn session_count(store: &MemoryStore, id: u64, database: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ? AND BINARY DB = BINARY ?",
    )
    .bind(id)
    .bind(database)
    .fetch_one(store.pool.as_ref())
    .await?)
}

/// Reach promotion's committed state without its cleanup: the checked status
/// rename, then the same fast-forward merge on main that `promote_checked`
/// issues. `promote()` then takes its already-merged arm straight to cleanup.
async fn merged_promoting_candidate(
    store: &MemoryStore,
    label: &str,
) -> Result<(Arc<Candidate>, CandidateNames, String)> {
    let candidate = store.begin_candidate(label).await?;
    candidate.view().put(label, &json!(true)).await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    transition_candidate(store, &names.open, &names.promoting, &target).await?;
    let (mut main, _) = owned_connection(&store.pool).await?;
    // The owned connections that issue checked branch procedures come from
    // this pool; its session must never hold a candidate branch itself.
    let main_database: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&mut main)
        .await?;
    assert_eq!(main_database, format!("kuru/{}", store.branch));
    assert_ne!(store.branch, names.promoting);
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
            .bind(&names.promoting)
            .fetch_all(&mut main),
    )
    .await??;
    sqlx::Connection::close(main).await?;
    assert_eq!(store.revision().await?, target);
    Ok((Arc::new(candidate), names, target))
}

/// Race the spawned cleanup against the product's session-wait observation.
/// Before the wait existed the checked branch procedure met the lingering
/// session at once; report exactly the fixed diagnostic record it produced.
async fn await_session_wait<T: std::fmt::Debug>(
    wait_started: tokio::sync::oneshot::Receiver<()>,
    operation: &mut tokio::task::JoinHandle<Result<T>>,
    path: &str,
) -> Result<()> {
    tokio::select! {
        biased;
        observed = tokio::time::timeout(TEST_DEADLINE, wait_started) => {
            observed.with_context(|| format!("{path} did not observe its lingering session"))??;
            Ok(())
        }
        finished = &mut *operation => {
            let result = finished?;
            let record = result.as_ref().err().and_then(candidate_failure_record);
            bail!(
                "{path} ran its checked branch procedure while a server session still held \
                 the branch: ok={} record={record:?}",
                result.is_ok()
            )
        }
    }
}

#[tokio::test]
async fn promoted_cleanup_deletes_status_ref_only_after_its_lingering_session_ends() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let (candidate, names, target) =
        merged_promoting_candidate(&store, "promoted cleanup linger").await?;
    let session = lingering_session(&store, &names.promoting).await?;

    let wait_started = store.shared.server.observe_next_candidate_wait().await;
    let promoting = candidate.clone();
    let mut promotion = tokio::spawn(async move { promoting.promote().await });
    await_session_wait(wait_started, &mut promotion, "promoted cleanup").await?;

    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut promotion)
            .await
            .is_err(),
        "promoted cleanup finished while its status session remained active"
    );
    assert_eq!(
        session_count(&store, session.id, &session.database).await?,
        1
    );
    let heads = candidate_heads(&store.pool, &names).await?;
    assert_eq!(heads.get(&names.promoting), Some(&target));
    assert!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none(),
        "the checked delete was issued before the session wait ended"
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(80),
            store.shared.server.fence_pool(&names.promoting),
        )
        .await
        .is_err(),
        "status admission was not fenced during the session wait"
    );

    session.end(&store).await?;
    let promoted = tokio::time::timeout(TEST_DEADLINE, promotion).await???;
    assert_eq!(promoted, target);
    assert!(candidate_heads(&store.pool, &names).await?.is_empty());
    assert_eq!(store.revision().await?, target);
    assert_eq!(
        store.get("promoted cleanup linger").await?,
        Some(json!(true))
    );
    drop(candidate);
    store.close().await
}

#[tokio::test]
async fn abandoned_cleanup_probes_and_deletes_only_after_its_lingering_session_ends() -> Result<()>
{
    let store = MemoryStore::temporary().await?;
    let base = store.revision().await?;
    let candidate = Arc::new(store.begin_candidate("abandoned cleanup linger").await?);
    candidate
        .view()
        .put("abandoned cleanup linger", &json!(true))
        .await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    transition_candidate(&store, &names.open, &names.abandoned, &target).await?;
    let session = lingering_session(&store, &names.abandoned).await?;

    let wait_started = store.shared.server.observe_next_candidate_wait().await;
    let abandoning = candidate.clone();
    let mut abandonment = tokio::spawn(async move { abandoning.abandon().await });
    await_session_wait(wait_started, &mut abandonment, "abandoned cleanup").await?;

    assert!(
        tokio::time::timeout(Duration::from_millis(80), &mut abandonment)
            .await
            .is_err(),
        "abandoned cleanup finished while its status session remained active"
    );
    assert_eq!(
        session_count(&store, session.id, &session.database).await?,
        1
    );
    let heads = candidate_heads(&store.pool, &names).await?;
    assert_eq!(heads.get(&names.abandoned), Some(&target));
    assert!(
        store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .is_none(),
        "the exclusion probe or forced delete was issued before the session wait ended"
    );

    session.end(&store).await?;
    tokio::time::timeout(TEST_DEADLINE, abandonment).await???;
    assert!(candidate_heads(&store.pool, &names).await?.is_empty());
    assert_eq!(store.revision().await?, base);
    assert_eq!(store.get("abandoned cleanup linger").await?, None);
    drop(candidate);
    store.close().await
}

/// A session that outlives the retirement bound fails the step before any
/// branch procedure: no ref changes, no forced or repeated delete, and the
/// fixed record names pool retirement, exactly as the gated rename does.
#[tokio::test]
async fn candidate_deletion_refuses_a_session_that_outlives_retirement() -> Result<()> {
    let store = MemoryStore::temporary().await?;
    let (promoted, promoted_names, promoted_target) =
        merged_promoting_candidate(&store, "promoted retirement bound").await?;
    let abandoned = Arc::new(store.begin_candidate("abandoned retirement bound").await?);
    abandoned
        .view()
        .put("abandoned retirement bound", &json!(true))
        .await?;
    let abandoned_names = CandidateNames::from_open(&abandoned.view.branch)?;
    let abandoned_target = abandoned.view().revision().await?;
    transition_candidate(
        &store,
        &abandoned_names.open,
        &abandoned_names.abandoned,
        &abandoned_target,
    )
    .await?;

    for (names, branch, target, force) in [
        (
            &promoted_names,
            &promoted_names.promoting,
            &promoted_target,
            false,
        ),
        (
            &abandoned_names,
            &abandoned_names.abandoned,
            &abandoned_target,
            true,
        ),
    ] {
        let session = lingering_session(&store, branch).await?;
        let error = tokio::time::timeout(
            TEST_DEADLINE,
            delete_candidate_ref(&store, branch, target, force, Duration::from_millis(500)),
        )
        .await?
        .expect_err("a checked candidate delete crossed a live server session");
        assert_eq!(
            candidate_failure_record(&error).as_deref(),
            Some(
                "candidate_owner stage=pool_retirement class=non_sql sqlstate=none vendor=0 reason=other"
            ),
            "force={force}: the refusal was not the bounded session wait"
        );
        assert!(
            format!("{error:#}").contains("candidate source session retirement deadline exceeded")
        );
        assert_eq!(
            session_count(&store, session.id, &session.database).await?,
            1
        );
        let heads = candidate_heads(&store.pool, names).await?;
        assert_eq!(heads.get(branch.as_str()), Some(target));
        assert!(
            store
                .shared
                .uncertain
                .lock()
                .expect("uncertain lock")
                .is_none()
        );
        session.end(&store).await?;
    }
    assert!(
        promoted
            .promoted
            .lock()
            .expect("candidate result lock")
            .is_none(),
        "a refused cleanup recorded the promotion as complete"
    );

    // Nothing was lost: once the sessions end, the ordinary product paths finish.
    assert_eq!(
        tokio::time::timeout(TEST_DEADLINE, promoted.promote()).await??,
        promoted_target
    );
    tokio::time::timeout(TEST_DEADLINE, abandoned.abandon()).await??;
    assert!(
        candidate_heads(&store.pool, &promoted_names)
            .await?
            .is_empty()
    );
    assert!(
        candidate_heads(&store.pool, &abandoned_names)
            .await?
            .is_empty()
    );
    assert_eq!(store.revision().await?, promoted_target);
    drop(promoted);
    drop(abandoned);
    store.close().await
}

/// A branch session that outlives any bound until the test releases it. The
/// server closes a session idle for its listener `read_timeout_millis`, which
/// `server_yaml` sets to `QUERY_TIMEOUT`, so an idle session would race the
/// retirement bound; this one issues a trivial statement every second until
/// the release signal.
async fn active_lingering_session(store: &MemoryStore, branch: &str) -> Result<LingeringSession> {
    let pool = store.shared.server.pool(branch).await?;
    let mut connection = pool.acquire().await?.detach();
    drop(pool);
    let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
        .fetch_one(&mut connection)
        .await?;
    let database: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(database, format!("kuru/{branch}"));
    assert_eq!(session_count(store, id, &database).await?, 1);
    let (release, mut released) = tokio::sync::oneshot::channel::<()>();
    let closed = tokio::spawn(async move {
        let mut activity = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = &mut released => break,
                _ = activity.tick() => {
                    sqlx::query("SELECT 1").execute(&mut connection).await?;
                }
            }
        }
        sqlx::Connection::close(connection).await?;
        Ok(())
    });
    Ok(LingeringSession {
        id,
        database,
        release,
        closed,
    })
}

/// Slow (about 31 s): holds one real server session on the status ref for the
/// full constant `QUERY_TIMEOUT` that every managed cleanup passes, so the
/// managed client meets the product bound itself, not a test-supplied one.
///
/// A session can only meet the deletion wait on a status ref (a session on
/// the open ref meets the status rename's wait instead), so the owner's store
/// first performs the product's own `open -> abandoned` rename, as
/// `candidate_deletion_refuses_a_session_that_outlives_retirement` does: an
/// abandonment interrupted after its durable rename. The managed client then
/// drives the remaining cleanup through the service with its exact target.
///
/// The owner's handler is the 30 s wait plus further steps, and the client's
/// reply bound is 35 s, so the reply can arrive complete (`StorageFailed`) or
/// be lost to the client's own deadline. The test names the shape that
/// arrived and asserts the fence, the unchanged refs and the unchanged main
/// on either. That the wait ran for its full bound rests on a server-side
/// event, not on the client's elapsed time: the owner holds the status ref's
/// admission across its wait, and a second admission, queued only once the
/// owner has observed the live session, is granted only when the owner
/// releases it.
#[tokio::test]
async fn slow_30s_managed_abandon_cleanup_bound_fences_client_and_keeps_status_ref() -> Result<()> {
    crate::test_support::warm_runtime_cache().await?;
    // Real lifecycles: one fresh service owner, then one local reopen. The
    // fixture's single-stall term is this test's one `QUERY_TIMEOUT` stall.
    let deadline = crate::test_support::FixtureDeadline::start(
        crate::test_support::fixture_deadline(1, 1),
        "bounded managed abandon fixture",
    );
    let root = crate::test_support::tempdir()?;
    let project = root.path().join("project");
    std::fs::create_dir(&project)?;
    let project = project.canonicalize()?;
    let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
    let scope = format!(
        "project/{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let options = crate::test_support::open_options(root.path().join("private"), scope)?;
    let outcome = async {
        let names = deadline
            .serve(
                async |served| {
                    // Hold the shared spawn guard only across calls that can
                    // start a process, never across the bounded wait below.
                    let (owner_store, memory) = {
                        let _gate = crate::spawn_gate::spawning().await;
                        let owner =
                            crate::service::ServiceOwner::open(options.clone(), &project).await?;
                        let owner_store = owner.inspection_store_for_test();
                        served.serve(owner)?;
                        let memory = crate::MemoryStore::open_managed_observed(
                            options.clone(),
                            project.clone(),
                            std::env::current_exe()?,
                        )
                        .1
                        .await?;
                        (owner_store, memory)
                    };
                    let candidate = memory.begin_candidate("bounded managed abandon").await?;
                    candidate.view().put("private", &json!(1)).await?;
                    let target = candidate.view().revision().await?;
                    let base = candidate.base().to_owned();
                    ensure!(target != base);
                    let page = owner_store.candidate_inventory(None, 16).await?;
                    let [status] = page.candidates.as_slice() else {
                        bail!("expected exactly one candidate ref, found {page:?}");
                    };
                    ensure!(status.state == CandidateRefState::OpenUnchanged);
                    ensure!(status.head.as_deref() == Some(target.as_str()));
                    let names = CandidateNames::from_open(&status.branch)?;
                    transition_candidate(&owner_store, &names.open, &names.abandoned, &target)
                        .await?;
                    let session = active_lingering_session(&owner_store, &names.abandoned).await?;

                    let server = &owner_store.shared.server;
                    let wait_started = server.observe_next_candidate_wait().await;
                    let started = Instant::now();
                    let (abandoned, released) =
                        tokio::join!(candidate.abandon_exact(&target), async {
                            tokio::time::timeout(TEST_DEADLINE, wait_started)
                                .await
                                .context("the owner did not observe the lingering session")?
                                .context("the owner's session-wait observer was dropped")?;
                            // Queued only now, while the owner holds the
                            // status ref's admission across its wait; granted
                            // when the owner releases it.
                            let admission = server.fence_pool(&names.abandoned).await?;
                            let released = started.elapsed();
                            drop(admission);
                            Ok::<_, anyhow::Error>(released)
                        });
                    let elapsed = started.elapsed();
                    let released = released?;
                    let error = abandoned.expect_err(
                        "managed abandonment deleted a ref under a live server session",
                    );
                    let message = format!("{error:#}");
                    // `ServiceFault::StorageFailed` resolves to the first
                    // message (a complete reply); the client's own reply
                    // deadline to the second (a lost reply).
                    let shape = if message.contains("memory service storage operation failed") {
                        "a complete StorageFailed reply"
                    } else if message.contains("memory service frame read deadline exceeded") {
                        "a reply lost to the client's reply deadline"
                    } else {
                        bail!("managed abandonment failed with neither reply shape: {message}")
                    };
                    eprintln!(
                        "managed abandon returned {shape} after {elapsed:?}; the owner released \
                         the status ref's admission after {released:?}: {error:#}"
                    );
                    ensure!(
                        released >= QUERY_TIMEOUT,
                        "the owner released the status ref's admission after {released:?}, before \
                         its {QUERY_TIMEOUT:?} session bound"
                    );
                    ensure!(
                        message.contains(
                            "memory service write outcome is uncertain; further client mutations \
                             are blocked"
                        ),
                        "the client did not fence {shape}: {message}"
                    );
                    let blocked = memory
                        .put("blocked", &json!(true))
                        .await
                        .expect_err("a fenced client issued another mutation");
                    ensure!(
                        format!("{blocked:#}").contains(
                            "memory service write outcome is uncertain; this client cannot issue \
                             another mutation"
                        ),
                        "the second mutation failed for another reason: {blocked:#}"
                    );

                    // Nothing was decided from the ambiguous wait: the session is
                    // still live, the status ref keeps its exact head, main never
                    // moved and no branch procedure was dispatched (it is recorded
                    // only after the wait). Read after the owner released the
                    // admission, on either reply shape.
                    ensure!(session_count(&owner_store, session.id, &session.database).await? == 1);
                    let heads = candidate_heads(&owner_store.pool, &names).await?;
                    ensure!(
                        heads == BTreeMap::from([(names.abandoned.clone(), target.clone())]),
                        "the bounded cleanup changed candidate refs: {heads:?}"
                    );
                    ensure!(owner_store.revision().await? == base);
                    ensure!(
                        owner_store
                            .shared
                            .uncertain
                            .lock()
                            .expect("uncertain lock")
                            .is_none(),
                        "the bounded cleanup dispatched a branch procedure"
                    );

                    session.end(&owner_store).await?;
                    let recovered = {
                        let _gate = crate::spawn_gate::spawning().await;
                        memory
                            .recover_candidate_transition()
                            .await?
                            .context("the fenced client had no pending transition")?
                    };
                    ensure!(
                        recovered.resolution == crate::CandidateTransitionResolution::Abandoned
                    );
                    ensure!(recovered.candidate.is_none());
                    memory.put("after-proof", &json!(true)).await?;
                    ensure!(memory.get("private").await?.is_none());
                    drop(candidate);
                    memory.close().await?;
                    Ok(names)
                },
                async |served| {
                    served
                        .retire(
                            &options,
                            None,
                            Duration::from_secs(10),
                            "bounded abandon owner did not reap",
                        )
                        .await
                },
            )
            .await?;

        // The retained status authority is resolved state: the next open
        // reclaims it without promoting or reviving the candidate.
        deadline
            .run(async {
                let _gate = crate::spawn_gate::spawning().await;
                let reopened = MemoryStore::open(options.clone()).await?;
                ensure!(candidate_heads(&reopened.pool, &names).await?.is_empty());
                ensure!(reopened.get("private").await?.is_none());
                ensure!(reopened.get("after-proof").await? == Some(json!(true)));
                reopened.close().await
            })
            .await
    }
    .await;
    root.release(outcome)
}
