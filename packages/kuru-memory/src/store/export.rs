use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{MySqlPool, Row};
use uuid::Uuid;

use super::{MemoryStore, QUERY_TIMEOUT, Shared, decode_message, migrations, revision};

const PAGE_SIZE: i64 = 256;

/// Immutable provenance for one committed active-memory export.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportProvenance {
    pub project_scope: String,
    pub branch: String,
    pub revision: String,
    pub schema_version: i32,
    pub message_count: u64,
    pub state_count: u64,
    pub context_summary_count: u64,
    pub context_cursor_count: u64,
    pub session_catalog_count: u64,
    pub public_turn_count: u64,
}

/// One application storage row retained by a committed export.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageRecord {
    Message {
        sequence: i64,
        namespace: String,
        session_id: Option<String>,
        role: String,
        content_format: String,
        content: String,
    },
    State {
        key: String,
        value: Value,
    },
    ContextSummary {
        summary_id: String,
        record: super::ContextSummaryRecord,
        record_format: String,
    },
    ContextCursor {
        cursor: super::ContextSummaryCursor,
    },
    SessionCatalog {
        record: super::SessionCatalogRecord,
    },
    PublicTurn {
        record: super::PublicTurnRecord,
    },
}

/// Opaque continuation for an [`ActiveExportSnapshot`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExportCursor {
    snapshot: Uuid,
    phase: Phase,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum Phase {
    Messages(Option<i64>),
    State(Option<Vec<u8>>),
    ContextSummaries(Option<String>),
    ContextCursors(Option<(Vec<u8>, Vec<u8>, Vec<u8>)>),
    SessionCatalog(Option<Vec<u8>>),
    PublicTurns(Option<String>),
}

/// One bounded export page and its continuation, if another page exists.
#[derive(Debug, Deserialize, Serialize)]
pub struct ExportPage {
    pub records: Vec<StorageRecord>,
    pub next: Option<ExportCursor>,
}

/// A validated, commit-qualified active-main reader.
#[derive(Clone, Debug)]
pub struct ActiveExportSnapshot {
    _shared: Arc<Shared>,
    pool: Arc<MySqlPool>,
    provenance: ExportProvenance,
    id: Uuid,
}

impl MemoryStore {
    /// Capture one committed active-main reader for complete application export.
    pub async fn begin_active_export(&self) -> Result<ActiveExportSnapshot> {
        self.readable()?;
        ensure!(
            self.branch == "main",
            "memory export requires the active main branch"
        );
        let captured_revision = revision(&self.pool).await?;
        let pool = self.shared.server.pool(&captured_revision).await?;
        ensure!(
            revision(&pool).await? == captured_revision,
            "commit-qualified export pool did not resolve to the captured revision"
        );
        let schema_version = migrations::validate_historical(&pool).await?;
        let message_count = count(&pool, "messages").await?;
        let state_count = count(&pool, "state").await?;
        let context_summary_count = if schema_version >= 5 {
            count(&pool, "context_summaries").await?
        } else {
            0
        };
        let context_cursor_count = if schema_version >= 5 {
            count(&pool, "context_summary_cursors").await?
        } else {
            0
        };
        let session_catalog_count = if schema_version >= 7 {
            count(&pool, "session_catalog").await?
        } else {
            0
        };
        let public_turn_count = if schema_version >= 7 {
            count(&pool, "session_public_turns").await?
        } else {
            0
        };
        Ok(ActiveExportSnapshot {
            _shared: self.shared.clone(),
            pool,
            provenance: ExportProvenance {
                project_scope: self.shared.project_scope.clone(),
                branch: "main".into(),
                revision: captured_revision,
                schema_version,
                message_count,
                state_count,
                context_summary_count,
                context_cursor_count,
                session_catalog_count,
                public_turn_count,
            },
            id: Uuid::new_v4(),
        })
    }
}

impl ActiveExportSnapshot {
    pub fn provenance(&self) -> &ExportProvenance {
        &self.provenance
    }

