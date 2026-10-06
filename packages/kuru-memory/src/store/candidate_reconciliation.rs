//! Exact-input private merges. Ordinary main publication remains fast-forward only.

use super::*;
use tokio::time::Instant;

/// A terminal checked reconciliation. Conflict metadata never contains row values.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateReconciliationResult {
    Reconciled {
        head: String,
        base: String,
    },
    Unchanged {
        head: String,
        base: String,
    },
    LiveMoved {
        head: String,
    },
    Conflict {
        tables: Vec<String>,
        state_keys: Vec<String>,
        coordinates_available: bool,
    },
}

/// Exact persisted evidence, independent of a possibly lost native reply.
/// An unchanged ref cannot distinguish a prior refusal from a prior no-op.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateReconciliationObservation {
    Committed { head: String },
    NotCommitted,
    StillUncertain,
}

pub(crate) struct ReconciliationRecovery {
    pub observation: CandidateReconciliationObservation,
    pub candidate: Option<Candidate>,
    pub status: Option<CandidateRefStatus>,
}

pub(super) fn commit(value: &str) -> Result<()> {
    ensure!(
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte)),
        "invalid immutable reconciliation revision"
    );
    Ok(())
}

async fn merge_base(
    pool: &MemoryPool,
    first: &str,
    second: &str,
    deadline: Instant,
) -> Result<String> {
    let base: String = crate::pool::within_until(
        deadline,
        sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
            .bind(first)
            .bind(second)
            .fetch_one(pool),
    )
    .await
    .context("candidate common-base lookup exceeded its budget")??;
    commit(&base)?;
    Ok(base)
}

async fn clean(pool: &MemoryPool, deadline: Instant) -> Result<bool> {
    Ok(crate::pool::within_until(
        deadline,
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status").fetch_one(pool),
    )
    .await
    .context("candidate clean-state lookup exceeded its budget")??
        == 0)
}

async fn parents(pool: &MemoryPool, head: &str, deadline: Instant) -> Result<Vec<(i64, String)>> {
    commit(head)?;
    let mut parents = Vec::new();
    // Dolt 2.3.5's broad commit-hash query failed with max1Row for two parents.
    // Exact-index queries work; separately prove there is no third parent.
    for index in 0..2 {
        let rows: Vec<(i64, String)> = crate::pool::within_until(
            deadline,
            sqlx::query_as(
                "SELECT parent_index, parent_hash FROM dolt_commit_ancestors \
                WHERE commit_hash = ? AND parent_index = ? LIMIT 2",
            )
            .bind(head)
            .bind(index)
            .fetch_all(pool),
        )
        .await
        .context("candidate parent lookup exceeded its budget")??;
        ensure!(rows.len() <= 1, "candidate parent index is ambiguous");
        for (_, parent) in &rows {
            commit(parent)?;
        }
        parents.extend(rows);
    }
    let extra: Vec<i64> = crate::pool::within_until(
        deadline,
        sqlx::query_scalar(
            "SELECT parent_index FROM dolt_commit_ancestors \
            WHERE commit_hash = ? AND (parent_index < 0 OR parent_index >= 2) LIMIT 1",
        )
        .bind(head)
        .fetch_all(pool),
    )
    .await
    .context("candidate parent cardinality lookup exceeded its budget")??;
    ensure!(
        extra.is_empty(),
        "candidate has unexpected parent cardinality"
    );
    Ok(parents)
}

