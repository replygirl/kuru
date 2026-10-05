//! Checked state reads and atomic compare/publication. SQL row locks do not
//! replace the owner's write mutex: every writer must use that same owner.

use super::*;
use sqlx::{MySql, QueryBuilder};

pub const MAX_STATE_BATCH_KEYS: usize = 256;
/// Cumulative stored JSON in the small coherent read batch.
pub const MAX_STATE_BATCH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct VersionedValue {
    pub value: Value,
    pub version: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StateExpectation {
    Absent,
    Version(u64),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StateStale {
    pub key: String,
    pub expected: StateExpectation,
    pub actual: Option<u64>,
}

impl std::fmt::Display for StateStale {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "state key {} changed: expected {:?}, actual {:?}",
            self.key, self.expected, self.actual
        )
    }
}

impl std::error::Error for StateStale {}

pub(crate) fn validate_keys(keys: &[String]) -> Result<()> {
    ensure!(
        keys.len() <= MAX_STATE_BATCH_KEYS,
        "state batch exceeds {MAX_STATE_BATCH_KEYS} keys"
    );
    let mut seen = BTreeSet::new();
    for key in keys {
        identifier("state key", key, 1024)?;
        ensure!(seen.insert(key), "state batch contains a duplicate key");
    }
    Ok(())
}

pub(crate) fn validate_conditional(
    expected: &[(String, StateExpectation)],
    values: &[(String, Value)],
    candidate: bool,
) -> Result<()> {
    ensure!(
        !expected.is_empty() && !values.is_empty(),
        "conditional state batch must contain expectations and values"
    );
    let keys: Vec<_> = values.iter().map(|(key, _)| key.clone()).collect();
    validate_keys(&keys)?;
    let expected_keys: Vec<_> = expected.iter().map(|(key, _)| key.clone()).collect();
    validate_keys(&expected_keys)?;
    for (key, expectation) in expected {
        ensure!(
            keys.contains(key),
            "conditional state expectation must name a written key"
        );
        if let StateExpectation::Version(version) = expectation {
            ensure!(
                *version <= i64::MAX as u64,
                "state expectation version exceeds the stored range"
            );
        }
    }
    crate::service::rpc::validate_conditional_request(expected, values, candidate)
}

fn checked_version(version: i64) -> Result<u64> {
    u64::try_from(version).context("stored state version is negative")
}

impl MemoryStore {
    pub async fn get_versioned(&self, key: &str) -> Result<Option<VersionedValue>> {
        self.readable()?;
        identifier("state key", key, 1024)?;
        ensure!(
            self.schema_version().await? >= migrations::STATE_VERSION,
            "versioned state requires an upgraded memory view"
        );
        let row = crate::pool::within(
            QUERY_TIMEOUT,
            sqlx::query("SELECT value, version FROM state WHERE `key` = ?")
                .bind(key.as_bytes())
                .fetch_optional(self.pool.as_ref()),
        )
        .await
        .context("versioned state read deadline exceeded")??;
        row.map(|row| {
            Ok(VersionedValue {
                value: serde_json::from_str(&row.try_get::<String, _>("value")?)
                    .context("stored state contains invalid JSON")?,
                version: checked_version(row.try_get("version")?)?,
            })
        })
        .transpose()
    }

    /// One SELECT snapshot, returned in request order with missing rows intact.
    pub async fn get_many(&self, keys: &[String]) -> Result<Vec<(String, Option<Value>)>> {
        Ok(self
            .get_state_batch(keys, false)
            .await?
            .into_iter()
            .map(|(key, row)| (key, row.map(|row| row.value)))
            .collect())
    }

    /// Values and versions from the same bounded SELECT snapshot.
    pub async fn get_many_versioned(
        &self,
        keys: &[String],
    ) -> Result<Vec<(String, Option<VersionedValue>)>> {
        self.readable()?;
        validate_keys(keys)?;
        ensure!(
            self.schema_version().await? >= migrations::STATE_VERSION,
            "versioned state requires an upgraded memory view"
        );
        self.get_state_batch(keys, true).await
    }

