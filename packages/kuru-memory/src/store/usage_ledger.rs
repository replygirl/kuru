//! Private, branch-pinned provider usage accounting.
//!
//! The permanent branch is deliberately reachable only through `UsageLedger`.
//! It shares the project's writer serialization and receipt reconciliation, but
//! never gives a caller a mutable `MemoryStore` view of that branch.

use super::*;
use kuru_core::{
    InvocationOutcome, InvocationStart, InvocationUsage, MoneyEstimate, PriceBasis, SessionUsage,
    UnappliedPriceTerm, Usage, UsageCompleteness, UsageObservation,
};
use sha2::{Digest, Sha256};

pub(crate) const BRANCH: &str = "kuru_usage_v1";
const FORMAT: u32 = 1;
const RECORD_PREFIX: &str = "kuru.usage.v1/record/";
const OBSERVATION_PREFIX: &str = "kuru.usage.v1/observation/";
const SESSION_PREFIX: &str = "kuru.usage.v1/session/";
const SESSION_INDEX_PREFIX: &str = "kuru.usage.v1/session-index/";
const OWNED_PREFIX: &str = "kuru.usage.v1/";
const PAGE_SIZE: i64 = 128;

#[derive(Clone, Debug)]
pub struct UsageLedger {
    store: MemoryStore,
}

/// Compact, typed natural-key proof for one accepted ledger call. The query
/// carries no duplicated invocation record or provider payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UsageProof {
    NewSession {
        session_id: String,
    },
    Admit {
        invocation_id: String,
        digest: String,
    },
    Observe {
        invocation_id: String,
        sequence: u64,
        digest: String,
    },
    Settle {
        invocation_id: String,
        digest: String,
    },
}

impl UsageProof {
    pub(crate) fn new_session(session_id: &str) -> Self {
        Self::NewSession {
            session_id: session_id.to_owned(),
        }
    }

    pub(crate) fn admit(start: &InvocationStart) -> Result<Self> {
        Ok(Self::Admit {
            invocation_id: start.invocation_id.clone(),
            digest: proof_digest(start)?,
        })
    }

    pub(crate) fn observe(invocation_id: &str, observation: &UsageObservation) -> Result<Self> {
        Ok(Self::Observe {
            invocation_id: invocation_id.to_owned(),
            sequence: observation.sequence,
            digest: proof_digest(observation)?,
        })
    }

    pub(crate) fn settle(invocation_id: &str, outcome: InvocationOutcome) -> Result<Self> {
        Ok(Self::Settle {
            invocation_id: invocation_id.to_owned(),
            digest: proof_digest(&outcome)?,
        })
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::NewSession { session_id } => session_id_valid(session_id),
            Self::Admit {
                invocation_id,
                digest,
            }
            | Self::Settle {
                invocation_id,
                digest,
            } => {
                invocation_id_valid(invocation_id)?;
                proof_digest_valid(digest)
            }
            Self::Observe {
                invocation_id,
                sequence,
                digest,
            } => {
                invocation_id_valid(invocation_id)?;
                ensure!(*sequence > 0, "usage proof sequence must start at one");
                proof_digest_valid(digest)
            }
        }
    }
}

fn proof_digest(value: &impl Serialize) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn proof_digest_valid(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid usage proof digest"
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SessionMarker {
    format: u32,
    session_id: String,
    historical_complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SessionIndex {
    format: u32,
    session_id: String,
    invocation_id: String,
    record_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredObservation {
    invocation_id: String,
    observation: UsageObservation,
}

enum Change {
    MarkNewSession(String),
    Admit(Box<InvocationStart>),
    Observe(String, UsageObservation),
    Settle(String, InvocationOutcome),
}

impl UsageLedger {
    pub(super) fn new(store: MemoryStore) -> Self {
        Self { store }
    }

    /// Mark a freshly created P7 session before its first provider call.
    /// This can never upgrade a session that has already been identified as
    /// pre-ledger/incomplete.
    pub async fn mark_new_session(&self, session_id: &str) -> Result<()> {
        session_id_valid(session_id)?;
        self.change(Change::MarkNewSession(session_id.into())).await
    }

    /// Insert the immutable identity before provider dispatch.
    pub async fn admit(&self, start: InvocationStart) -> Result<()> {
        start.validate()?;
        self.change(Change::Admit(Box::new(start))).await
    }

    /// Durably retain one local ordered provider usage observation.
    pub async fn observe(&self, invocation_id: &str, observation: UsageObservation) -> Result<()> {
        invocation_id_valid(invocation_id)?;
        observation.validate()?;
        self.change(Change::Observe(invocation_id.into(), observation))
            .await
    }

    /// Record the terminal local outcome without fabricating missing usage.
    pub async fn settle(&self, invocation_id: &str, outcome: InvocationOutcome) -> Result<()> {
        invocation_id_valid(invocation_id)?;
        self.change(Change::Settle(invocation_id.into(), outcome))
            .await
    }

    /// Fold bounded invocation records on read; no mutable session total is
    /// stored, so retries and uncertain receipts cannot double count it.
    pub async fn session(&self, session_id: &str) -> Result<SessionUsage> {
        session_id_valid(session_id)?;
        self.store.readable()?;
        let marker = read_marker(self.store.pool.as_ref(), session_id).await?;
        let mut fold = SessionFold::new(session_id, marker);
        let mut after = None;
        loop {
            let page =
                session_index_page(self.store.pool.as_ref(), session_id, after.as_deref()).await?;
            if page.is_empty() {
                break;
            }
            for (key, value) in &page {
                let key = String::from_utf8(key.clone())
                    .context("usage session index key is not UTF-8")?;
                let index = decode_session_index(value)?;
                ensure!(
                    key == session_index_key(&index.session_id, &index.invocation_id)
                        && index.session_id == session_id,
                    "usage session index key does not match its record"
                );
                let record = read_state(self.store.pool.as_ref(), &index.record_key)
                    .await?
                    .context("usage session index references a missing invocation")?;
                let record = decode_record(&record)?;
                ensure!(
                    record.start.session_id == session_id
                        && record.start.invocation_id == index.invocation_id
                        && index.record_key == record_key(&record.start.invocation_id),
                    "usage session index references a mismatched invocation"
                );
                fold.add(&record)?;
            }
            after = page
                .last()
                .map(|(key, _)| String::from_utf8(key.clone()))
                .transpose()
                .context("usage session index key is not UTF-8")?;
        }
        fold.finish()
    }

    /// Inspect one existing natural key after any accepted SQL session has
    /// settled. A missing key is only noncommit proof once the service's
    /// handler-completion or prior-owner-reap boundary is also established.
    pub(crate) async fn inspect_proof(&self, proof: &UsageProof) -> Result<bool> {
        proof.validate()?;
        self.store.readable()?;
        let _guard = self.store.shared.write.lock().await;
        self.store.resolve_uncertain().await?;
        let pool = self.store.pool.as_ref();
        let matching = match proof {
            UsageProof::NewSession { session_id } => read_marker(pool, session_id)
                .await?
                .is_some_and(|marker| marker.historical_complete),
            UsageProof::Admit {
                invocation_id,
                digest,
            } => match read_state(pool, &record_key(invocation_id)).await? {
                Some(value) => {
                    let record = decode_record(&value)?;
                    ensure!(
                        record.start.invocation_id == *invocation_id,
                        "usage proof record identity changed"
                    );
                    ensure!(
                        proof_digest(&record.start)? == *digest,
                        LogicalReceiptConflict
                    );
                    true
                }
                None => false,
            },
            UsageProof::Observe {
                invocation_id,
                sequence,
                digest,
            } => match read_state(pool, &observation_key(invocation_id, *sequence)).await? {
                Some(value) => {
                    let observation = decode_observation(&value)?;
                    ensure!(
                        observation.invocation_id == *invocation_id
                            && observation.observation.sequence == *sequence,
                        "usage proof observation identity changed"
                    );
                    ensure!(
                        proof_digest(&observation.observation)? == *digest,
                        LogicalReceiptConflict
                    );
                    true
                }
                None => false,
            },
            UsageProof::Settle {
                invocation_id,
                digest,
            } => match read_state(pool, &record_key(invocation_id)).await? {
                Some(value) => {
                    let record = decode_record(&value)?;
                    ensure!(
                        record.start.invocation_id == *invocation_id,
                        "usage proof record identity changed"
                    );
                    match record.outcome {
                        Some(outcome) => {
                            ensure!(proof_digest(&outcome)? == *digest, LogicalReceiptConflict);
                            true
                        }
                        None => false,
                    }
                }
                None => false,
            },
        };
        Ok(matching)
    }

    async fn change(&self, change: Change) -> Result<()> {
        self.store.writable()?;
        let guard = self.store.shared.write.clone().lock_owned().await;
        self.store.resolve_uncertain().await?;
        let store = self.store.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let operation = Uuid::new_v4().to_string();
            let (mut connection, id) = owned_connection(&store.pool).await?;
            *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: store.pool.clone(),
                connection: id,
                receipt: Receipt::Operation(operation.clone()),
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                apply_change(&mut connection, &operation, change),
            )
            .await;
            drop(connection);
            match result {
                Ok(Ok(false)) => {
                    *store.shared.uncertain.lock().expect("uncertain lock") = None;
                    Ok(())
                }
                Ok(Ok(true)) => {
                    *store.shared.uncertain.lock().expect("uncertain lock") = None;
                    Ok(())
                }
                other => {
                    if store.resolve_uncertain().await? == Some(true) {
                        return Ok(());
                    }
                    other.context("usage ledger write deadline exceeded")??;
                    bail!("usage ledger mutation did not produce its durable receipt")
                }
            }
        })
        .await
        .context("usage ledger worker failed")?
    }
}