    /// Read one page. `None` is the first message position, never a key value.
    pub async fn page(&self, cursor: Option<ExportCursor>) -> Result<ExportPage> {
        self.ensure_open()?;
        let phase = match cursor {
            None => Phase::Messages(None),
            Some(cursor) => {
                ensure!(
                    cursor.snapshot == self.id,
                    "export cursor belongs to a different snapshot"
                );
                cursor.phase
            }
        };
        match phase {
            Phase::Messages(after) => {
                let records = messages(&self.pool, self.provenance.schema_version, after).await?;
                let next_sequence = match records.last() {
                    Some(StorageRecord::Message { sequence, .. })
                        if records.len() == PAGE_SIZE as usize =>
                    {
                        Some(*sequence)
                    }
                    _ => None,
                };
                if let Some(sequence) = next_sequence {
                    return Ok(ExportPage {
                        records,
                        next: Some(self.cursor(Phase::Messages(Some(sequence)))),
                    });
                }
                Ok(ExportPage {
                    records,
                    next: Some(self.cursor(Phase::State(None))),
                })
            }
            Phase::State(after) => {
                let records = state(&self.pool, after).await?;
                let next = match records.last() {
                    Some(StorageRecord::State { key, .. })
                        if records.len() == PAGE_SIZE as usize =>
                    {
                        Some(self.cursor(Phase::State(Some(key.as_bytes().to_vec()))))
                    }
                    _ => Some(self.cursor(Phase::ContextSummaries(None))),
                };
                Ok(ExportPage { records, next })
            }
            Phase::ContextSummaries(after) => {
                if self.provenance.schema_version < 5 {
                    return Ok(ExportPage {
                        records: Vec::new(),
                        next: Some(self.cursor(Phase::ContextCursors(None))),
                    });
                }
                let records =
                    context_summaries(&self.pool, self.provenance.schema_version, after).await?;
                let next = match records.last() {
                    Some(StorageRecord::ContextSummary { summary_id, .. })
                        if records.len() == PAGE_SIZE as usize =>
                    {
                        Some(self.cursor(Phase::ContextSummaries(Some(summary_id.clone()))))
                    }
                    _ => Some(self.cursor(Phase::ContextCursors(None))),
                };
                Ok(ExportPage { records, next })
            }
            Phase::ContextCursors(after) => {
                if self.provenance.schema_version < 5 {
                    return Ok(ExportPage {
                        records: Vec::new(),
                        next: Some(self.cursor(Phase::SessionCatalog(None))),
                    });
                }
                let records = context_cursors(&self.pool, after).await?;
                let next = match records.last() {
                    Some(StorageRecord::ContextCursor { cursor })
                        if records.len() == PAGE_SIZE as usize =>
                    {
                        Some(self.cursor(Phase::ContextCursors(Some((
                            cursor.actor_namespace.as_bytes().to_vec(),
                            cursor.session_id.as_bytes().to_vec(),
                            cursor.source_namespace.as_bytes().to_vec(),
                        )))))
                    }
                    _ => Some(self.cursor(Phase::SessionCatalog(None))),
                };
                Ok(ExportPage { records, next })
            }
            Phase::SessionCatalog(after) => {
                if self.provenance.schema_version < 7 {
                    return Ok(ExportPage {
                        records: Vec::new(),
                        next: Some(self.cursor(Phase::PublicTurns(None))),
                    });
                }
                let records = session_catalog(&self.pool, after).await?;
                let next = match records.last() {
                    Some(StorageRecord::SessionCatalog { record })
                        if records.len() == PAGE_SIZE as usize =>
                    {
                        Some(self.cursor(Phase::SessionCatalog(Some(
                            record.session_id.as_bytes().to_vec(),
                        ))))
                    }
                    _ => Some(self.cursor(Phase::PublicTurns(None))),
                };
                Ok(ExportPage { records, next })
            }
            Phase::PublicTurns(after) => {
                if self.provenance.schema_version < 7 {
                    return Ok(ExportPage {
                        records: Vec::new(),
                        next: None,
                    });
                }
                let records = public_turns(&self.pool, after).await?;
                let next = match records.last() {
                    Some(StorageRecord::PublicTurn { record })
                        if records.len() == PAGE_SIZE as usize =>
                    {
                        Some(self.cursor(Phase::PublicTurns(Some(record.node_id.clone()))))
                    }
                    _ => None,
                };
                Ok(ExportPage { records, next })
            }
        }
    }

