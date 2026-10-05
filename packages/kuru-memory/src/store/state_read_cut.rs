//! Immutable selected-view state reads. A cut owns a reference to a checked
//! revision pool, never the mutable view's transaction or writer lease.

use super::*;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StateReadProvenance {
    pub project_scope: String,
    pub branch: String,
    pub revision: String,
    pub schema_version: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StateReadCursor {
    snapshot: Uuid,
    prefix: String,
    after: Vec<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StateReadPage {
    pub values: Vec<(String, VersionedValue)>,
    pub next: Option<StateReadCursor>,
}

struct Reader {
    _shared: Arc<Shared>,
    pool: Arc<MemoryPool>,
}

struct Cut {
    provenance: StateReadProvenance,
    id: Uuid,
    reader: RwLock<Option<Reader>>,
}

#[derive(Clone)]
pub struct StateReadCut(Arc<Cut>);

impl std::fmt::Debug for StateReadCut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateReadCut")
            .field("provenance", self.provenance())
            .finish()
    }
}

impl MemoryStore {
    pub async fn begin_state_read_cut(&self) -> Result<StateReadCut> {
        self.readable()?;
        let captured = revision(&self.pool).await?;
        self.state_read_cut_at(&self.branch, captured).await
    }

    pub(crate) async fn candidate_state_read_cut(
        &self,
        branch: &str,
        captured: String,
    ) -> Result<StateReadCut> {
        let status = self.candidate_ref_status(branch).await?;
        ensure!(
            matches!(
                status.state,
                CandidateRefState::OpenUnchanged | CandidateRefState::OpenConflict
            ) && status.head.as_ref() == Some(&captured),
            "candidate state cut selection changed"
        );
        self.state_read_cut_at(branch, captured).await
    }

    async fn state_read_cut_at(&self, branch: &str, captured: String) -> Result<StateReadCut> {
        let pool = self.shared.server.pool(&captured).await?;
        ensure!(
            revision(&pool).await? == captured,
            "state cut pool did not resolve to its captured revision"
        );
        let schema_version = migrations::validate_historical(&pool).await?;
        ensure!(
            schema_version >= migrations::STATE_VERSION,
            "state cut requires an upgraded memory view"
        );
        Ok(StateReadCut(Arc::new(Cut {
            provenance: StateReadProvenance {
                project_scope: self.shared.project_scope.clone(),
                branch: branch.to_owned(),
                revision: captured,
                schema_version,
            },
            id: Uuid::new_v4(),
            reader: RwLock::new(Some(Reader {
                _shared: self.shared.clone(),
                pool,
            })),
        })))
    }
}

impl StateReadCut {
    pub fn provenance(&self) -> &StateReadProvenance {
        &self.0.provenance
    }

    pub async fn get_versioned(&self, key: &str) -> Result<Option<VersionedValue>> {
        identifier("state key", key, 1024)?;
        let retained = self.0.reader.read().await;
        let reader = retained.as_ref().context("state cut is closed")?;
        ensure!(!reader.pool.is_closed(), "state cut owner is closed");
        let value = crate::pool::within(QUERY_TIMEOUT, async {
            let row = sqlx::query("SELECT OCTET_LENGTH(value) AS value_bytes, IF(OCTET_LENGTH(value) <= ?, value, NULL) AS value, version FROM state WHERE `key` = ?")
                .bind(crate::service::rpc::OPERATION_FRAME_LIMIT as u64)
                .bind(key.as_bytes())
                .fetch_optional(reader.pool.as_ref())
                .await?;
            row.map(decode).transpose()
        })
        .await
        .context("state cut read budget elapsed")??;
        crate::service::rpc::validate_state_cut_value_reply(&value)?;
        Ok(value)
    }