/// Establish the permanent branch only during writable project open.
pub(super) async fn establish(store: &MemoryStore) -> Result<()> {
    store.writable()?;
    let _guard = store.shared.write.lock().await;
    store.resolve_uncertain().await?;
    let exists: Vec<String> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT name FROM dolt_branches WHERE BINARY name = BINARY ? LIMIT 2")
            .bind(BRANCH)
            .fetch_all(store.pool.as_ref()),
    )
    .await
    .context("usage ledger branch inspection deadline exceeded")??;
    ensure!(
        exists.len() <= 1,
        "usage ledger branch identity is ambiguous"
    );
    if exists.is_empty() {
        let base = revision(&store.pool).await?;
        tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind(BRANCH)
                .bind(base)
                .fetch_all(store.pool.as_ref()),
        )
        .await
        .context("usage ledger branch creation deadline exceeded")??;
    }
    let pool = store.shared.server.pool(BRANCH).await?;
    validate_branch(pool.as_ref()).await?;
    migrations::upgrade_usage(&store.shared.server, pool.as_ref()).await?;
    migrations::validate_usage(&store.shared.server, pool.as_ref()).await?;
    validate_branch(pool.as_ref()).await?;
    *store.shared.usage_pool.lock().expect("usage pool lock") = Some(pool);
    Ok(())
}

async fn validate_branch(pool: &MySqlPool) -> Result<()> {
    let dirty: i64 = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(pool),
    )
    .await
    .context("usage ledger working-set validation deadline exceeded")??;
    ensure!(dirty == 0, "usage ledger branch has uncommitted changes");
    let version = migrations::validate_historical(pool).await?;
    if version <= 3 {
        let old_receipts: Vec<(String, String)> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_as("SELECT id, label FROM operations LIMIT 2").fetch_all(pool),
        )
        .await
        .context("historical usage receipt validation deadline exceeded")??;
        ensure!(
            old_receipts.len() <= 1,
            "historical usage receipt state is ambiguous"
        );
    }
    // Current usage receipts intentionally survive later ledger writes;
    // there is no one-row limit on an upgraded writable branch.
    let mut after = None;
    loop {
        let rows = owned_state_page(pool, after.as_deref()).await?;
        if rows.is_empty() {
            break;
        }
        for (key, value) in &rows {
            let key = String::from_utf8(key.clone()).context("usage ledger key is not UTF-8")?;
            if key.starts_with(SESSION_PREFIX) {
                let marker = decode_marker(value)?;
                ensure!(
                    key == session_key(&marker.session_id),
                    "usage marker key is not canonical"
                );
            } else if key.starts_with(RECORD_PREFIX) {
                let record = decode_record(value)?;
                ensure!(
                    key == record_key(&record.start.invocation_id),
                    "usage ledger record key does not match its invocation"
                );
            } else if key.starts_with(OBSERVATION_PREFIX) {
                let observation = decode_observation(value)?;
                ensure!(
                    key == observation_key(
                        &observation.invocation_id,
                        observation.observation.sequence
                    ),
                    "usage observation key does not match its value"
                );
            } else if key.starts_with(SESSION_INDEX_PREFIX) {
                let index = decode_session_index(value)?;
                ensure!(
                    key == session_index_key(&index.session_id, &index.invocation_id)
                        && index.record_key == record_key(&index.invocation_id),
                    "usage session index is not canonical"
                );
            } else {
                bail!("usage ledger owns an unrecognized state key");
            }
        }
        after = rows
            .last()
            .map(|(key, _)| String::from_utf8(key.clone()))
            .transpose()
            .context("usage ledger key is not UTF-8")?;
    }
    Ok(())
}