pub(super) async fn observe(
    store: &MemoryStore,
    branch: &str,
    from: &str,
    live: &str,
) -> Result<CandidateReconciliationObservation> {
    commit(from)?;
    commit(live)?;
    let names = CandidateNames::from_open(branch)?;
    let heads = candidate_heads(&store.pool, &names).await?;
    if heads.len() != 1 || !heads.contains_key(branch) {
        return Ok(CandidateReconciliationObservation::StillUncertain);
    }
    let head = &heads[branch];
    commit(head)?;
    let pool = store.shared.server.pool(branch).await?;
    let deadline = write_deadline();
    if !clean(&pool, deadline).await? {
        return Ok(CandidateReconciliationObservation::StillUncertain);
    }
    if head == from {
        return Ok(CandidateReconciliationObservation::NotCommitted);
    }
    let proved = if head == live {
        merge_base(&pool, from, live, deadline).await? == from
    } else {
        parents(&pool, head, deadline).await? == [(0, from.to_owned()), (1, live.to_owned())]
    };
    Ok(if proved {
        CandidateReconciliationObservation::Committed { head: head.clone() }
    } else {
        CandidateReconciliationObservation::StillUncertain
    })
}

async fn schema_at(store: &MemoryStore, at: &str, deadline: Instant) -> Result<()> {
    commit(at)?;
    let schema: i32 = crate::pool::within_until(
        deadline,
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT version FROM kuru_schema AS OF '{at}' WHERE id = 1"
        )))
        .fetch_one(store.pool.as_ref()),
    )
    .await
    .context("candidate historical-schema lookup exceeded its budget")??;
    ensure!(
        schema == migrations::CURRENT_VERSION,
        CandidateRefRejected(CandidateRefRefusal::SchemaUnverified)
    );
    Ok(())
}

async fn membership_tokens(
    store: &MemoryStore,
    at: &str,
    deadline: Instant,
) -> Result<BTreeMap<String, i64>> {
    schema_at(store, at, deadline).await?;
    let mut query = sqlx::QueryBuilder::<sqlx::MySql>::new(format!(
        "SELECT `key`, version FROM state AS OF '{at}' WHERE `key` IN ("
    ));
    let keys: Vec<_> = Mode::ALL
        .iter()
        .map(|mode| format!("{}/{mode}/membership", store.shared.project_scope))
        .collect();
    let mut separated = query.separated(", ");
    for key in &keys {
        separated.push_bind(key.as_bytes());
    }
    separated.push_unseparated(") LIMIT ");
    query.push_bind((keys.len() + 1) as i64);
    let rows: Vec<(Vec<u8>, i64)> = crate::pool::within_until(
        deadline,
        query.build_query_as().fetch_all(store.pool.as_ref()),
    )
    .await
    .context("candidate membership-token lookup exceeded its budget")??;
    ensure!(
        rows.len() <= keys.len(),
        "ambiguous candidate membership tokens"
    );
    rows.into_iter()
        .map(|(key, version)| {
            let key = String::from_utf8(key).context("membership key is not UTF-8")?;
            ensure!(
                keys.contains(&key) && version >= 0,
                "invalid candidate membership token"
            );
            Ok((key, version))
        })
        .collect()
}

fn native_conflict(error: &anyhow::Error) -> bool {
    let Some(sqlx::Error::Database(database)) = error.downcast_ref::<sqlx::Error>() else {
        return false;
    };
    let Some(mysql) = database.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() else {
        return false;
    };
    mysql.number() == 1105 && mysql.code() == Some("HY000") && (
        database.message().starts_with("Merge conflict detected, transaction rolled back.")
        || database.message().starts_with("Merge conflict detected, @autocommit transaction rolled back.")
        || database.message().starts_with("Committing this transaction resulted in a working set with constraint violations, transaction rolled back.")
    )
}