    /// Fail if an application-side renderer did not emit this snapshot exactly.
    pub fn verify_counts(
        &self,
        message_count: u64,
        state_count: u64,
        context_summary_count: u64,
        context_cursor_count: u64,
        session_catalog_count: u64,
        public_turn_count: u64,
    ) -> Result<()> {
        ensure!(
            message_count == self.provenance.message_count
                && state_count == self.provenance.state_count
                && context_summary_count == self.provenance.context_summary_count
                && context_cursor_count == self.provenance.context_cursor_count
                && session_catalog_count == self.provenance.session_catalog_count
                && public_turn_count == self.provenance.public_turn_count,
            "export records do not match captured committed counts"
        );
        Ok(())
    }

    fn cursor(&self, phase: Phase) -> ExportCursor {
        ExportCursor {
            snapshot: self.id,
            phase,
        }
    }

    fn ensure_open(&self) -> Result<()> {
        ensure!(!self.pool.is_closed(), "memory store is closed");
        Ok(())
    }
}

async fn count(pool: &MySqlPool, table: &'static str) -> Result<u64> {
    let query = match table {
        "messages" => "SELECT COUNT(*) FROM messages",
        "state" => "SELECT COUNT(*) FROM state",
        "context_summaries" => "SELECT COUNT(*) FROM context_summaries",
        "context_summary_cursors" => "SELECT COUNT(*) FROM context_summary_cursors",
        "session_catalog" => "SELECT COUNT(*) FROM session_catalog",
        "session_public_turns" => "SELECT COUNT(*) FROM session_public_turns",
        _ => unreachable!("export registry is fixed"),
    };
    let count: i64 = tokio::time::timeout(QUERY_TIMEOUT, sqlx::query_scalar(query).fetch_one(pool))
        .await
        .context("export count deadline exceeded")??;
    u64::try_from(count).context("export count is negative")
}

async fn messages(
    pool: &MySqlPool,
    schema_version: i32,
    after: Option<i64>,
) -> Result<Vec<StorageRecord>> {
    let current = schema_version >= 3;
    let session_provenance = schema_version >= 5;
    let rows = match after {
        Some(after) => {
            let query = if session_provenance {
                "SELECT sequence, namespace, session_id, role, content_format, content FROM messages WHERE sequence > ? ORDER BY sequence LIMIT ?"
            } else if current {
                "SELECT sequence, namespace, role, content_format, content FROM messages WHERE sequence > ? ORDER BY sequence LIMIT ?"
            } else {
                "SELECT sequence, namespace, role, content FROM messages WHERE sequence > ? ORDER BY sequence LIMIT ?"
            };
            tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query(query)
                .bind(after)
                .bind(PAGE_SIZE)
                .fetch_all(pool),
            )
            .await
        }
        None => {
            let query = if session_provenance {
                "SELECT sequence, namespace, session_id, role, content_format, content FROM messages ORDER BY sequence LIMIT ?"
            } else if current {
                "SELECT sequence, namespace, role, content_format, content FROM messages ORDER BY sequence LIMIT ?"
            } else {
                "SELECT sequence, namespace, role, content FROM messages ORDER BY sequence LIMIT ?"
            };
            tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query(query)
                .bind(PAGE_SIZE)
                .fetch_all(pool),
            )
            .await
        }
    }
    .context("export message page deadline exceeded")??;
    rows.into_iter()
        .map(|row| {
            let sequence: i64 = row.try_get("sequence")?;
            let role = String::from_utf8(row.try_get("role")?)
                .context("export message role is not UTF-8")?;
            let content_format: String = if current {
                row.try_get("content_format")?
            } else {
                "text-v1".into()
            };
            let content: String = row.try_get("content")?;
            decode_message(role.clone(), &content_format, &content)
                .with_context(|| format!("invalid export message sequence {sequence}"))?;
            Ok(StorageRecord::Message {
                sequence,
                namespace: String::from_utf8(row.try_get("namespace")?)
                    .context("export message namespace is not UTF-8")?,
                session_id: if session_provenance {
                    row.try_get::<Option<Vec<u8>>, _>("session_id")?
                        .map(String::from_utf8)
                        .transpose()
                        .context("export message session identity is not UTF-8")?
                } else {
                    None
                },
                role,
                content_format,
                content,
            })
        })
        .collect()
}