async fn apply_change(
    connection: &mut MySqlConnection,
    operation: &str,
    change: Change,
) -> Result<bool> {
    let mut transaction = connection.begin().await?;
    let changed = match change {
        Change::MarkNewSession(session_id) => {
            let marker = read_marker_tx(&mut transaction, &session_id).await?;
            let has_records = session_has_records_tx(&mut transaction, &session_id).await?;
            match marker {
                Some(marker) if marker.historical_complete => false,
                Some(_) => bail!("usage session is already pre-ledger and cannot become complete"),
                None if has_records => bail!("usage session already has invocation records"),
                None => {
                    put_state_tx(
                        &mut transaction,
                        &session_key(&session_id),
                        &SessionMarker {
                            format: FORMAT,
                            session_id: session_id.clone(),
                            historical_complete: true,
                        },
                    )
                    .await?;
                    true
                }
            }
        }
        Change::Admit(start) => {
            let start = *start;
            let marker = read_marker_tx(&mut transaction, &start.session_id).await?;
            if marker.is_none() {
                put_state_tx(
                    &mut transaction,
                    &session_key(&start.session_id),
                    &SessionMarker {
                        format: FORMAT,
                        session_id: start.session_id.clone(),
                        historical_complete: false,
                    },
                )
                .await?;
            }
            let key = record_key(&start.invocation_id);
            match read_state_tx(&mut transaction, &key).await? {
                Some(value) => {
                    ensure!(
                        decode_record(&value)?.start == start,
                        "usage admission conflicts with an existing invocation"
                    );
                    false
                }
                None => {
                    let index = SessionIndex {
                        format: FORMAT,
                        session_id: start.session_id.clone(),
                        invocation_id: start.invocation_id.clone(),
                        record_key: key.clone(),
                    };
                    let index_key = session_index_key(&start.session_id, &start.invocation_id);
                    ensure!(
                        read_state_tx(&mut transaction, &index_key).await?.is_none(),
                        "usage session index conflicts with an existing invocation"
                    );
                    put_state_tx(&mut transaction, &key, &initial_record(start)).await?;
                    put_state_tx(&mut transaction, &index_key, &index).await?;
                    true
                }
            }
        }
        Change::Observe(invocation_id, observation) => {
            let key = record_key(&invocation_id);
            let encoded = read_state_tx(&mut transaction, &key)
                .await?
                .context("usage observation has no admitted invocation")?;
            let mut record = decode_record(&encoded)?;
            let observation_key = observation_key(&invocation_id, observation.sequence);
            if let Some(existing) = read_state_tx(&mut transaction, &observation_key).await? {
                let existing = decode_observation(&existing)?;
                ensure!(
                    existing.invocation_id == invocation_id && existing.observation == observation,
                    "usage observation conflicts with its durable sequence"
                );
                false
            } else {
                ensure!(
                    record.terminal_usage.is_none(),
                    "usage terminal observation is already recorded"
                );
                ensure!(
                    record.outcome.is_none(),
                    "usage invocation is already settled"
                );
                if let Some(last) = record.last_usage_sequence {
                    ensure!(
                        observation.sequence > last,
                        "usage observation sequence is stale"
                    );
                }
                merge_usage(&mut record.usage, &observation.usage);
                record.last_usage_sequence = Some(observation.sequence);
                if observation.terminal {
                    record.terminal_usage = Some(observation.usage.clone());
                }
                record.incomplete = record.terminal_usage.as_ref().is_none_or(usage_missing);
                put_state_tx(&mut transaction, &key, &record).await?;
                put_state_tx(
                    &mut transaction,
                    &observation_key,
                    &StoredObservation {
                        invocation_id,
                        observation,
                    },
                )
                .await?;
                true
            }
        }
        Change::Settle(invocation_id, outcome) => {
            let key = record_key(&invocation_id);
            let encoded = read_state_tx(&mut transaction, &key)
                .await?
                .context("usage settlement has no admitted invocation")?;
            let mut record = decode_record(&encoded)?;
            match record.outcome {
                Some(existing) => {
                    ensure!(
                        existing == outcome,
                        "usage settlement conflicts with its durable outcome"
                    );
                    false
                }
                None => {
                    record.outcome = Some(outcome);
                    record.incomplete = record.terminal_usage.as_ref().is_none_or(usage_missing);
                    put_state_tx(&mut transaction, &key, &record).await?;
                    true
                }
            }
        }
    };
    if !changed {
        transaction.rollback().await?;
        return Ok(false);
    }
    let version: i32 = sqlx::query_scalar("SELECT version FROM kuru_schema WHERE id = 1")
        .fetch_one(&mut *transaction)
        .await?;
    ensure!(
        version == migrations::CURRENT_VERSION,
        "usage ledger branch requires retained receipt schema before writing"
    );
    sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
        .bind(operation)
        .bind("usage ledger v1")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(format!("usage ledger v1 [{operation}]"))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

fn initial_record(start: InvocationStart) -> InvocationUsage {
    InvocationUsage {
        start,
        usage: Usage::default(),
        last_usage_sequence: None,
        terminal_usage: None,
        outcome: None,
        incomplete: true,
    }
}

fn merge_usage(current: &mut Usage, observed: &Usage) {
    if observed.input_tokens.is_some() {
        current.input_tokens = observed.input_tokens;
    }
    if observed.output_tokens.is_some() {
        current.output_tokens = observed.output_tokens;
    }
    if observed.cached_input_tokens.is_some() {
        current.cached_input_tokens = observed.cached_input_tokens;
    }
    if observed.reasoning_output_tokens.is_some() {
        current.reasoning_output_tokens = observed.reasoning_output_tokens;
    }
}

fn usage_missing(usage: &Usage) -> bool {
    usage.input_tokens.is_none()
        || usage.output_tokens.is_none()
        || usage.cached_input_tokens.is_none()
        || usage.reasoning_output_tokens.is_none()
}

async fn read_state_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    key: &str,
) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT value FROM state WHERE `key` = ? FOR UPDATE")
            .bind(key.as_bytes())
            .fetch_optional(&mut **transaction)
            .await?,
    )
}

async fn put_state_tx<T: Serialize>(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    key: &str,
    value: &T,
) -> Result<()> {
    let encoded = serde_json::to_string(value)?;
    ensure!(
        encoded.len() <= 64 * 1024,
        "usage ledger record exceeds its byte limit"
    );
    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)")
        .bind(key.as_bytes())
        .bind(encoded)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn read_marker(pool: &MySqlPool, session_id: &str) -> Result<Option<SessionMarker>> {
    let value: Option<String> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
            .bind(session_key(session_id).as_bytes())
            .fetch_optional(pool),
    )
    .await
    .context("usage session marker read deadline exceeded")??;
    value
        .map(|value| decode_marker(&value))
        .transpose()?
        .map(|marker| {
            ensure!(
                marker.session_id == session_id,
                "usage marker belongs to another session"
            );
            Ok(marker)
        })
        .transpose()
}

