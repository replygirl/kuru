//! Pinned-engine inputs to the checked private-candidate reconciliation seam.

use super::*;
use crate as kuru_memory;
use futures::FutureExt;
use serde_json::json;

#[test]
fn candidate_reconciliation_fixtures_use_the_closing_scope() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    crate::test_support::assert_async_tests_run_in_closing(
        &root,
        &[root.join("store/candidate_reconciliation_tests.rs")],
    );
}

async fn fixture() -> Result<MemoryStore> {
    let store = MemoryStore::temporary().await?;
    crate::test_support::closing::register(store.server_for_teardown());
    Ok(store)
}

async fn native_merge(candidate: &Candidate, live: &str) -> Result<(bool, i64)> {
    let view = candidate.view();
    let deadline = write_deadline();
    let (mut connection, id) = owned_connection(&view.pool, deadline).await?;
    let result = crate::pool::within_until(deadline, async {
        let rows = sqlx::query("CALL DOLT_MERGE(?)")
            .bind(live)
            .fetch_all(&mut connection)
            .await?;
        let row = rows.first().context("merge returned no outcome")?;
        Ok::<_, anyhow::Error>((
            row.try_get::<i64, _>("fast_forward")? != 0,
            row.try_get::<i64, _>("conflicts")?,
        ))
    })
    .await;
    // The native default refuses conflicting commit. Roll back its SQL
    // transaction before releasing the selected branch session; no force,
    // conflict sysvar or extra commit changes the observed engine policy.
    if !matches!(result, Ok(Ok((_, 0)))) {
        crate::pool::within_until(deadline, sqlx::query("ROLLBACK").execute(&mut connection))
            .await??;
    }
    drop(connection);
    await_session_end(&view.pool, id, QUERY_TIMEOUT).await?;
    result
        .context("native merge exceeded its existing write budget")?
        .context("native DOLT_MERGE selected exact live revision")
}

async fn parents(store: &MemoryStore, head: &str) -> Result<Vec<(i64, String)>> {
    let mut parents = Vec::new();
    // The pinned broad commit-hash query failed with max1Row for a merge.
    // Exact parent-index queries work without scanning history; separately
    // prove the complete parent cardinality.
    for index in 0..2 {
        let rows: Vec<(i64, String)> = crate::pool::within(
            QUERY_TIMEOUT,
            sqlx::query_as(
                "SELECT parent_index, parent_hash FROM dolt_commit_ancestors \
                 WHERE commit_hash = ? AND parent_index = ? LIMIT 2",
            )
            .bind(head)
            .bind(index)
            .fetch_all(store.pool.as_ref()),
        )
        .await
        .context("indexed candidate parent query budget")?
        .context("indexed candidate parent query")?;
        ensure!(rows.len() <= 1, "duplicate candidate parent index");
        parents.extend(rows);
    }
    let extra: Vec<i64> = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar(
            "SELECT parent_index FROM dolt_commit_ancestors \
             WHERE commit_hash = ? AND (parent_index < 0 OR parent_index >= 2) LIMIT 1",
        )
        .bind(head)
        .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("candidate extra-parent query budget")?
    .context("candidate extra-parent query")?;
    ensure!(extra.is_empty(), "unexpected candidate parent cardinality");
    Ok(parents)
}