async fn state(pool: &MySqlPool, after: Option<Vec<u8>>) -> Result<Vec<StorageRecord>> {
    let rows = match after {
        Some(after) => {
            tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query(
                    "SELECT `key`, value FROM state WHERE `key` > ? ORDER BY `key` LIMIT ?",
                )
                .bind(after)
                .bind(PAGE_SIZE)
                .fetch_all(pool),
            )
            .await
        }
        None => {
            tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query("SELECT `key`, value FROM state ORDER BY `key` LIMIT ?")
                    .bind(PAGE_SIZE)
                    .fetch_all(pool),
            )
            .await
        }
    }
    .context("export state page deadline exceeded")??;
    rows.into_iter()
        .map(|row| {
            let key =
                String::from_utf8(row.try_get("key")?).context("export state key is not UTF-8")?;
            let raw: String = row.try_get("value")?;
            Ok(StorageRecord::State {
                key,
                value: serde_json::from_str(&raw).context("export state contains invalid JSON")?,
            })
        })
        .collect()
}

async fn context_summaries(
    pool: &MySqlPool,
    schema_version: i32,
    after: Option<String>,
) -> Result<Vec<StorageRecord>> {
    let projection = if schema_version >= 6 {
        "SELECT summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, operation_id, producer_actor_id, invocation_id, record_format, summary FROM context_summaries"
    } else {
        "SELECT summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, NULL AS operation_id, NULL AS producer_actor_id, invocation_id, record_format, summary FROM context_summaries"
    };
    let after_query = format!("{projection} WHERE summary_id > ? ORDER BY summary_id LIMIT ?");
    let first_query = format!("{projection} ORDER BY summary_id LIMIT ?");
    let rows = match after {
        Some(after) => sqlx::query(sqlx::AssertSqlSafe(after_query))
            .bind(after)
            .bind(PAGE_SIZE)
            .fetch_all(pool),
        None => sqlx::query(sqlx::AssertSqlSafe(first_query))
            .bind(PAGE_SIZE)
            .fetch_all(pool),
    };
    let rows = tokio::time::timeout(QUERY_TIMEOUT, rows)
        .await
        .context("export context summary page deadline exceeded")??;
    rows.into_iter()
        .map(|row| {
            let utf8 = |column| -> Result<String> {
                String::from_utf8(row.try_get(column)?)
                    .with_context(|| format!("export context summary {column} is not UTF-8"))
            };
            let optional_utf8 = |column| -> Result<Option<String>> {
                row.try_get::<Option<Vec<u8>>, _>(column)?
                    .map(String::from_utf8)
                    .transpose()
                    .with_context(|| format!("export context summary {column} is not UTF-8"))
            };
            let summary_id: String = row.try_get("summary_id")?;
            ensure!(
                summary_id.len() == 64
                    && summary_id
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "export context summary identity is malformed"
            );
            let record = super::ContextSummaryRecord {
                actor_namespace: utf8("actor_namespace")?,
                session_id: utf8("session_id")?,
                source_namespace: utf8("source_namespace")?,
                summary_namespace: utf8("summary_namespace")?,
                source_view: row.try_get("source_view")?,
                source_revision: row.try_get("source_revision")?,
                after_sequence: row.try_get("after_sequence")?,
                through_sequence: row.try_get("through_sequence")?,
                turn_id: optional_utf8("turn_id")?,
                operation_id: optional_utf8("operation_id")?,
                producer_actor_id: optional_utf8("producer_actor_id")?,
                invocation_id: utf8("invocation_id")?,
                summary: row.try_get("summary")?,
            };
            super::validate_context_summary(&record)?;
            let record_format: String = row.try_get("record_format")?;
            ensure!(
                record_format == super::context_summary_format(&record)?,
                "export context summary format is unsupported"
            );
            Ok(StorageRecord::ContextSummary {
                summary_id,
                record,
                record_format,
            })
        })
        .collect()
}