impl MemoryStore {
    /// Call only after the original tuple's worker completion or owner reap is
    /// established. Proof and fresh attachment share the owner's write guard.
    pub(crate) async fn candidate_reconciliation_outcome(
        &self,
        branch: &str,
        from: &str,
        live: &str,
    ) -> Result<ReconciliationRecovery> {
        self.readable()?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let observation = observe(self, branch, from, live).await?;
        let (candidate, status) = match &observation {
            CandidateReconciliationObservation::StillUncertain => (None, None),
            CandidateReconciliationObservation::Committed { head } => {
                schema_at(self, head, write_deadline()).await?;
                let candidate = self
                    .candidate_from_branch(branch.to_owned(), live.to_owned())
                    .await?;
                let current = self.revision().await?;
                let status = CandidateRefStatus {
                    branch: branch.to_owned(),
                    head: Some(head.clone()),
                    base: Some(live.to_owned()),
                    state: if current == live {
                        CandidateRefState::OpenUnchanged
                    } else {
                        CandidateRefState::OpenConflict
                    },
                };
                (Some(candidate), Some(status))
            }
            CandidateReconciliationObservation::NotCommitted => {
                schema_at(self, from, write_deadline()).await?;
                let current = self.revision().await?;
                let base = merge_base(&self.pool, from, &current, write_deadline()).await?;
                let candidate = self
                    .candidate_from_branch(branch.to_owned(), base.clone())
                    .await?;
                let status = CandidateRefStatus {
                    branch: branch.to_owned(),
                    head: Some(from.to_owned()),
                    base: Some(base.clone()),
                    state: if current == base {
                        CandidateRefState::OpenUnchanged
                    } else {
                        CandidateRefState::OpenConflict
                    },
                };
                (Some(candidate), Some(status))
            }
        };
        Ok(ReconciliationRecovery {
            observation,
            candidate,
            status,
        })
    }
}

async fn conflict_coordinates(
    pool: &MemoryPool,
    from: &str,
    live: &str,
    deadline: Instant,
) -> Result<(Vec<String>, Vec<String>)> {
    commit(from)?;
    commit(live)?;
    let tables: Vec<String> = crate::pool::within_until(
        deadline,
        sqlx::query_scalar(
            "SELECT `table` FROM DOLT_PREVIEW_MERGE_CONFLICTS_SUMMARY(?, ?) \
            ORDER BY `table` LIMIT 17",
        )
        .bind(from)
        .bind(live)
        .fetch_all(pool),
    )
    .await
    .context("candidate conflict preview exceeded its budget")??;
    ensure!(
        !tables.is_empty() && tables.len() <= 16,
        "unavailable bounded conflict tables"
    );
    for table in &tables {
        ensure!(
            table.len() <= 128
                && table
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "unavailable conflict table identity"
        );
    }
    let mut keys = Vec::new();
    if tables.iter().any(|table| table == "state") {
        // The dynamic table-function schema requires resolved revisions while
        // planning. Only checked immutable hashes enter this fixed projection.
        let sql = format!(
            "SELECT COALESCE(our_key, their_key, base_key) \
            FROM DOLT_PREVIEW_MERGE_CONFLICTS('{from}', '{live}', 'state') LIMIT 33"
        );
        let rows: Vec<Vec<u8>> = crate::pool::within_until(
            deadline,
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).fetch_all(pool),
        )
        .await
        .context("candidate conflict key preview exceeded its budget")??;
        ensure!(rows.len() <= 32, "unavailable bounded conflict keys");
        for key in rows {
            let key = String::from_utf8(key).context("unavailable conflict key encoding")?;
            session_identity("candidate conflict key", &key, 1024)?;
            keys.push(key);
        }
        keys.sort();
        keys.dedup();
    }
    Ok((tables, keys))
}

impl Candidate {
    pub(crate) async fn reconciliation_outcome(
        &self,
        from: &str,
        live: &str,
    ) -> Result<ReconciliationRecovery> {
        self.live
            .candidate_reconciliation_outcome(self.branch(), from, live)
            .await
    }