#[tokio::test]
async fn pinned_candidate_merge_preserves_both_sides_and_ordered_exact_parents() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = fixture().await?;
        store
            .put(
                "project/fixture/ifs/membership",
                &json!({"parts": ["seed"]}),
            )
            .await?;
        let candidate = store.begin_candidate("merge-contract").await?;
        let view = candidate.view();
        view.put_many(&[
            (
                "project/fixture/ifs/membership".into(),
                json!({"parts": ["seed", "dream"]}),
            ),
            ("dream-undo".into(), json!({"parts": ["seed"]})),
        ])
        .await?;
        view.append("shared-dream", "tool", "candidate note")
            .await?;
        let from = view.revision().await?;
        store
            .put_many(&[
                (
                    "project/fixture/ifs/state/report".into(),
                    json!({"activation": 0.9}),
                ),
                (
                    "project/fixture/session/session-a".into(),
                    json!({"turns": 1}),
                ),
            ])
            .await?;
        store
            .append("live-private", "user", "live conversation")
            .await?;
        let live = store.revision().await?;
        assert_eq!(native_merge(&candidate, &live).await?, (false, 0));
        let merged = view.revision().await?;
        assert_ne!(merged, from);
        assert_ne!(merged, live);
        assert_eq!(
            parents(&store, &merged).await?,
            vec![(0, from), (1, live.clone())]
        );
        assert_eq!(store.revision().await?, live);
        assert_eq!(
            view.get("project/fixture/ifs/state/report").await?,
            Some(json!({"activation": 0.9}))
        );
        assert_eq!(
            view.get("dream-undo").await?,
            Some(json!({"parts": ["seed"]}))
        );
        assert_eq!(
            view.get_versioned("project/fixture/ifs/membership")
                .await?
                .unwrap()
                .version,
            1
        );
        assert_eq!(
            view.get_versioned("project/fixture/ifs/state/report")
                .await?
                .unwrap()
                .version,
            0
        );
        assert_eq!(view.history("shared-dream", 10).await?.len(), 1);
        assert_eq!(view.history("live-private", 10).await?.len(), 1);
        assert!(candidate_branch_is_clean(&store, candidate.branch()).await?);
        store.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn pinned_candidate_merge_fast_forward_and_noop_create_no_extra_commit() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = fixture().await?;
        let candidate = store.begin_candidate("ff-contract").await?;
        store.put("live-only", &json!(1)).await?;
        let live = store.revision().await?;
        assert_eq!(native_merge(&candidate, &live).await?, (true, 0));
        assert_eq!(candidate.view().revision().await?, live);
        let before = parents(&store, &live).await?;
        assert_eq!(native_merge(&candidate, &live).await?.1, 0);
        assert_eq!(candidate.view().revision().await?, live);
        assert_eq!(parents(&store, &live).await?, before);
        assert!(candidate_branch_is_clean(&store, candidate.branch()).await?);
        store.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn pinned_candidate_merge_default_conflict_rolls_back_but_identical_cells_merge() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let store = fixture().await?;
        store.put("overlap", &json!(0)).await?;
        let candidate = store.begin_candidate("conflict-contract").await?;
        candidate.view().put("overlap", &json!(1)).await?;
        store.put("overlap", &json!(2)).await?;
        let from = candidate.view().revision().await?;
        let live = store.revision().await?;
        let error = native_merge(&candidate, &live)
            .await
            .expect_err("default merge committed conflicting rows");
        assert!(error.downcast_ref::<sqlx::Error>().is_some(), "{error:#}");
        let tables: Vec<String> = crate::pool::within(
            QUERY_TIMEOUT,
            sqlx::query_scalar(
                "SELECT `table` FROM DOLT_PREVIEW_MERGE_CONFLICTS_SUMMARY(?, ?) \
                 ORDER BY `table` LIMIT 17",
            )
            .bind(&from)
            .bind(&live)
            .fetch_all(store.pool.as_ref()),
        )
        .await
        .context("pinned read-only conflict summary budget")?
        .context("pinned read-only conflict summary")?;
        assert_eq!(tables, vec!["state"]);
        // This table function derives its result columns while planning.
        // Supply only checked immutable hashes, rather than unresolved bind
        // parameters, so its `our_key` schema is available to the planner.
        for revision in [&from, &live] {
            ensure!(
                revision.len() == 32
                    && revision
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte)),
                "invalid immutable preview revision"
            );
        }
        let preview = format!(
            "SELECT COALESCE(our_key, their_key, base_key) \
             FROM DOLT_PREVIEW_MERGE_CONFLICTS('{from}', '{live}', 'state') LIMIT 33"
        );
        let keys: Vec<Vec<u8>> = crate::pool::within(
            QUERY_TIMEOUT,
            sqlx::query_scalar(sqlx::AssertSqlSafe(preview)).fetch_all(store.pool.as_ref()),
        )
        .await
        .context("pinned read-only conflict key budget")?
        .context("pinned read-only conflict key")?;
        assert_eq!(keys, vec![b"overlap".to_vec()]);
        assert_eq!(candidate.view().revision().await?, from);
        assert_eq!(store.revision().await?, live);
        assert_eq!(candidate.view().get("overlap").await?, Some(json!(1)));
        assert_eq!(store.get("overlap").await?, Some(json!(2)));
        assert!(candidate_branch_is_clean(&store, candidate.branch()).await?);

        let equal = store.begin_candidate("identical-contract").await?;
        equal.view().put("overlap", &json!(3)).await?;
        store.put("overlap", &json!(3)).await?;
        let from = equal.view().revision().await?;
        let live = store.revision().await?;
        assert_eq!(native_merge(&equal, &live).await?, (false, 0));
        let merged = equal.view().revision().await?;
        assert_eq!(
            parents(&store, &merged).await?,
            vec![(0, from), (1, live.clone())]
        );
        assert_eq!(store.revision().await?, live);
        assert!(candidate_branch_is_clean(&store, equal.branch()).await?);
        store.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn local_reconciliation_adopts_exact_live_base_before_fast_forward() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = fixture().await?;
        let membership = format!("{}/ifs/membership", store.shared.project_scope);
        store.put(&membership, &json!({"parts": ["seed"]})).await?;
        let candidate = store.begin_candidate("reconcile-local").await?;
        let creation_base = candidate.base().to_owned();
        candidate
            .view()
            .put_many(&[
                (membership.clone(), json!({"parts": ["seed", "dream"]})),
                ("dream-undo".into(), json!({"parts": ["seed"]})),
            ])
            .await?;
        let from = candidate.view().revision().await?;
        store
            .put("live-report", &json!({"activation": 0.7}))
            .await?;
        let live = store.revision().await?;
        let (result, fresh) = candidate.reconcile_with_live(&from, &live).await?;
        let CandidateReconciliationResult::Reconciled { head, base } = result else {
            bail!("ordinary moved-live reconciliation did not commit");
        };
        assert_eq!(base, live);
        let fresh = fresh.context("reconciliation did not return its fresh handle")?;
        assert_eq!(fresh.base(), live);
        assert_eq!(candidate.base(), creation_base);
        assert_eq!(store.revision().await?, live);
        assert_eq!(
            fresh.view().get("live-report").await?,
            Some(json!({"activation": 0.7}))
        );
        assert_eq!(
            fresh.view().get("dream-undo").await?,
            Some(json!({"parts": ["seed"]}))
        );
        assert_eq!(
            candidate_reconciliation::observe(&store, candidate.branch(), &from, &live).await?,
            CandidateReconciliationObservation::Committed { head: head.clone() }
        );
        assert_eq!(fresh.promote_exact(&head).await?, head);
        assert_eq!(
            store.get(&membership).await?,
            Some(json!({"parts": ["seed", "dream"]}))
        );
        store.close().await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn local_reconciliation_refuses_identical_and_unequal_membership_overlap() -> Result<()> {
    kuru_memory::test_support::closing(async {
        for live_value in [json!(1), json!(2)] {
            let store = fixture().await?;
            let membership = format!("{}/ifs/membership", store.shared.project_scope);
            store.put(&membership, &json!(0)).await?;
            let candidate = store.begin_candidate("membership-overlap").await?;
            candidate.view().put(&membership, &json!(1)).await?;
            let from = candidate.view().revision().await?;
            store.put(&membership, &live_value).await?;
            let live = store.revision().await?;
            let (result, fresh) = candidate.reconcile_with_live(&from, &live).await?;
            assert_eq!(
                result,
                CandidateReconciliationResult::Conflict {
                    tables: vec!["state".into()],
                    state_keys: vec![membership.clone()],
                    coordinates_available: true,
                }
            );
            assert!(fresh.is_none());
            assert_eq!(candidate.view().revision().await?, from);
            assert_eq!(store.revision().await?, live);
            assert_eq!(candidate.view().get(&membership).await?, Some(json!(1)));
            assert_eq!(store.get(&membership).await?, Some(live_value));
            assert_eq!(
                candidate_reconciliation::observe(&store, candidate.branch(), &from, &live).await?,
                CandidateReconciliationObservation::NotCommitted
            );
            candidate.abandon_exact(&from).await?;
            store.close().await?;
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn local_reconciliation_checks_exact_inputs_and_private_fast_forward() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = fixture().await?;
        let candidate = store.begin_candidate("reconcile-inputs").await?;
        let from = candidate.view().revision().await?;
        store.put("live-only", &json!(1)).await?;
        let live = store.revision().await?;
        let (result, fresh) = candidate.reconcile_with_live(&from, &from).await?;
        assert_eq!(
            result,
            CandidateReconciliationResult::LiveMoved { head: live.clone() }
        );
        assert!(fresh.is_none());
        assert_eq!(candidate.view().revision().await?, from);
        assert!(
            candidate
                .reconcile_with_live("bad' revision", &live)
                .await
                .is_err()
        );
        assert!(
            candidate
                .reconcile_with_live(&live, &live)
                .await
                .unwrap_err()
                .downcast_ref::<CandidateRefRejected>()
                .is_some()
        );
        let (result, fresh) = candidate.reconcile_with_live(&from, &live).await?;
        assert_eq!(
            result,
            CandidateReconciliationResult::Reconciled {
                head: live.clone(),
                base: live.clone()
            }
        );
        let fresh = fresh.context("private fast-forward returned no fresh handle")?;
        let (unchanged, _) = fresh.reconcile_with_live(&live, &live).await?;
        assert_eq!(
            unchanged,
            CandidateReconciliationResult::Unchanged {
                head: live.clone(),
                base: live.clone()
            }
        );
        assert_eq!(
            candidate_reconciliation::observe(&store, candidate.branch(), &from, &live).await?,
            CandidateReconciliationObservation::Committed { head: live.clone() }
        );
        assert_eq!(
            candidate_reconciliation::observe(&store, candidate.branch(), &live, &live).await?,
            CandidateReconciliationObservation::NotCommitted
        );
        assert_eq!(store.revision().await?, live);
        store.close().await?;
        Ok(())
    })
    .await
}

async fn insert_fixture_message(
    store: &MemoryStore,
    sequence: i64,
    namespace: &str,
    session: Option<&str>,
    content: &str,
    format: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO messages (sequence, namespace, session_id, role, content, content_format) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(sequence).bind(namespace.as_bytes()).bind(session.map(str::as_bytes))
        .bind(b"dream".as_slice()).bind(content).bind(format)
        .execute(store.pool.as_ref()).await.context("controlled fixture INSERT message row")?;
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind("controlled reconciliation fixture row")
        .bind(AUTHOR)
        .fetch_all(store.pool.as_ref())
        .await
        .context("controlled fixture message row commit")?;
    Ok(())
}

#[tokio::test]
async fn local_reconciliation_message_collision_is_explicit_and_preserves_both_histories()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        let store = fixture().await?;
        let candidate = store.begin_candidate("message-collision").await?;
        insert_fixture_message(
            &candidate.view(),
            1_000_000,
            "collision",
            None,
            "PRIVATE_CANDIDATE",
            TEXT_FORMAT,
        )
        .await?;
        insert_fixture_message(
            &store,
            1_000_000,
            "collision",
            None,
            "PRIVATE_LIVE",
            TEXT_FORMAT,
        )
        .await?;
        let from = candidate.view().revision().await?;
        let live = store.revision().await?;
        let (result, fresh) = candidate
            .reconcile_with_live(&from, &live)
            .await
            .context("controlled message-PK collision reconciliation")?;
        let CandidateReconciliationResult::Conflict {
            tables,
            coordinates_available,
            ..
        } = result
        else {
            bail!("conflicting message primary key silently reconciled");
        };
        ensure!(!coordinates_available || tables.iter().any(|table| table == "messages"));
        ensure!(
            fresh.is_none()
                && candidate.view().revision().await? == from
                && store.revision().await? == live
        );
        ensure!(
            candidate.view().history("collision", 10).await?[0].plain_text()
                == Some("PRIVATE_CANDIDATE")
        );
        ensure!(store.history("collision", 10).await?[0].plain_text() == Some("PRIVATE_LIVE"));
        ensure!(candidate_branch_is_clean(&store, candidate.branch()).await?);
        candidate.abandon_exact(&from).await?;
        store.close().await
    })
    .await
}