    async fn get_state_batch(
        &self,
        keys: &[String],
        versioned: bool,
    ) -> Result<Vec<(String, Option<VersionedValue>)>> {
        self.readable()?;
        validate_keys(keys)?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = QueryBuilder::<MySql>::new(
            "SELECT `key`, OCTET_LENGTH(value) AS value_bytes, IF(OCTET_LENGTH(value) <= ",
        );
        query.push_bind(MAX_STATE_BATCH_BYTES as u64);
        query.push(", value, NULL) AS value, ");
        query.push(if versioned { "version" } else { "0" });
        query.push(" AS row_version FROM state WHERE `key` IN (");
        let mut separated = query.separated(", ");
        for key in keys {
            separated.push_bind(key.as_bytes());
        }
        separated.push_unseparated(")");
        let mut found = crate::pool::within(QUERY_TIMEOUT, async {
            let mut transaction = self.pool.begin().await?;
            let mut found = BTreeMap::new();
            let mut total = 0usize;
            {
                let mut rows = query.build().fetch(&mut *transaction);
                while let Some(row) = rows.try_next().await? {
                    let bytes = usize::try_from(row.try_get::<i64, _>("value_bytes")?)?;
                    total = total
                        .checked_add(bytes)
                        .context("state batch read size overflow")?;
                    ensure!(
                        total <= MAX_STATE_BATCH_BYTES,
                        "state batch read exceeds {MAX_STATE_BATCH_BYTES} stored JSON bytes"
                    );
                    let key = String::from_utf8(row.try_get("key")?)
                        .context("stored state key is not UTF-8")?;
                    let value = serde_json::from_str(&row.try_get::<String, _>("value")?)
                        .context("stored state contains invalid JSON")?;
                    found.insert(
                        key,
                        VersionedValue {
                            value,
                            version: checked_version(row.try_get("row_version")?)?,
                        },
                    );
                }
            }
            transaction.commit().await?;
            Ok::<_, anyhow::Error>(found)
        })
        .await
        .context("state batch read deadline exceeded")??;
        Ok(keys
            .iter()
            .map(|key| (key.clone(), found.remove(key)))
            .collect())
    }

    pub async fn put_many_conditional(
        &self,
        expected: &[(String, StateExpectation)],
        values: &[(String, Value)],
    ) -> Result<()> {
        validate_conditional(expected, values, self.branch != "main")?;
        self.writable()?;
        ensure!(
            self.schema_version().await? >= migrations::STATE_VERSION,
            "conditional state requires an upgraded memory view"
        );
        self.mutate(
            "conditional state",
            Mutation::StateConditional {
                expected: expected.to_vec(),
                values: encode_state(values)?,
            },
        )
        .await
    }
}

pub(super) async fn compare(
    transaction: &mut sqlx::Transaction<'_, MySql>,
    expected: &[(String, StateExpectation)],
) -> Result<()> {
    let mut query = QueryBuilder::<MySql>::new("SELECT `key`, version FROM state WHERE `key` IN (");
    let mut separated = query.separated(", ");
    for (key, _) in expected {
        separated.push_bind(key.as_bytes());
    }
    separated.push_unseparated(")");
    let mut versions = BTreeMap::new();
    for row in query.build().fetch_all(&mut **transaction).await? {
        let key =
            String::from_utf8(row.try_get("key")?).context("stored state key is not UTF-8")?;
        versions.insert(key, checked_version(row.try_get("version")?)?);
    }
    for (key, expectation) in expected {
        let actual = versions.get(key).copied();
        let matches = match expectation {
            StateExpectation::Absent => actual.is_none(),
            StateExpectation::Version(version) => actual == Some(*version),
        };
        if !matches {
            return Err(StateStale {
                key: key.clone(),
                expected: *expectation,
                actual,
            }
            .into());
        }
    }
    Ok(())
}

