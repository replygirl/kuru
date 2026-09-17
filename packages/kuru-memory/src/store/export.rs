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
}

/// One application storage row retained by a committed export.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageRecord {
    Message {
        sequence: i64,
        namespace: String,
        role: String,
        content_format: String,
        content: String,
    },
    State {
        key: String,
        value: Value,
    },
}

/// Opaque continuation for an [`ActiveExportSnapshot`].
#[derive(Clone, Debug)]
pub struct ExportCursor {
    snapshot: Uuid,
    phase: Phase,
}

#[derive(Clone, Debug)]
enum Phase {
    Messages(Option<i64>),
    State(Option<Vec<u8>>),
}

/// One bounded export page and its continuation, if another page exists.
#[derive(Debug)]
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
                    _ => None,
                };
                Ok(ExportPage { records, next })
            }
        }
    }

    /// Fail if an application-side renderer did not emit this snapshot exactly.
    pub fn verify_counts(&self, message_count: u64, state_count: u64) -> Result<()> {
        ensure!(
            message_count == self.provenance.message_count
                && state_count == self.provenance.state_count,
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
    let rows = match after {
        Some(after) => {
            let query = if current {
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
            let query = if current {
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
        snapshot.verify_counts(258, 257)?;
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
            StorageRecord::Message { sequence: -2, namespace, role, content_format, content }
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
                StorageRecord::State { .. } => None,
            })
            .collect();
        assert_eq!(sequences.first(), Some(&i64::MIN));
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        let keys: Vec<_> = records
            .iter()
            .filter_map(|record| match record {
                StorageRecord::Message { .. } => None,
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
        let messages = empty.page(None).await?;
        assert!(messages.records.is_empty());
        let state = empty.page(messages.next).await?;
        assert!(state.records.is_empty());
        assert!(state.next.is_none());
        empty.verify_counts(0, 0)?;
        store.append("export", "note", "one").await?;
        let first = store.begin_active_export().await?;
        let second = store.begin_active_export().await?;
        let cursor = first.page(None).await?.next.expect("message cursor");
        let error = second.page(Some(cursor)).await.expect_err("foreign cursor");
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
        assert_eq!(snapshot.provenance().schema_version, 3);
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
        snapshot.verify_counts(2, 0)?;
        store.close().await?;
        Ok(())
    }
}