#[tokio::test]
async fn local_reconciliation_restart_sequence_and_session_cursor_privacy() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let root = crate::test_support::tempdir()?;
        let scope = temporary_scope();
        let actor = format!("{scope}/ifs/identity/actor");
        let options = crate::test_support::warmed_open_options(root.path().join("private"), scope).await?;
        let mut opened = Vec::new();
        let outcome = std::panic::AssertUnwindSafe(async {
            let store = MemoryStore::open(options.clone()).await?;
            opened.push(store.clone());
            crate::test_support::closing::register(store.server_for_teardown());
            store.append_session_message(&actor, "session-a", &Message::text("user", "OWN_FIRST")).await?;
            let id = Uuid::new_v4();
            let candidate = store.begin_candidate_with_id("sequence-restart", id).await?;
            candidate.view().append(&actor, "dream", "UNATTRIBUTED_DREAM_SENTINEL").await?;
            let private_sequence: i64 = sqlx::query_scalar("SELECT MAX(sequence) FROM messages")
                .fetch_one(candidate.view().pool.as_ref()).await?;
            ensure!(sqlx::query_scalar::<_, i64>("SELECT MAX(sequence) FROM messages")
                .fetch_one(store.pool.as_ref()).await? < private_sequence);
            drop(candidate);
            store.close().await?;
            let reopened = MemoryStore::open(options.clone()).await.context("highest-private-sequence fixture owner reopen")?;
            opened.push(reopened.clone());
            crate::test_support::closing::register(reopened.server_for_teardown());
            let CandidateLookup::Open(candidate) = reopened.candidate_for_id(id).await? else {
                bail!("restart discarded highest-sequence open candidate");
            };
            reopened.append_session_message(&actor, "session-a", &Message::text("user", "OWN_LATER")).await?;
            let captured = reopened.session_source_snapshot(&actor, "session-a", &actor, 0, 16).await?;
            let through = captured.through_inclusive.context("own source has no boundary")?;
            ensure!(through > private_sequence, "restarted allocator reused a candidate sequence");
            // Supported opaque tool payloads retain their exact session
            // ownership and never enter another session's raw context.
            reopened.append_session_message(&actor, "session-b", &Message::tool_result(
                "foreign-call", json!({"opaque":"OPAQUE_FOREIGN_SENTINEL"}), false,
            )).await?;
            let from = candidate.view().revision().await?;
            let live = reopened.revision().await?;
            let (result, fresh) = candidate.reconcile_with_live(&from, &live).await.context("restart/session-privacy exact merge")?;
            let CandidateReconciliationResult::Reconciled { head, .. } = result else { bail!("nonoverlapping session histories did not reconcile"); };
            let fresh = fresh.context("merged histories lost fresh handle")?;
            fresh.promote_exact(&head).await?;
            let raw = reopened.session_history_window(&actor, "session-a", 16).await?;
            ensure!(raw.messages.len() == 2 && raw.messages.iter().all(|message| message.text_projection().starts_with("OWN_")));
            let record = ContextSummaryRecord {
                actor_namespace: actor.clone(), session_id: "session-a".into(), source_namespace: actor.clone(),
                summary_namespace: format!("{actor}/session/session-a/summaries"),
                source_view: captured.view, source_revision: captured.revision,
                after_sequence: 0, through_sequence: through, turn_id: Some("turn-own".into()),
                operation_id: None, producer_actor_id: None, invocation_id: "invocation-own".into(), summary: "own session only".into(),
            };
            reopened.checkpoint_context_summary(&ContextSummaryCheckpoint { record, private_reasoning: vec![] }).await?;
            ensure!(reopened.context_summary_cursor(&actor, "session-a", &actor).await?.context("own cursor missing")?.through_sequence == through);
            ensure!(reopened.session_source_snapshot(&actor, "session-a", &actor, 0, 16).await?.rows.len() == 2);
            let export = reopened.begin_active_export().await?;
            let mut cursor = None;
            let mut records = vec![];
            loop {
                let page = export.page(cursor).await?;
                records.extend(page.records);
                cursor = page.next;
                if cursor.is_none() { break; }
            }
            ensure!(records.iter().any(|row| matches!(row, StorageRecord::Message { session_id: None, content, .. } if content.contains("UNATTRIBUTED_DREAM_SENTINEL"))));
            ensure!(records.iter().any(|row| matches!(row, StorageRecord::Message { session_id: Some(session), content_format, content, .. } if session == "session-b" && content_format == TYPED_FORMAT && content.contains("OPAQUE_FOREIGN_SENTINEL"))));
            drop(export);
            reopened.close().await
        }).catch_unwind().await;
        let mut cleanup_errors = Vec::new();
        for store in opened {
            if let Err(error) = store.close().await {
                cleanup_errors.push(format!("fixture owner close: {error:#}"));
            }
        }
        match outcome {
            Ok(result) => root.release(result.and_then(|()| {
                ensure!(cleanup_errors.is_empty(), "{}", cleanup_errors.join("; "));
                Ok(())
            })),
            Err(panic) => {
                let _ = root.release::<()>(Err(anyhow::anyhow!("fixture panicked; cleanup: {}", cleanup_errors.join("; "))));
                std::panic::resume_unwind(panic)
            }
        }
    }).await
}