async fn context_cursors(
    pool: &MySqlPool,
    after: Option<(Vec<u8>, Vec<u8>, Vec<u8>)>,
) -> Result<Vec<StorageRecord>> {
    let rows = match after {
        Some((actor, session, source)) => {
            sqlx::query("SELECT actor_namespace, session_id, source_namespace, through_sequence, summary_id, source_view, source_revision FROM context_summary_cursors WHERE (actor_namespace, session_id, source_namespace) > (?, ?, ?) ORDER BY actor_namespace, session_id, source_namespace LIMIT ?")
                .bind(actor)
                .bind(session)
                .bind(source)
                .bind(PAGE_SIZE)
                .fetch_all(pool)
        }
        None => {
            sqlx::query("SELECT actor_namespace, session_id, source_namespace, through_sequence, summary_id, source_view, source_revision FROM context_summary_cursors ORDER BY actor_namespace, session_id, source_namespace LIMIT ?")
                .bind(PAGE_SIZE)
                .fetch_all(pool)
        }
    };
    let rows = tokio::time::timeout(QUERY_TIMEOUT, rows)
        .await
        .context("export context cursor page deadline exceeded")??;
    rows.into_iter()
        .map(|row| {
            let utf8 = |column| -> Result<String> {
                String::from_utf8(row.try_get(column)?)
                    .with_context(|| format!("export context cursor {column} is not UTF-8"))
            };
            let cursor = super::ContextSummaryCursor {
                actor_namespace: utf8("actor_namespace")?,
                session_id: utf8("session_id")?,
                source_namespace: utf8("source_namespace")?,
                through_sequence: row.try_get("through_sequence")?,
                summary_id: row.try_get("summary_id")?,
                source_view: row.try_get("source_view")?,
                source_revision: row.try_get("source_revision")?,
            };
            super::validate_context_summary_cursor(&cursor)?;
            Ok(StorageRecord::ContextCursor { cursor })
        })
        .collect()
}

async fn session_catalog(pool: &MySqlPool, after: Option<Vec<u8>>) -> Result<Vec<StorageRecord>> {
    let rows = match after {
        Some(after) => sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id > ? ORDER BY session_id LIMIT ?")
            .bind(after)
            .bind(PAGE_SIZE)
            .fetch_all(pool),
        None => sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog ORDER BY session_id LIMIT ?")
            .bind(PAGE_SIZE)
            .fetch_all(pool),
    };
    tokio::time::timeout(QUERY_TIMEOUT, rows)
        .await
        .context("export session catalog page deadline exceeded")??
        .into_iter()
        .map(|row| {
            Ok(StorageRecord::SessionCatalog {
                record: super::decode_session_catalog_row(&row)?,
            })
        })
        .collect()
}