    /// Reconcile one exact private head with one caller-captured live revision.
    /// Return a fresh checked handle only after the effective base is proved.
    pub(crate) async fn reconcile_with_live(
        &self,
        expected_head: &str,
        expected_live: &str,
    ) -> Result<(CandidateReconciliationResult, Option<Candidate>)> {
        commit(expected_head)?;
        commit(expected_live)?;
        self.live.writable()?;
        let guard = self.live.shared.write.clone().lock_owned().await;
        self.live.resolve_uncertain().await?;
        let worker = self.live.clone();
        let branch = self.view.branch.clone();
        let from = expected_head.to_owned();
        let live = expected_live.to_owned();
        tokio::spawn(async move {
            let _guard = guard;
            let deadline = write_deadline();
            let names = CandidateNames::from_open(&branch)?;
            let heads = candidate_heads(&worker.pool, &names).await?;
            ensure!(
                heads.len() == 1 && heads.get(&branch) == Some(&from),
                CandidateRefRejected(CandidateRefRefusal::Changed)
            );
            let pool = worker.shared.server.pool(&branch).await?;
            ensure!(
                clean(&pool, deadline).await?,
                CandidateRefRejected(CandidateRefRefusal::Changed)
            );
            let current = worker.revision().await?;
            if current != live {
                return Ok((
                    CandidateReconciliationResult::LiveMoved { head: current },
                    None,
                ));
            }
            let base = merge_base(&worker.pool, &from, &live, deadline).await?;
            let old = membership_tokens(&worker, &base, deadline).await?;
            let candidate = membership_tokens(&worker, &from, deadline).await?;
            let latest = membership_tokens(&worker, &live, deadline).await?;
            let overlapping: Vec<_> = candidate
                .keys()
                .chain(old.keys())
                .filter(|key| {
                    candidate.get(*key) != old.get(*key) && latest.get(*key) != old.get(*key)
                })
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if !overlapping.is_empty() {
                return Ok((
                    CandidateReconciliationResult::Conflict {
                        tables: vec!["state".into()],
                        state_keys: overlapping,
                        coordinates_available: true,
                    },
                    None,
                ));
            }
            if base == live {
                let fresh = worker.candidate_from_branch(branch, live.clone()).await?;
                return Ok((
                    CandidateReconciliationResult::Unchanged {
                        head: from,
                        base: live,
                    },
                    Some(fresh),
                ));
            }
            let (mut connection, id) = owned_connection(&pool, deadline).await?;
            *worker.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: pool.clone(),
                connection: id,
                receipt: Receipt::CandidateReconciliation {
                    branch: branch.clone(),
                    from: from.clone(),
                    live: live.clone(),
                },
            });
            let result = crate::pool::within_until(
                deadline,
                sqlx::query("CALL DOLT_MERGE(?)")
                    .bind(&live)
                    .fetch_all(&mut connection),
            )
            .await
            .context("candidate merge exceeded its existing budget")
            .and_then(|result| result.map_err(anyhow::Error::from));
            if result.is_err() {
                // Refusal cleanup precedes proof; a failed rollback is not a
                // definite conflict until the original session has ended.
                let _ = crate::pool::within_until(
                    deadline,
                    sqlx::query("ROLLBACK").execute(&mut connection),
                )
                .await;
            }
            drop(connection);
            let committed = worker.resolve_uncertain().await? == Some(true);
            if committed {
                let CandidateReconciliationObservation::Committed { head } =
                    observe(&worker, &branch, &from, &live).await?
                else {
                    bail!("candidate reconciliation lost its exact committed proof");
                };
                let fresh = worker.candidate_from_branch(branch, live.clone()).await?;
                return Ok((
                    CandidateReconciliationResult::Reconciled { head, base: live },
                    Some(fresh),
                ));
            }
            if let Err(error) = result {
                if native_conflict(&error) {
                    ensure!(
                        worker.revision().await? == live,
                        "live changed while the native refusal settled"
                    );
                    let coordinates =
                        conflict_coordinates(&worker.pool, &from, &live, deadline).await;
                    let available = coordinates.is_ok();
                    let (tables, state_keys) = coordinates.unwrap_or_default();
                    return Ok((
                        CandidateReconciliationResult::Conflict {
                            tables,
                            state_keys,
                            coordinates_available: available,
                        },
                        None,
                    ));
                }
                return Err(error);
            }
            bail!("candidate merge did not retain its exact reconciliation outcome")
        })
        .await
        .context("candidate reconciliation worker failed")?
    }
}