#[tokio::test]
async fn local_reconciliation_historical_dirty_and_negative_tokens_refuse_before_effects()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        for invalid in ["historical", "dirty", "negative"] {
            let store = fixture().await?;
            let membership = format!("{}/ifs/membership", store.shared.project_scope);
            store.put(&membership, &json!({"parts":[]})).await?;
            let candidate = store.begin_candidate("invalid-selected-view").await?;
            let view = candidate.view();
            view.put("candidate-only", &json!(1)).await?;
            store.put("live-only", &json!(2)).await?;
            match invalid {
                "historical" => {
                    sqlx::query("UPDATE kuru_schema SET version = 9 WHERE id = 1")
                        .execute(view.pool.as_ref())
                        .await?;
                }
                "dirty" => {
                    sqlx::query("UPDATE state SET value = ? WHERE `key` = ?")
                        .bind("99")
                        .bind(b"candidate-only".as_slice())
                        .execute(view.pool.as_ref())
                        .await?;
                }
                "negative" => {
                    sqlx::query("UPDATE state SET version = -1 WHERE `key` = ?")
                        .bind(membership.as_bytes())
                        .execute(view.pool.as_ref())
                        .await?;
                }
                _ => unreachable!(),
            }
            if invalid != "dirty" {
                sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                    .bind("controlled invalid reconciliation input")
                    .bind(AUTHOR)
                    .fetch_all(view.pool.as_ref())
                    .await?;
            }
            let from = view.revision().await?;
            let live = store.revision().await?;
            let error = candidate
                .reconcile_with_live(&from, &live)
                .await
                .expect_err("invalid selected view reached a native merge");
            if invalid == "historical" {
                ensure!(
                    error
                        .downcast_ref::<CandidateRefRejected>()
                        .is_some_and(|error| matches!(
                            error.0,
                            CandidateRefRefusal::SchemaUnverified
                        ))
                );
            }
            ensure!(view.revision().await? == from && store.revision().await? == live);
            ensure!(store.get("candidate-only").await?.is_none());
            ensure!(view.get("live-only").await?.is_none());
            ensure!(
                candidate_branch_is_clean(&store, candidate.branch()).await?
                    == (invalid != "dirty")
            );
            store.close().await?;
        }
        Ok(())
    })
    .await
}
