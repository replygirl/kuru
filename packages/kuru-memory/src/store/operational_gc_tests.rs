use super::*;
use serde_json::json;

const TEST_DEADLINE: Duration = Duration::from_secs(10);

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

    assert_eq!(candidate.promote().await?, target);
    assert_eq!(candidate.promote().await?, target);
    assert_eq!(store.revision().await?, target);
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
    let (held, held_id) = owned_connection(&candidate.view.pool).await?;
    let error = candidate
        .abandon()
        .await
        .expect_err("live candidate session permitted force cleanup");
    assert!(format!("{error:#}").contains("in use"));
    let heads = candidate_heads(&store.pool, &names).await?;
    assert_eq!(heads.get(&names.open), Some(&target));
    assert!(!heads.contains_key(&names.abandoned));
    assert!(store.history("candidate history", 10).await?.is_empty());

    drop(held);
    await_session_end(&store.pool, held_id, TEST_DEADLINE).await?;
    tokio::time::timeout(TEST_DEADLINE, candidate.abandon()).await??;
    assert!(candidate_heads(&store.pool, &names).await?.is_empty());
    store.put("after held view", &json!(true)).await?;
    store.close().await
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