/// Every overwrite on schema 9 advances the version, even when its value is
/// equal. Historical branches retain their exact prior SQL/schema contract.
pub(super) async fn upsert(
    transaction: &mut sqlx::Transaction<'_, MySql>,
    versioned: bool,
    values: Vec<(String, String)>,
) -> Result<()> {
    if versioned {
        // Check the complete batch before changing any row. A corrupt negative
        // version must never be silently normalized by an unconditional writer.
        for chunk in values.chunks(MAX_STATE_BATCH_KEYS) {
            let mut query =
                QueryBuilder::<MySql>::new("SELECT version FROM state WHERE `key` IN (");
            let mut separated = query.separated(", ");
            for (key, _) in chunk {
                separated.push_bind(key.as_bytes());
            }
            separated.push_unseparated(")");
            for row in query.build().fetch_all(&mut **transaction).await? {
                let version = checked_version(row.try_get("version")?)?;
                ensure!(
                    version < i64::MAX as u64,
                    "stored state version cannot advance beyond its range"
                );
            }
        }
    }
    let sql = if versioned {
        "INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value), version = version + 1"
    } else {
        "INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)"
    };
    for (key, value) in values {
        sqlx::query(sql)
            .bind(key.as_bytes())
            .bind(value)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn receipts(store: &MemoryStore) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM operations")
            .fetch_one(store.pool.as_ref())
            .await?)
    }

    #[test]
    fn conditional_state_exact_encoded_byte_boundary() -> Result<()> {
        use crate::service::rpc::{
            OPERATION_FRAME_LIMIT, ServiceCall, ServiceRequest, ViewOperation, encoded_bytes,
        };
        let expected = vec![("key".into(), StateExpectation::Absent)];
        let mut values = vec![("key".into(), json!(""))];
        for candidate in [false, true] {
            let request = ServiceRequest::with_id(
                "00000000-0000-0000-0000-000000000000",
                Uuid::nil(),
                ServiceCall::View {
                    candidate: candidate.then(Uuid::nil),
                    operation: Box::new(ViewOperation::PutManyConditional {
                        expected: expected.clone(),
                        values: vec![("key".into(), json!(""))],
                    }),
                },
            );
            let overhead = encoded_bytes(&request)?;
            values[0].1 = json!("x".repeat(OPERATION_FRAME_LIMIT - overhead));
            validate_conditional(&expected, &values, candidate)?;
            values[0].1 = json!("x".repeat(OPERATION_FRAME_LIMIT - overhead + 1));
            assert!(validate_conditional(&expected, &values, candidate).is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn conditional_state_local_race_rollback_and_invalidation() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let result = async {
            let expected = vec![("shared".into(), StateExpectation::Absent)];
            let first = vec![("shared".into(), json!(1)), ("first".into(), json!(true))];
            let second = vec![("shared".into(), json!(2)), ("second".into(), json!(true))];
            let (a, b) = tokio::join!(
                store.put_many_conditional(&expected, &first),
                store.put_many_conditional(&expected, &second)
            );
            assert_ne!(a.is_ok(), b.is_ok());
            let error = if let Err(error) = a {
                error
            } else {
                b.unwrap_err()
            };
            assert_eq!(error.downcast_ref::<StateStale>().unwrap().actual, Some(0));
            let winner = store.get_versioned("shared").await?.unwrap();
            let distinct = ["K", "k", "e", "é"].map(|key| (key.to_owned(), json!(key)));
            let absent = distinct
                .iter()
                .map(|(key, _)| (key.clone(), StateExpectation::Absent))
                .collect::<Vec<_>>();
            store.put_many_conditional(&absent, &distinct).await?;
            let keys = distinct
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            assert_eq!(
                store.get_many(&keys).await?,
                distinct
                    .iter()
                    .map(|(key, value)| (key.clone(), Some(value.clone())))
                    .collect::<Vec<_>>()
            );
            let rows = store.get_many_versioned(&keys).await?;
            for ((key, row), (expected_key, value)) in rows.into_iter().zip(&distinct) {
                assert_eq!(&key, expected_key);
                assert_eq!(
                    row,
                    Some(VersionedValue {
                        value: value.clone(),
                        version: 0
                    })
                );
            }
            assert_eq!(
                store.get_many_versioned(&["missing".into()]).await?,
                vec![("missing".into(), None)]
            );
            let before = (store.revision().await?, receipts(&store).await?);
            let error = store
                .put_many_conditional(
                    &[("shared".into(), StateExpectation::Absent)],
                    &[
                        ("shared".into(), json!(9)),
                        ("companion".into(), json!(true)),
                    ],
                )
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<StateStale>().is_some());
            assert_eq!(store.get("companion").await?, None);
            assert_eq!(before, (store.revision().await?, receipts(&store).await?));
            store
                .put_many_conditional(
                    &[("shared".into(), StateExpectation::Version(0))],
                    &[
                        ("shared".into(), winner.value.clone()),
                        ("retry".into(), json!(true)),
                    ],
                )
                .await?;
            assert_eq!(store.get_versioned("shared").await?.unwrap().version, 1);
            store.put("shared", &winner.value).await?;
            store
                .checkpoint(
                    "conversation",
                    &[Message::text("user", "later")],
                    &[("shared".into(), winner.value)],
                )
                .await?;
            assert_eq!(store.get_versioned("shared").await?.unwrap().version, 3);
            assert!(
                store
                    .put_many_conditional(
                        &[("shared".into(), StateExpectation::Version(1))],
                        &[("shared".into(), json!(10))]
                    )
                    .await
                    .unwrap_err()
                    .downcast_ref::<StateStale>()
                    .is_some()
            );
            assert_eq!(
                store
                    .get_many(&["retry".into(), "missing".into(), "shared".into()])
                    .await?
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.is_some()))
                    .collect::<Vec<_>>(),
                vec![("retry", true), ("missing", false), ("shared", true)]
            );
            let candidate = store.begin_candidate("conditional fixture").await?;
            candidate
                .view()
                .put_many_conditional(
                    &[("shared".into(), StateExpectation::Version(3))],
                    &[("shared".into(), json!(11))],
                )
                .await?;
            assert_eq!(store.get_versioned("shared").await?.unwrap().version, 3);
            candidate.promote().await?;
            assert_eq!(store.get_versioned("shared").await?.unwrap().version, 4);
            store
                .put_many(&[("pair-a".into(), json!(0)), ("pair-b".into(), json!(0))])
                .await?;
            let (writer, reader) = tokio::join!(
                async {
                    for value in 1..=12 {
                        store
                            .put_many(&[
                                ("pair-a".into(), json!(value)),
                                ("pair-b".into(), json!(value)),
                            ])
                            .await?;
                    }
                    Ok::<(), anyhow::Error>(())
                },
                async {
                    for _ in 0..24 {
                        let pair = store
                            .get_many_versioned(&["pair-a".into(), "pair-b".into()])
                            .await?;
                        assert_eq!(
                            pair[0].1, pair[1].1,
                            "batch crossed a committed publication"
                        );
                        let row = pair[0].1.as_ref().unwrap();
                        assert_eq!(
                            row.value,
                            json!(row.version),
                            "value and version crossed snapshots"
                        );
                    }
                    Ok::<(), anyhow::Error>(())
                }
            );
            writer?;
            reader?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        result.and(store.close().await)
    }

    #[tokio::test]
    async fn conditional_state_invalid_and_corrupt_versions_have_no_partial_effects() -> Result<()>
    {
        let store = MemoryStore::temporary().await?;
        let result = async {
            store.put("guard", &json!(0)).await?;
            for invalid in [-1, i64::MAX] {
                sqlx::query("UPDATE state SET version = ? WHERE `key` = ?")
                    .bind(invalid)
                    .bind(b"guard".as_slice())
                    .execute(store.pool.as_ref())
                    .await?;
                sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                    .bind("Corrupt or exhausted state version")
                    .bind(AUTHOR)
                    .fetch_all(store.pool.as_ref())
                    .await?;
                let before = (store.revision().await?, receipts(&store).await?);
                assert!(
                    store
                        .put_many_conditional(
                            &[("new".into(), StateExpectation::Absent)],
                            &[("new".into(), json!(1)), ("guard".into(), json!(1))]
                        )
                        .await
                        .is_err()
                );
                assert_eq!(store.get("new").await?, None);
                assert_eq!(store.get("guard").await?, Some(json!(0)));
                assert!(store.put("guard", &json!(0)).await.is_err());
                assert!(
                    store
                        .checkpoint(
                            "rollback-conversation",
                            &[Message::text("user", "must roll back")],
                            &[
                                ("checkpoint-companion".into(), json!(true)),
                                ("guard".into(), json!(1))
                            ]
                        )
                        .await
                        .is_err()
                );
                assert!(store.history("rollback-conversation", 10).await?.is_empty());
                assert_eq!(store.get("checkpoint-companion").await?, None);
                if invalid < 0 {
                    assert!(store.get_versioned("guard").await.is_err());
                    assert!(store.get_many_versioned(&["guard".into()]).await.is_err());
                }
                assert_eq!(before, (store.revision().await?, receipts(&store).await?));
            }
            let before = (store.revision().await?, receipts(&store).await?);
            let valid = vec![("valid".into(), json!(0))];
            for expected in [
                vec![],
                vec![("valid".into(), StateExpectation::Version(u64::MAX))],
                vec![("other".into(), StateExpectation::Absent)],
                vec![("valid".into(), StateExpectation::Absent); 2],
            ] {
                assert!(store.put_many_conditional(&expected, &valid).await.is_err());
            }
            assert!(
                store
                    .put_many_conditional(
                        &[("valid".into(), StateExpectation::Absent)],
                        &[("valid".into(), json!(0)), ("x".repeat(1025), json!(0))]
                    )
                    .await
                    .is_err()
            );
            assert!(store.get_many(&vec!["duplicate".into(); 2]).await.is_err());
            assert!(
                store
                    .get_many(&(0..257).map(|i| i.to_string()).collect::<Vec<_>>())
                    .await
                    .is_err()
            );
            assert!(
                store
                    .put_many_conditional(
                        &[("valid".into(), StateExpectation::Absent)],
                        &[(
                            "valid".into(),
                            json!(
                                "\u{0001}"
                                    .repeat(crate::service::rpc::OPERATION_FRAME_LIMIT / 6 + 1)
                            )
                        )]
                    )
                    .await
                    .is_err()
            );
            assert_eq!(before, (store.revision().await?, receipts(&store).await?));
            // Legacy state remains unrestricted. Only the new batch read refuses
            // its oversized result; the scalar API retains compatibility.
            store
                .put("large", &json!("x".repeat(MAX_STATE_BATCH_BYTES)))
                .await?;
            assert!(store.get_many(&["large".into()]).await.is_err());
            assert!(store.get_many_versioned(&["large".into()]).await.is_err());
            assert!(store.get("large").await?.is_some());
            let half = json!("x".repeat(MAX_STATE_BATCH_BYTES / 2));
            store
                .put_many(&[("half-a".into(), half.clone()), ("half-b".into(), half)])
                .await?;
            assert!(
                store
                    .get_many(&["half-a".into(), "half-b".into()])
                    .await
                    .is_err()
            );
            Ok::<(), anyhow::Error>(())
        }
        .await;
        result.and(store.close().await)
    }
}