async fn public_turns(pool: &MySqlPool, after: Option<String>) -> Result<Vec<StorageRecord>> {
    let rows = match after {
        Some(after) => sqlx::query("SELECT node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format FROM session_public_turns WHERE node_id > ? ORDER BY node_id LIMIT ?")
            .bind(after)
            .bind(PAGE_SIZE)
            .fetch_all(pool),
        None => sqlx::query("SELECT node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format FROM session_public_turns ORDER BY node_id LIMIT ?")
            .bind(PAGE_SIZE)
            .fetch_all(pool),
    };
    tokio::time::timeout(QUERY_TIMEOUT, rows)
        .await
        .context("export public turn page deadline exceeded")??
        .into_iter()
        .map(|row| {
            Ok(StorageRecord::PublicTurn {
                record: super::decode_public_turn_row(&row)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AUTHOR;
    use anyhow::Result;
    use serde_json::json;

    async fn commit_fixture_rows(store: &MemoryStore) -> Result<String> {
        let large = "x".repeat(128 * 1024);
        sqlx::query(
            "INSERT INTO messages (sequence, namespace, role, content) VALUES (?, ?, ?, ?)",
        )
        .bind(i64::MIN)
        .bind(b"export/notes".as_slice())
        .bind(b"note".as_slice())
        .bind("committed-message-minimum")
        .execute(store.pool.as_ref())
        .await?;
        sqlx::query(
            "INSERT INTO messages (sequence, namespace, role, content) VALUES (?, ?, ?, ?)",
        )
        .bind(-2_i64)
        .bind(b"export/unknown-namespace".as_slice())
        .bind(b"legacy/unknown-role".as_slice())
        .bind(&large)
        .execute(store.pool.as_ref())
        .await?;
        for index in 0..256 {
            sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                .bind(b"export/notes".as_slice())
                .bind(b"note".as_slice())
                .bind(format!("committed-message-{index}"))
                .execute(store.pool.as_ref())
                .await?;
        }
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(b"export/unknown-state".as_slice())
            .bind(r#"{"unknown":{"kept":true}}"#)
            .execute(store.pool.as_ref())
            .await?;
        for index in 0..256 {
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(format!("state/page-{index:03}").into_bytes())
                .bind("null")
                .execute(store.pool.as_ref())
                .await?;
        }
        sqlx::query("CALL DOLT_COMMIT('-Am', 'export fixture rows', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(store.pool.as_ref())
            .await?;
        Ok(large)
    }

    #[tokio::test]
    async fn active_export_is_revision_pinned_paged_and_excludes_later_or_candidate_rows()
    -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let large = commit_fixture_rows(&store).await?;
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session("export-session").await?;
        ledger
            .admit(kuru_core::InvocationStart {
                session_id: "export-session".into(),
                invocation_id: "export-invocation".into(),
                operation_id: "export-fixture".into(),
                phase: kuru_core::UsagePhase::Speak,
                actor_id: "fixture".into(),
                route: "fixture".into(),
                model: "fixture".into(),
                price_at_invocation: None,
            })
            .await?;
        drop(ledger);
        let snapshot = store.begin_active_export().await?;
        assert_eq!(snapshot.provenance().branch, "main");
        assert_eq!(
            snapshot.provenance().schema_version,
            migrations::CURRENT_VERSION
        );
        assert_eq!(snapshot.provenance().message_count, 258);
        assert_eq!(snapshot.provenance().state_count, 257);

        store
            .append("export/notes", "note", "main-after-snapshot")
            .await?;
        let candidate = store.begin_candidate("private export candidate").await?;
        candidate
            .view()
            .append("export/notes", "dream", "candidate-after-snapshot")
            .await?;
        sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
            .bind(b"export/notes".as_slice())
            .bind(b"note".as_slice())
            .bind("dirty-after-snapshot")
            .execute(store.pool.as_ref())
            .await?;

        let mut cursor = None;
        let mut records = Vec::new();
        let mut pages = 0;
        loop {
            let page = snapshot.page(cursor).await?;
            pages += 1;
            records.extend(page.records);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert!(pages >= 4, "messages and state must use distinct pages");
        snapshot.verify_counts(258, 257, 0, 0, 0, 0)?;
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, StorageRecord::Message { .. }))
                .count(),
            258
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record, StorageRecord::State { .. }))
                .count(),
            257
        );
        assert!(records.iter().any(|record| matches!(
            record,
            StorageRecord::Message { sequence, content, .. }
                if *sequence == i64::MIN && content == "committed-message-minimum"
        )));
        assert!(records.iter().any(|record| matches!(
            record,
            StorageRecord::Message { sequence: -2, namespace, role, content_format, content, .. }
                if namespace == "export/unknown-namespace"
                    && role == "legacy/unknown-role"
                    && content_format == "text-v1"
                    && content == &large
        )));
        assert!(records.iter().any(|record| matches!(
            record,
            StorageRecord::State { key, value }
                if key == "export/unknown-state" && value == &json!({"unknown":{"kept":true}})
        )));
        let sequences: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                StorageRecord::Message { sequence, .. } => Some(*sequence),
                StorageRecord::State { .. }
                | StorageRecord::ContextSummary { .. }
                | StorageRecord::ContextCursor { .. }
                | StorageRecord::SessionCatalog { .. }
                | StorageRecord::PublicTurn { .. } => None,
            })
            .collect();
        assert_eq!(sequences.first(), Some(&i64::MIN));
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        let keys: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                StorageRecord::Message { .. }
                | StorageRecord::ContextSummary { .. }
                | StorageRecord::ContextCursor { .. }
                | StorageRecord::SessionCatalog { .. }
                | StorageRecord::PublicTurn { .. } => None,
                StorageRecord::State { key, .. } => Some(key.clone()),
            })
            .collect();
        let mut expected_keys = vec!["export/unknown-state".to_owned()];
        expected_keys.extend((0..256).map(|index| format!("state/page-{index:03}")));
        assert_eq!(
            keys, expected_keys,
            "state keyset order crossed a page boundary"
        );
        let rendered = serde_json::to_string(&records)?;
        for omitted in [
            "main-after-snapshot",
            "candidate-after-snapshot",
            "dirty-after-snapshot",
            "kuru.usage.v1",
            "export-invocation",
        ] {
            assert!(!rendered.contains(omitted), "snapshot included {omitted}");
        }
        assert_ne!(store.revision().await?, snapshot.provenance().revision);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn export_cursor_cannot_cross_committed_snapshots() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let empty = store.begin_active_export().await?;
        let first = empty.page(None).await?;
        assert!(first.records.is_empty());
        let foreign_cursor = first.next.clone().context("empty export cursor")?;
        let mut cursor = first.next;
        let mut empty_pages = 0;
        while let Some(next) = cursor {
            assert!(
                empty_pages < 5,
                "empty export must terminate after all phases"
            );
            let page = empty.page(Some(next)).await?;
            assert!(page.records.is_empty());
            cursor = page.next;
            empty_pages += 1;
        }
        assert_eq!(empty_pages, 5, "empty v7 export must visit every phase");
        empty.verify_counts(0, 0, 0, 0, 0, 0)?;
        store.append("export", "note", "one").await?;
        let first = store.begin_active_export().await?;
        let second = store.begin_active_export().await?;
        let _ = first;
        let error = second
            .page(Some(foreign_cursor))
            .await
            .expect_err("foreign cursor");
        assert!(error.to_string().contains("different snapshot"));
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn export_rejects_malformed_committed_storage_instead_of_omitting_it() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(b"export/malformed".as_slice())
            .bind("{not-json")
            .execute(store.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'malformed export fixture', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(store.pool.as_ref())
            .await?;
        let snapshot = store.begin_active_export().await?;
        let state = snapshot.page(snapshot.page(None).await?.next).await;
        let error = state.expect_err("malformed state must fail export");
        assert!(error.to_string().contains("invalid JSON"));
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn export_keeps_legacy_and_typed_message_formats_distinct() -> Result<()> {
        use kuru_core::{ContentBlock, Message};

        let store = MemoryStore::temporary().await?;
        let literal = r#"{"blocks":[{"type":"text","text":"literal JSON"}]}"#;
        store.append("export/mixed", "user", literal).await?;
        let typed = Message {
            role: "assistant".into(),
            blocks: vec![
                ContentBlock::Text {
                    text: "before".into(),
                },
                ContentBlock::ToolUse {
                    id: "tool-7".into(),
                    name: "file_read".into(),
                    arguments: json!({"path":"README.md"}),
                },
            ],
        };
        store.append_message("export/mixed", &typed).await?;
        let snapshot = store.begin_active_export().await?;
        assert_eq!(
            snapshot.provenance().schema_version,
            migrations::CURRENT_VERSION
        );
        let records = snapshot.page(None).await?.records;
        assert_eq!(records.len(), 2);
        assert!(matches!(&records[0], StorageRecord::Message {
            role, content_format, content, ..
        } if role == "user" && content_format == "text-v1" && content == literal));
        assert!(matches!(&records[1], StorageRecord::Message {
            role, content_format, content, ..
        } if role == "assistant"
            && content_format == "typed-v1"
            && serde_json::from_str::<Value>(content).ok()
                == Some(json!({"blocks": typed.blocks.clone()}))));
        snapshot.verify_counts(2, 0, 0, 0, 0, 0)?;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn full_fidelity_export_retains_producer_private_reasoning_summaries() -> Result<()> {
        use crate::ReasoningSummaryRecord;

        let store = MemoryStore::temporary().await?;
        store
            .put_reasoning_summaries(&[ReasoningSummaryRecord {
                session_id: "session-private".into(),
                turn_id: Some("turn-private".into()),
                operation_id: None,
                actor_id: "actor-private".into(),
                invocation_id: "invocation-private".into(),
                item_id: Some("provider-item-private".into()),
                output_index: Some(7),
                summary_index: 2,
                text: "producer-only summary".into(),
            }])
            .await?;

        let snapshot = store.begin_active_export().await?;
        let messages = snapshot.page(None).await?;
        assert!(messages.records.is_empty());
        let state = snapshot.page(messages.next).await?;
        assert!(
            matches!(state.records.as_slice(), [StorageRecord::State { key, value }]
            if key.starts_with("kuru/private/reasoning-summary/v1/")
                && value["text"] == "producer-only summary")
        );
        snapshot.verify_counts(0, 1, 0, 0, 0, 0)?;
        store.close().await?;
        Ok(())
    }
}