async fn read_marker_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    session_id: &str,
) -> Result<Option<SessionMarker>> {
    read_state_tx(transaction, &session_key(session_id))
        .await?
        .map(|value| decode_marker(&value))
        .transpose()?
        .map(|marker| {
            ensure!(
                marker.session_id == session_id,
                "usage marker belongs to another session"
            );
            Ok(marker)
        })
        .transpose()
}

async fn session_has_records_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    session_id: &str,
) -> Result<bool> {
    let prefix = session_index_prefix(session_id);
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM state WHERE LEFT(BINARY `key`, ?) = BINARY ? LIMIT 1 FOR UPDATE",
    )
    .bind(prefix.len() as i64)
    .bind(prefix.as_bytes())
    .fetch_one(&mut **transaction)
    .await?
        > 0)
}

async fn read_state(pool: &MySqlPool, key: &str) -> Result<Option<String>> {
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
            .bind(key.as_bytes())
            .fetch_optional(pool),
    )
    .await
    .context("usage ledger state read deadline exceeded")?
    .map_err(Into::into)
}

async fn session_index_page(
    pool: &MySqlPool,
    session_id: &str,
    after: Option<&str>,
) -> Result<Vec<(Vec<u8>, String)>> {
    let prefix = session_index_prefix(session_id);
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_as("SELECT `key`, value FROM state WHERE LEFT(BINARY `key`, ?) = BINARY ? AND (? IS NULL OR BINARY `key` > BINARY ?) ORDER BY BINARY `key` LIMIT ?")
            .bind(prefix.len() as i64)
            .bind(prefix.as_bytes())
            .bind(after)
            .bind(after)
            .bind(PAGE_SIZE)
            .fetch_all(pool),
    )
    .await
    .context("usage ledger session index deadline exceeded")?
    .map_err(Into::into)
}

async fn owned_state_page(pool: &MySqlPool, after: Option<&str>) -> Result<Vec<(Vec<u8>, String)>> {
    tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_as("SELECT `key`, value FROM state WHERE LEFT(BINARY `key`, ?) = BINARY ? AND (? IS NULL OR BINARY `key` > BINARY ?) ORDER BY BINARY `key` LIMIT ?")
            .bind(OWNED_PREFIX.len() as i64)
            .bind(OWNED_PREFIX.as_bytes())
            .bind(after)
            .bind(after)
            .bind(PAGE_SIZE)
            .fetch_all(pool),
    )
    .await
    .context("usage ledger state validation deadline exceeded")?
    .map_err(Into::into)
}

fn record_key(invocation_id: &str) -> String {
    format!("{RECORD_PREFIX}{}", key_digest(invocation_id))
}
fn observation_key(invocation_id: &str, sequence: u64) -> String {
    format!(
        "{OBSERVATION_PREFIX}{}/{sequence:020}",
        key_digest(invocation_id)
    )
}
fn session_key(session_id: &str) -> String {
    format!("{SESSION_PREFIX}{}", key_digest(session_id))
}
fn session_index_prefix(session_id: &str) -> String {
    format!("{SESSION_INDEX_PREFIX}{}/", key_digest(session_id))
}
fn session_index_key(session_id: &str, invocation_id: &str) -> String {
    format!(
        "{}{}",
        session_index_prefix(session_id),
        key_digest(invocation_id)
    )
}
fn key_digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_record(value: &str) -> Result<InvocationUsage> {
    let record: InvocationUsage =
        serde_json::from_str(value).context("usage ledger record is malformed")?;
    record.start.validate()?;
    if let Some(sequence) = record.last_usage_sequence {
        ensure!(sequence > 0, "usage ledger sequence is invalid");
    }
    Ok(record)
}
fn decode_observation(value: &str) -> Result<StoredObservation> {
    let observation: StoredObservation =
        serde_json::from_str(value).context("usage observation is malformed")?;
    invocation_id_valid(&observation.invocation_id)?;
    observation.observation.validate()?;
    Ok(observation)
}
fn decode_marker(value: &str) -> Result<SessionMarker> {
    let marker: SessionMarker =
        serde_json::from_str(value).context("usage session marker is malformed")?;
    ensure!(
        marker.format == FORMAT,
        "usage session marker format is unsupported"
    );
    session_id_valid(&marker.session_id)?;
    Ok(marker)
}
fn decode_session_index(value: &str) -> Result<SessionIndex> {
    let index: SessionIndex =
        serde_json::from_str(value).context("usage session index is malformed")?;
    ensure!(
        index.format == FORMAT,
        "usage session index format is unsupported"
    );
    session_id_valid(&index.session_id)?;
    invocation_id_valid(&index.invocation_id)?;
    ensure!(
        index.record_key == record_key(&index.invocation_id),
        "usage session index record key is invalid"
    );
    Ok(index)
}
fn session_id_valid(value: &str) -> Result<()> {
    identifier("usage session ID", value, 256)?;
    ensure!(
        !value.chars().any(char::is_control),
        "usage session ID must not contain controls"
    );
    Ok(())
}
fn invocation_id_valid(value: &str) -> Result<()> {
    identifier("usage invocation ID", value, 256)?;
    ensure!(
        !value.chars().any(char::is_control),
        "usage invocation ID must not contain controls"
    );
    Ok(())
}

struct SessionFold {
    session_id: String,
    historical_complete: bool,
    invocation_count: u64,
    incomplete_invocations: u64,
    known_usage: Usage,
    complete: UsageCompleteness,
    api_standard: EstimateFold,
    api_equivalent: EstimateFold,
}

impl SessionFold {
    fn new(session_id: &str, marker: Option<SessionMarker>) -> Self {
        let historical_complete = marker.is_some_and(|marker| marker.historical_complete);
        Self {
            session_id: session_id.into(),
            historical_complete,
            invocation_count: 0,
            incomplete_invocations: 0,
            known_usage: Usage::default(),
            complete: UsageCompleteness {
                input_tokens: historical_complete,
                output_tokens: historical_complete,
                cached_input_tokens: historical_complete,
                reasoning_output_tokens: historical_complete,
            },
            api_standard: EstimateFold::default(),
            api_equivalent: EstimateFold::default(),
        }
    }