    pub async fn page(
        &self,
        prefix: &str,
        cursor: Option<StateReadCursor>,
    ) -> Result<StateReadPage> {
        identifier("state prefix", prefix, 1024)?;
        let after = match cursor {
            None => Vec::new(),
            Some(cursor) => {
                ensure!(
                    cursor.snapshot == self.0.id && cursor.prefix == prefix,
                    "state cursor belongs to a different cut or prefix"
                );
                ensure!(cursor.after.len() <= 1024, "state cursor key is oversized");
                cursor.after
            }
        };
        let retained = self.0.reader.read().await;
        let reader = retained.as_ref().context("state cut is closed")?;
        ensure!(!reader.pool.is_closed(), "state cut owner is closed");
        crate::pool::within(QUERY_TIMEOUT, async {
            let mut rows = sqlx::query(
                "SELECT `key`, OCTET_LENGTH(value) AS value_bytes, IF(OCTET_LENGTH(value) <= ?, value, NULL) AS value, version FROM state WHERE LEFT(`key`, ?) = ? AND `key` > ? ORDER BY `key` LIMIT 256",
            )
            .bind(crate::service::rpc::OPERATION_FRAME_LIMIT as u64)
            .bind(prefix.len() as i64)
            .bind(prefix.as_bytes())
            .bind(after)
            .fetch(reader.pool.as_ref());
            let mut page = StateReadPage { values: Vec::new(), next: None };
            let mut bytes = 0usize;
            while let Some(row) = rows.try_next().await? {
                let key = String::from_utf8(row.try_get("key")?)
                    .context("stored state key is not UTF-8")?;
                let value = decode(row)?;
                let encoded = crate::service::rpc::encoded_bytes(&(&key, &value))?;
                if !page.values.is_empty() && bytes.saturating_add(encoded) > MAX_STATE_BATCH_BYTES {
                    page.next = Some(self.cursor(prefix, &page.values));
                    break;
                }
                bytes = bytes.saturating_add(encoded);
                page.values.push((key, value));
                if bytes >= MAX_STATE_BATCH_BYTES || page.values.len() == MAX_STATE_BATCH_KEYS {
                    page.next = Some(self.cursor(prefix, &page.values));
                    break;
                }
            }
            crate::service::rpc::validate_state_cut_page_reply(&page)?;
            Ok::<_, anyhow::Error>(page)
        })
        .await
        .context("state cut page budget elapsed")?
    }

    fn cursor(&self, prefix: &str, values: &[(String, VersionedValue)]) -> StateReadCursor {
        StateReadCursor {
            snapshot: self.0.id,
            prefix: prefix.to_owned(),
            after: values.last().expect("nonempty page").0.as_bytes().to_vec(),
        }
    }

    /// Close every clone of this cut after any in-flight read ends. Revision
    /// pools can be shared with another cut/export, so release this reference
    /// instead of closing their common pool.
    pub async fn close(&self) -> Result<()> {
        let reader = self.0.reader.write().await.take();
        drop(reader);
        Ok(())
    }
}