    fn add(&mut self, record: &InvocationUsage) -> Result<()> {
        self.invocation_count = self
            .invocation_count
            .checked_add(1)
            .context("usage invocation count overflows")?;
        if record.incomplete {
            self.incomplete_invocations = self.incomplete_invocations.saturating_add(1);
        }
        add_component(
            &mut self.known_usage.input_tokens,
            record.usage.input_tokens,
            record
                .terminal_usage
                .as_ref()
                .and_then(|usage| usage.input_tokens),
            &mut self.complete.input_tokens,
        )?;
        add_component(
            &mut self.known_usage.output_tokens,
            record.usage.output_tokens,
            record
                .terminal_usage
                .as_ref()
                .and_then(|usage| usage.output_tokens),
            &mut self.complete.output_tokens,
        )?;
        add_component(
            &mut self.known_usage.cached_input_tokens,
            record.usage.cached_input_tokens,
            record
                .terminal_usage
                .as_ref()
                .and_then(|usage| usage.cached_input_tokens),
            &mut self.complete.cached_input_tokens,
        )?;
        add_component(
            &mut self.known_usage.reasoning_output_tokens,
            record.usage.reasoning_output_tokens,
            record
                .terminal_usage
                .as_ref()
                .and_then(|usage| usage.reasoning_output_tokens),
            &mut self.complete.reasoning_output_tokens,
        )?;
        let fold = match record
            .start
            .price_at_invocation
            .as_ref()
            .map(|price| &price.basis)
        {
            Some(PriceBasis::ApiStandard { .. }) => &mut self.api_standard,
            Some(PriceBasis::ApiEquivalent { .. }) => &mut self.api_equivalent,
            None => {
                self.api_standard.mark(UnappliedPriceTerm::InvocationPrice);
                self.api_equivalent
                    .mark(UnappliedPriceTerm::InvocationPrice);
                return Ok(());
            }
        };
        if !self.historical_complete || record.incomplete {
            fold.incomplete = true;
        }
        fold.add(record)?;
        Ok(())
    }

    fn finish(self) -> Result<SessionUsage> {
        Ok(SessionUsage {
            session_id: self.session_id,
            historical_complete: self.historical_complete,
            invocation_count: self.invocation_count,
            incomplete_invocations: self.incomplete_invocations,
            known_usage: self.known_usage,
            component_complete: self.complete,
            api_standard: self.api_standard.finish(),
            api_equivalent: self.api_equivalent.finish(),
        })
    }
}

#[cfg(test)]
fn fold_session(
    session_id: &str,
    marker: Option<SessionMarker>,
    records: &[InvocationUsage],
) -> Result<SessionUsage> {
    let mut fold = SessionFold::new(session_id, marker);
    for record in records {
        fold.add(record)?;
    }
    fold.finish()
}

fn add_component(
    total: &mut Option<u64>,
    value: Option<u64>,
    terminal_value: Option<u64>,
    complete: &mut bool,
) -> Result<()> {
    match (total.as_mut(), value) {
        (Some(total), Some(value)) => {
            *total = total.checked_add(value).context("usage total overflows")?
        }
        (Some(_), None) | (None, None) => *complete = false,
        (None, Some(value)) => {
            *total = Some(value);
        }
    }
    if terminal_value.is_none() {
        *complete = false;
    }
    Ok(())
}

#[derive(Default)]
struct EstimateFold {
    usd: f64,
    known: bool,
    incomplete: bool,
    unapplied: BTreeSet<UnappliedPriceTerm>,
}
impl EstimateFold {
    /// Record an incomplete subtotal together with the term it leaves out, so
    /// the rendered label can name it instead of reporting a bare gap.
    fn mark(&mut self, term: UnappliedPriceTerm) {
        self.incomplete = true;
        self.unapplied.insert(term);
    }

    fn add(&mut self, record: &InvocationUsage) -> Result<()> {
        let Some(price) = &record.start.price_at_invocation else {
            self.mark(UnappliedPriceTerm::InvocationPrice);
            return Ok(());
        };
        let (Ok(mut input_rate), Ok(mut output_rate)) = (
            parse_rate(&price.input_per_million_usd),
            parse_rate(&price.output_per_million_usd),
        ) else {
            self.mark(UnappliedPriceTerm::InvocationPrice);
            return Ok(());
        };
        let input = record.usage.input_tokens;
        let mut cached_multiplier = 1.0;
        if let Some(tier) = &price.long_context_tier {
            match input {
                Some(input) if input > tier.input_tokens_over => {
                    let (Ok(input_multiplier), Ok(cached_input_multiplier), Ok(output_multiplier)) = (
                        parse_rate(&tier.input_multiplier),
                        parse_rate(&tier.cached_input_multiplier),
                        parse_rate(&tier.output_multiplier),
                    ) else {
                        self.mark(UnappliedPriceTerm::LongContextTier);
                        return Ok(());
                    };
                    input_rate *= input_multiplier;
                    cached_multiplier = cached_input_multiplier;
                    output_rate *= output_multiplier;
                }
                Some(_) => {}
                None => {
                    // The tier cannot be decided without the input count, so it
                    // is named as unapplied rather than half-applied.
                    self.mark(UnappliedPriceTerm::LongContextTier);
                    return Ok(());
                }
            }
        }
        if price.cache_write.is_some() {
            self.mark(UnappliedPriceTerm::CacheWriteRate);
        }
        let mut amount = 0.0;
        let mut known = false;
        if let Some(output) = record.usage.output_tokens {
            amount += output as f64 * output_rate / 1_000_000.0;
            known = true;
        } else {
            self.mark(UnappliedPriceTerm::TokenComponents);
        }
        match (input, record.usage.cached_input_tokens) {
            (Some(input), Some(cached)) if cached <= input => {
                amount += (input - cached) as f64 * input_rate / 1_000_000.0;
                known = true;
                if cached > 0 {
                    let cached_rate = match &price.cached_input_per_million_usd {
                        Some(rate) => match parse_rate(rate) {
                            Ok(rate) => rate * cached_multiplier,
                            Err(_) => {
                                self.mark(UnappliedPriceTerm::CachedInputRate);
                                return Ok(());
                            }
                        },
                        None => {
                            self.mark(UnappliedPriceTerm::CachedInputRate);
                            0.0
                        }
                    };
                    if cached_rate > 0.0 {
                        amount += cached as f64 * cached_rate / 1_000_000.0;
                        known = true;
                    }
                }
            }
            (Some(_), Some(_)) | (Some(_), None) | (None, _) => {
                self.mark(UnappliedPriceTerm::TokenComponents)
            }
        }
        if !amount.is_finite() {
            self.mark(UnappliedPriceTerm::InvocationPrice);
            return Ok(());
        }
        self.usd += amount;
        if !self.usd.is_finite() {
            self.mark(UnappliedPriceTerm::InvocationPrice);
            return Ok(());
        }
        self.known |= known;
        Ok(())
    }
    fn finish(self) -> MoneyEstimate {
        MoneyEstimate {
            known_usd: (self.known && self.usd.is_finite()).then(|| format!("{:.6}", self.usd)),
            incomplete: self.incomplete,
            unapplied: self.unapplied.into_iter().collect(),
        }
    }
}
fn parse_rate(value: &str) -> Result<f64> {
    let parsed: f64 = value.parse().context("frozen usage price is malformed")?;
    ensure!(
        parsed.is_finite() && parsed >= 0.0,
        "frozen usage price is invalid"
    );
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::{CacheWriteTerms, LongContextTier, PriceSchedule, SourceCitation, UsagePhase};

    fn start(session_id: &str, invocation_id: &str) -> InvocationStart {
        InvocationStart {
            session_id: session_id.into(),
            invocation_id: invocation_id.into(),
            operation_id: "turn-1".into(),
            phase: UsagePhase::Speak,
            actor_id: "speaker".into(),
            route: "responses".into(),
            model: "model".into(),
            price_at_invocation: None,
        }
    }

    #[tokio::test]
    async fn natural_key_proofs_survive_sibling_writes_and_reopen() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let store = reopen(&root).await?;
        let ledger = store.usage_ledger()?;
        let admitted = start("proof-session", "proof-invocation");
        let observation = UsageObservation {
            sequence: 1,
            terminal: true,
            usage: Usage {
                input_tokens: Some(11),
                ..Usage::default()
            },
        };
        let session_proof = UsageProof::new_session("proof-session");
        let admit_proof = UsageProof::admit(&admitted)?;
        let observe_proof = UsageProof::observe("proof-invocation", &observation)?;
        let settle_proof = UsageProof::settle("proof-invocation", InvocationOutcome::Succeeded)?;
        for proof in [&session_proof, &admit_proof, &observe_proof, &settle_proof] {
            ensure!(!ledger.inspect_proof(proof).await?);
        }
        ledger.mark_new_session("proof-session").await?;
        ledger.admit(admitted.clone()).await?;
        ledger
            .observe("proof-invocation", observation.clone())
            .await?;
        ledger
            .settle("proof-invocation", InvocationOutcome::Succeeded)
            .await?;
        ledger
            .admit(start("proof-session", "sibling-invocation"))
            .await?;
        for proof in [&session_proof, &admit_proof, &observe_proof, &settle_proof] {
            ensure!(ledger.inspect_proof(proof).await?);
        }
        let changed = UsageProof::observe(
            "proof-invocation",
            &UsageObservation {
                usage: Usage {
                    input_tokens: Some(12),
                    ..Usage::default()
                },
                ..observation
            },
        )?;
        ensure!(
            ledger
                .inspect_proof(&changed)
                .await
                .unwrap_err()
                .downcast_ref::<LogicalReceiptConflict>()
                .is_some(),
            "changed observation did not conflict with its durable natural key"
        );
        drop(ledger);
        store.close().await?;

        let reopened = reopen(&root).await?;
        let ledger = reopened.usage_ledger()?;
        for proof in [&session_proof, &admit_proof, &observe_proof, &settle_proof] {
            ensure!(ledger.inspect_proof(proof).await?);
        }
        drop(ledger);
        reopened.close().await
    }

    #[tokio::test]
    async fn permanent_branch_keeps_usage_out_of_main_and_candidate_promotion() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let main_before = store.revision().await?;
        let candidate = store
            .begin_candidate("usage branch must not stale candidate")
            .await?;
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session("session-1").await?;
        ledger.admit(start("session-1", "invocation-1")).await?;
        ledger
            .observe(
                "invocation-1",
                UsageObservation {
                    sequence: 1,
                    terminal: false,
                    usage: Usage {
                        input_tokens: Some(12),
                        cached_input_tokens: Some(2),
                        ..Usage::default()
                    },
                },
            )
            .await?;
        ledger
            .observe(
                "invocation-1",
                UsageObservation {
                    sequence: 2,
                    terminal: true,
                    usage: Usage {
                        output_tokens: Some(3),
                        ..Usage::default()
                    },
                },
            )
            .await?;
        ledger
            .settle("invocation-1", InvocationOutcome::Succeeded)
            .await?;
        assert_eq!(store.revision().await?, main_before);

        candidate
            .view()
            .append("candidate", "assistant", "draft")
            .await?;
        candidate.promote().await?;
        assert_eq!(
            store.history("candidate", 1).await?[0].plain_text(),
            Some("draft")
        );

        let usage = ledger.session("session-1").await?;
        assert!(usage.historical_complete);
        assert_eq!(usage.invocation_count, 1);
        assert_eq!(usage.known_usage.input_tokens, Some(12));
        assert_eq!(usage.known_usage.output_tokens, Some(3));
        assert_eq!(usage.known_usage.cached_input_tokens, Some(2));
        assert_eq!(usage.known_usage.reasoning_output_tokens, None);
        assert!(!usage.component_complete.reasoning_output_tokens);
        assert_eq!(usage.incomplete_invocations, 1);
        drop(ledger);
        store.close().await
    }

    #[tokio::test]
    async fn markers_duplicates_and_conflicting_sequences_remain_honest() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        ledger.admit(start("resumed", "invocation-1")).await?;
        assert!(!ledger.session("resumed").await?.historical_complete);
        assert!(ledger.mark_new_session("resumed").await.is_err());
        ledger.admit(start("resumed", "invocation-1")).await?;
        let first = UsageObservation {
            sequence: 1,
            terminal: false,
            usage: Usage {
                input_tokens: Some(0),
                ..Usage::default()
            },
        };
        ledger.observe("invocation-1", first.clone()).await?;
        ledger.observe("invocation-1", first).await?;
        assert!(
            ledger
                .observe(
                    "invocation-1",
                    UsageObservation {
                        sequence: 1,
                        terminal: false,
                        usage: Usage {
                            input_tokens: Some(1),
                            ..Usage::default()
                        },
                    },
                )
                .await
                .is_err()
        );
        ledger
            .settle("invocation-1", InvocationOutcome::Cancelled)
            .await?;
        ledger
            .settle("invocation-1", InvocationOutcome::Cancelled)
            .await?;
        assert!(
            ledger
                .settle("invocation-1", InvocationOutcome::Succeeded)
                .await
                .is_err()
        );
        assert!(
            ledger
                .observe(
                    "invocation-1",
                    UsageObservation {
                        sequence: 2,
                        terminal: true,
                        usage: Usage::default(),
                    },
                )
                .await
                .is_err()
        );
        drop(ledger);
        store.close().await
    }

    async fn reopen(root: &crate::test_support::TempDir) -> Result<MemoryStore> {
        MemoryStore::open(crate::test_support::open_options(
            root.path().to_owned(),
            format!("project/{}", "0".repeat(64)),
        )?)
        .await
    }

    #[tokio::test]
    async fn receipts_and_preledger_marker_survive_reopen() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let store = reopen(&root).await?;
        let ledger = store.usage_ledger()?;
        ledger.admit(start("resumed", "invocation-1")).await?;
        ledger
            .observe(
                "invocation-1",
                UsageObservation {
                    sequence: 1,
                    terminal: true,
                    usage: Usage {
                        input_tokens: Some(7),
                        ..Usage::default()
                    },
                },
            )
            .await?;
        ledger
            .settle("invocation-1", InvocationOutcome::Succeeded)
            .await?;
        let usage_pool = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("usage branch pool missing")?;
        let retained: Vec<String> = sqlx::query_scalar("SELECT id FROM operations ORDER BY id")
            .fetch_all(usage_pool.as_ref())
            .await?;
        assert_eq!(retained.len(), 3, "later ledger writes erased a receipt");
        drop(usage_pool);
        drop(ledger);
        store.close().await?;

        let reopened = reopen(&root).await?;
        let ledger = reopened.usage_ledger()?;
        let session = ledger.session("resumed").await?;
        assert!(!session.historical_complete);
        assert_eq!(session.known_usage.input_tokens, Some(7));
        assert!(!session.component_complete.input_tokens);
        let usage_pool = reopened
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("reopened usage branch pool missing")?;
        let after: Vec<String> = sqlx::query_scalar("SELECT id FROM operations ORDER BY id")
            .fetch_all(usage_pool.as_ref())
            .await?;
        assert_eq!(after, retained, "reopen changed retained usage receipts");
        ledger.admit(start("resumed", "invocation-1")).await?;
        assert!(ledger.mark_new_session("resumed").await.is_err());
        assert!(
            ledger
                .observe(
                    "invocation-1",
                    UsageObservation {
                        sequence: 1,
                        terminal: true,
                        usage: Usage {
                            input_tokens: Some(8),
                            ..Usage::default()
                        },
                    },
                )
                .await
                .is_err()
        );
        drop(ledger);
        reopened.close().await
    }

    #[tokio::test]
    async fn history_window_is_branch_pinned_and_counts_empty_suffixes() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        store.append("history", "user", "one").await?;
        store.append("history", "assistant", "two").await?;
        let empty = store.history_window("history", 0).await?;
        assert!(empty.messages.is_empty());
        assert_eq!(empty.total_rows, 2);

        let candidate = store.begin_candidate("history window").await?;
        candidate
            .view()
            .append("history", "assistant", "candidate")
            .await?;
        let candidate_window = candidate.view().history_window("history", 2).await?;
        assert_eq!(candidate_window.total_rows, 3);
        assert_eq!(candidate_window.messages.len(), 2);
        assert_eq!(candidate_window.messages[1].plain_text(), Some("candidate"));
        assert_eq!(store.history_window("history", 2).await?.total_rows, 2);
        drop(candidate);
        store.close().await
    }

    #[tokio::test]
    async fn uncertain_committed_receipt_reconciles_without_replaying_usage() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let start = start("resumed", "invocation-1");
        let operation = Uuid::new_v4().to_string();
        let (mut connection, id) = owned_connection(&ledger.store.pool).await?;
        assert!(
            apply_change(
                &mut connection,
                &operation,
                Change::Admit(Box::new(start.clone())),
            )
            .await?
        );
        drop(connection);
        *ledger
            .store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock") = Some(Pending {
            pool: ledger.store.pool.clone(),
            connection: id,
            receipt: Receipt::Operation(operation),
        });
        assert_eq!(ledger.store.reconcile().await?, Some(true));
        ledger.admit(start).await?;
        assert_eq!(ledger.session("resumed").await?.invocation_count, 1);
        drop(ledger);
        store.close().await
    }

    #[tokio::test]
    async fn malformed_reserved_receipt_schema_refuses_writable_reopen() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let store = reopen(&root).await?;
        let usage_pool = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("usage ledger pool is unavailable")?;
        sqlx::query("ALTER TABLE operations DROP COLUMN label")
            .execute(usage_pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'malformed usage receipt schema', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(usage_pool.as_ref())
            .await?;
        drop(usage_pool);
        store.close().await?;
        assert!(reopen(&root).await.is_err());
        Ok(())
    }

    #[test]
    fn reducer_preserves_absent_components_and_observed_zeroes() -> Result<()> {
        let marker = Some(SessionMarker {
            format: FORMAT,
            session_id: "fresh".into(),
            historical_complete: true,
        });
        let absent = InvocationUsage {
            start: start("fresh", "absent"),
            usage: Usage {
                input_tokens: Some(9),
                output_tokens: Some(2),
                ..Usage::default()
            },
            last_usage_sequence: Some(1),
            terminal_usage: None,
            outcome: Some(InvocationOutcome::Failed),
            incomplete: true,
        };
        let absent_usage = fold_session("fresh", marker.clone(), &[absent])?;
        assert_eq!(absent_usage.known_usage.cached_input_tokens, None);
        assert_eq!(absent_usage.known_usage.reasoning_output_tokens, None);
        assert!(!absent_usage.component_complete.cached_input_tokens);
        assert!(!absent_usage.component_complete.reasoning_output_tokens);

        let zeroes = InvocationUsage {
            start: start("fresh", "zeroes"),
            usage: Usage {
                input_tokens: Some(0),
                output_tokens: Some(0),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
            },
            last_usage_sequence: Some(1),
            terminal_usage: Some(Usage {
                input_tokens: Some(0),
                output_tokens: Some(0),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
            }),
            outcome: Some(InvocationOutcome::Succeeded),
            incomplete: false,
        };
        let zero_usage = fold_session("fresh", marker, &[zeroes])?;
        assert_eq!(zero_usage.known_usage.cached_input_tokens, Some(0));
        assert_eq!(zero_usage.known_usage.reasoning_output_tokens, Some(0));
        assert!(zero_usage.component_complete.cached_input_tokens);
        assert!(zero_usage.component_complete.reasoning_output_tokens);
        Ok(())
    }

    #[test]
    fn reducer_marks_terminal_gaps_and_prices_known_normal_terms() -> Result<()> {
        let mut start = start("fresh", "priced");
        start.price_at_invocation = Some(PriceSchedule {
            basis: PriceBasis::ApiStandard {
                api_model: "model".into(),
            },
            source: SourceCitation {
                url: "https://example.invalid/prices".into(),
                checked_on: "2026-09-16".into(),
            },
            input_per_million_usd: "1".into(),
            cached_input_per_million_usd: Some("0.5".into()),
            output_per_million_usd: "2".into(),
            long_context_tier: Some(LongContextTier {
                input_tokens_over: 10,
                input_multiplier: "3".into(),
                cached_input_multiplier: "4".into(),
                output_multiplier: "5".into(),
            }),
            cache_write: Some(CacheWriteTerms::PerMillionUsd { value: "1".into() }),
            promotional_available_at_least_through: None,
        });
        let record = InvocationUsage {
            start,
            usage: Usage {
                input_tokens: Some(1_000),
                output_tokens: Some(1_000),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
            },
            last_usage_sequence: Some(1),
            terminal_usage: Some(Usage {
                input_tokens: None,
                output_tokens: Some(1_000),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
            }),
            outcome: Some(InvocationOutcome::Succeeded),
            incomplete: true,
        };
        let usage = fold_session(
            "fresh",
            Some(SessionMarker {
                format: FORMAT,
                session_id: "fresh".into(),
                historical_complete: true,
            }),
            &[record],
        )?;
        assert_eq!(usage.incomplete_invocations, 1);
        assert!(!usage.component_complete.input_tokens);
        assert_eq!(usage.api_standard.known_usd.as_deref(), Some("0.013000"));
        assert!(usage.api_standard.incomplete);
        // The long-context tier applied here; only the unmodelled cache-write
        // rate is named, so a reader learns what a reprice would still add.
        assert_eq!(
            usage.api_standard.unapplied,
            vec![UnappliedPriceTerm::CacheWriteRate]
        );
        Ok(())
    }

    #[test]
    fn reducer_keeps_complete_and_partial_frozen_price_subtotals_honest() -> Result<()> {
        let price = |basis, cached_input_per_million_usd| PriceSchedule {
            basis,
            source: SourceCitation {
                url: "https://example.invalid/prices".into(),
                checked_on: "2026-09-16".into(),
            },
            input_per_million_usd: "1".into(),
            cached_input_per_million_usd,
            output_per_million_usd: "2".into(),
            long_context_tier: Some(LongContextTier {
                input_tokens_over: 1_000,
                input_multiplier: "3".into(),
                cached_input_multiplier: "4".into(),
                output_multiplier: "5".into(),
            }),
            cache_write: None,
            promotional_available_at_least_through: None,
        };
        let usage = Usage {
            input_tokens: Some(100),
            output_tokens: Some(20),
            cached_input_tokens: Some(0),
            reasoning_output_tokens: Some(0),
        };
        let record = |invocation_id, basis| InvocationUsage {
            start: InvocationStart {
                price_at_invocation: Some(price(basis, Some("0.5".into()))),
                ..start("fresh", invocation_id)
            },
            usage: usage.clone(),
            last_usage_sequence: Some(1),
            terminal_usage: Some(usage.clone()),
            outcome: Some(InvocationOutcome::Succeeded),
            incomplete: false,
        };
        let marker = Some(SessionMarker {
            format: FORMAT,
            session_id: "fresh".into(),
            historical_complete: true,
        });
        let standard = fold_session(
            "fresh",
            marker.clone(),
            &[record(
                "standard",
                PriceBasis::ApiStandard {
                    api_model: "model".into(),
                },
            )],
        )?;
        assert_eq!(standard.api_standard.known_usd.as_deref(), Some("0.000140"));
        assert!(!standard.api_standard.incomplete);
        assert!(standard.api_standard.unapplied.is_empty());

        let equivalent = fold_session(
            "fresh",
            marker.clone(),
            &[record(
                "equivalent",
                PriceBasis::ApiEquivalent {
                    api_model: "subscription-model".into(),
                },
            )],
        )?;
        assert_eq!(
            equivalent.api_equivalent.known_usd.as_deref(),
            Some("0.000140")
        );
        assert!(!equivalent.api_equivalent.incomplete);

        let pre_ledger = fold_session(
            "fresh",
            Some(SessionMarker {
                format: FORMAT,
                session_id: "fresh".into(),
                historical_complete: false,
            }),
            &[record(
                "pre-ledger",
                PriceBasis::ApiStandard {
                    api_model: "model".into(),
                },
            )],
        )?;
        assert_eq!(
            pre_ledger.api_standard.known_usd.as_deref(),
            Some("0.000140")
        );
        assert!(pre_ledger.api_standard.incomplete);
        // Pre-ledger history is its own reported gap, not an unapplied price term.
        assert!(pre_ledger.api_standard.unapplied.is_empty());

        let partial_usage = Usage {
            input_tokens: Some(100),
            output_tokens: Some(20),
            cached_input_tokens: Some(10),
            reasoning_output_tokens: Some(0),
        };
        let partial = InvocationUsage {
            start: InvocationStart {
                price_at_invocation: Some(price(
                    PriceBasis::ApiStandard {
                        api_model: "model".into(),
                    },
                    None,
                )),
                ..start("fresh", "unknown-cached-rate")
            },
            usage: partial_usage.clone(),
            last_usage_sequence: Some(1),
            terminal_usage: Some(partial_usage),
            outcome: Some(InvocationOutcome::Succeeded),
            incomplete: false,
        };
        let partial = fold_session("fresh", marker.clone(), &[partial])?;
        assert_eq!(partial.api_standard.known_usd.as_deref(), Some("0.000130"));
        assert!(partial.api_standard.incomplete);
        assert_eq!(
            partial.api_standard.unapplied,
            vec![UnappliedPriceTerm::CachedInputRate]
        );

        // A tier that cannot be decided is named, never applied in part: the
        // raw components and the frozen price stay on the record for a reprice.
        let undecidable = InvocationUsage {
            start: InvocationStart {
                price_at_invocation: Some(price(
                    PriceBasis::ApiStandard {
                        api_model: "model".into(),
                    },
                    Some("0.5".into()),
                )),
                ..start("fresh", "unknown-input-count")
            },
            usage: Usage {
                input_tokens: None,
                output_tokens: Some(20),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
            },
            last_usage_sequence: Some(1),
            terminal_usage: Some(Usage {
                input_tokens: None,
                output_tokens: Some(20),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: Some(0),
            }),
            outcome: Some(InvocationOutcome::Succeeded),
            incomplete: false,
        };
        let undecidable_record = undecidable.clone();
        let undecidable = fold_session("fresh", marker, &[undecidable])?;
        assert_eq!(undecidable.api_standard.known_usd, None);
        assert_eq!(
            undecidable.api_standard.unapplied,
            vec![UnappliedPriceTerm::LongContextTier]
        );
        assert!(
            undecidable_record
                .start
                .price_at_invocation
                .as_ref()
                .is_some_and(|price| price.long_context_tier.is_some())
        );
        assert_eq!(undecidable_record.usage.output_tokens, Some(20));
        Ok(())
    }
}