fn decode(row: sqlx::mysql::MySqlRow) -> Result<VersionedValue> {
    ensure!(
        usize::try_from(row.try_get::<i64, _>("value_bytes")?)?
            <= crate::service::rpc::OPERATION_FRAME_LIMIT,
        "stored state exceeds the service envelope"
    );
    let version = u64::try_from(row.try_get::<i64, _>("version")?)
        .context("stored state version is negative")?;
    Ok(VersionedValue {
        value: serde_json::from_str(&row.try_get::<String, _>("value")?)
            .context("stored state contains invalid JSON")?,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as kuru_memory;
    use serde_json::json;

    #[test]
    fn state_read_cut_async_fixtures_use_the_closing_scope() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        crate::test_support::assert_async_tests_run_in_closing(
            &root,
            &[
                root.join("store/state_read_cut.rs"),
                root.join("facade/state_read_cut.rs"),
            ],
        );
    }

    async fn commit(pool: &MemoryPool, message: &str) -> Result<()> {
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind(message)
            .bind(AUTHOR)
            .fetch_all(pool)
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn state_read_cut_keeps_complete_revision_across_pages_and_candidates() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let store = MemoryStore::temporary().await?;
            crate::test_support::closing::register(store.server_for_teardown());
            let values: Vec<_> = (0..301)
                .map(|n| {
                    (
                        format!("reports/{n:04}"),
                        json!({"activation":n,"archived":n>=256}),
                    )
                })
                .collect();
            store.put_many(&values).await?;
            store.put("membership", &json!({"parts":["old"]})).await?;
            let cut = store.begin_state_read_cut().await?;
            let sibling = store.begin_state_read_cut().await?;
            let first = cut.page("reports/", None).await?;
            assert_eq!(first.values.len(), 256);
            let cursor = first.next.context("first page has continuation")?;
            assert!(
                sibling
                    .page("reports/", Some(cursor.clone()))
                    .await
                    .is_err()
            );
            assert!(cut.page("other/", Some(cursor.clone())).await.is_err());
            store
                .put("reports/0300", &json!({"activation":"new"}))
                .await?;
            store
                .put("reports/0301", &json!({"activation":"inserted"}))
                .await?;
            store.put("membership", &json!({"parts":["new"]})).await?;
            let rest = cut.page("reports/", Some(cursor)).await?;
            assert_eq!(rest.values.len(), 45);
            assert!(rest.next.is_none());
            assert_eq!(rest.values.last().unwrap().1.value, values[300].1);
            assert_eq!(
                cut.get_versioned("membership").await?.unwrap().value,
                json!({"parts":["old"]})
            );
            assert!(cut.get_versioned("missing").await?.is_none());
            let clone = cut.clone();
            cut.close().await?;
            cut.close().await?;
            assert!(clone.page("reports/", None).await.is_err());
            assert_eq!(sibling.page("reports/", None).await?.values.len(), 256);
            sibling.close().await?;

            let candidate = store.begin_candidate("read cut isolation").await?;
            let view = candidate.view();
            view.put("reports/0300", &json!({"candidate":true})).await?;
            let candidate_cut = view.begin_state_read_cut().await?;
            assert_eq!(candidate_cut.provenance().branch, view.pinned_view());
            view.put("reports/0300", &json!({"candidate":"later"}))
                .await?;
            assert_eq!(
                candidate_cut
                    .get_versioned("reports/0300")
                    .await?
                    .unwrap()
                    .value,
                json!({"candidate":true})
            );
            assert_eq!(
                store.get("reports/0300").await?,
                Some(json!({"activation":"new"}))
            );
            candidate_cut.close().await?;
            candidate.abandon().await?;
            store.close().await
        })
        .await
    }

    #[tokio::test]
    async fn state_read_cut_large_row_uses_existing_envelope_and_refuses_corruption() -> Result<()>
    {
        kuru_memory::test_support::closing(async {
            let store = MemoryStore::temporary().await?;
            crate::test_support::closing::register(store.server_for_teardown());
            let large = json!("x".repeat(MAX_STATE_BATCH_BYTES + 1024));
            store
                .put_many_conditional(
                    &[("large/value".into(), StateExpectation::Absent)],
                    &[("large/value".into(), large.clone())],
                )
                .await?;
            assert!(
                store
                    .get_many_versioned(&["large/value".into()])
                    .await
                    .is_err()
            );
            let cut = store.begin_state_read_cut().await?;
            let page = cut.page("large/", None).await?;
            assert_eq!(page.values.len(), 1);
            assert_eq!(page.values[0].1.value, large);
            assert_eq!(
                cut.get_versioned("large/value").await?.unwrap().value,
                large
            );
            assert!(cut.page("large/", page.next).await?.values.is_empty());
            cut.close().await?;
            sqlx::query("INSERT INTO state (`key`, value, version) VALUES (?, REPEAT('x', ?), 0)")
                .bind(b"oversized/value".as_slice())
                .bind((crate::service::rpc::OPERATION_FRAME_LIMIT + 1) as u64)
                .execute(store.pool.as_ref())
                .await?;
            commit(&store.pool, "Oversized corrupt state fixture").await?;
            let cut = store.begin_state_read_cut().await?;
            for result in [
                cut.get_versioned("oversized/value").await.map(|_| ()),
                cut.page("oversized/", None).await.map(|_| ()),
            ] {
                assert!(
                    format!("{:#}", result.unwrap_err()).contains("exceeds the service envelope")
                );
            }
            cut.close().await?;
            sqlx::query("INSERT INTO state (`key`, value, version) VALUES (?, 'null', -1)")
                .bind(b"negative/value".as_slice())
                .execute(store.pool.as_ref())
                .await?;
            commit(&store.pool, "Negative corrupt state fixture").await?;
            let cut = store.begin_state_read_cut().await?;
            assert!(cut.get_versioned("negative/value").await.is_err());
            assert!(cut.page("negative/", None).await.is_err());
            cut.close().await?;
            store.close().await
        })
        .await
    }
}
