//! Private, branch-pinned provider usage accounting.
//!
//! The permanent branch is deliberately reachable only through `UsageLedger`.
//! It shares the project's writer serialization and receipt reconciliation, but
//! never gives a caller a mutable `MemoryStore` view of that branch.
//!
//! # Content-bound validation
//!
//! A writable open publishes the ledger only when every owned row (every
//! `state` row under `kuru.usage.v1/`) of the exact content being activated
//! passes [`validate_owned_row`]. That is proved either by a full scan or by a
//! validation record on the branch head: the commit-message trailer
//! `Kuru-Usage-State: <validator id> <state hash>`, where the hash is
//! `DOLT_HASHOF_TABLE('state')` (content-addressed and history-independent).
//! A record is accepted only when it is on HEAD, names exactly [`VALIDATOR`]
//! and equals the live table hash; anything else is Missing, which means scan
//! and re-record, never refuse.
//!
//! Each ledger write starts from validated content (the open's scan or record,
//! held in memory), validates each row it writes with the same function before
//! its `INSERT`, deletes nothing, and records the hash of the content it
//! produced in the same `DOLT_COMMIT`. By induction every recorded content is
//! valid. The argument rests on three invariants, each tested:
//!
//! 1. **Row-locality.** [`validate_owned_row`]'s verdict depends only on one
//!    key and its value. Adding a cross-row check, or any other input, to the
//!    scan requires bumping the `v1` stem of [`VALIDATOR`] and redoing this
//!    argument.
//! 2. **Same function.** Writes call [`validate_owned_row`] from
//!    `put_state_tx`, the only statement that writes owned rows; there is no
//!    lookalike and no `DELETE`.
//! 3. **No other writer of owned rows.** Migration publications (which do not
//!    touch `state`), template adoption (`kuru_instance` only) and foreign or
//!    manual commits carry no valid record, so they fall to the full scan.

use super::*;
use crate::open_timeline::{self, Event};
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
// Key-range paging. `state.key` is VARBINARY, so these compare and order by
// bytes; Dolt derives a primary-key range from them only while the column
// stays unwrapped (no `BINARY(...)`, which also forced a sort of every row).
// The first page and later pages are separate strings so that no bind-time
// `OR` decides whether the range survives planning.
const RANGE_FIRST_PAGE: &str =
    "SELECT `key`, value FROM state WHERE `key` >= ? AND `key` < ? ORDER BY `key` LIMIT ?";
const RANGE_NEXT_PAGE: &str =
    "SELECT `key`, value FROM state WHERE `key` > ? AND `key` < ? ORDER BY `key` LIMIT ?";
const RANGE_ANY_FOR_UPDATE: &str =
    "SELECT 1 FROM state WHERE `key` >= ? AND `key` < ? LIMIT 1 FOR UPDATE";
const RANGE_ANY: &str = "SELECT 1 FROM state WHERE `key` >= ? AND `key` < ? LIMIT 1";

/// The identity of [`validate_owned_row`] and the decoders it calls, bound to
/// the release: every release boundary, in either direction, costs one full
/// scan. Bump the `v1` stem together with the golden corpus whenever a verdict
/// changes within one version string.
pub(crate) const VALIDATOR: &str = concat!("kuru.usage.state.v1+", env!("CARGO_PKG_VERSION"));
const RECORD_TRAILER: &str = "Kuru-Usage-State:";
const RECORD_SUBJECT: &str = "usage ledger validation v1";
const RECORD_MESSAGE_MAX: usize = 512;
const RECORD_VALIDATOR_MAX: usize = 128;
const STATE_HASH_LEN: usize = 32;

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
        let indexed = KeyRange::prefix(&session_index_prefix(session_id))?;
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page =
                session_index_page(self.store.pool.as_ref(), &indexed, after.as_deref()).await?;
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
            after = page.last().map(|(key, _)| key.clone());
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
        self.inspect_proof_unguarded(proof).await
    }

    /// The same natural-key read without the write guard or reconciliation.
    /// Each positive proof is one read of a final-state key that its write
    /// publishes in the same `DOLT_COMMIT`, so `true` is durable committed
    /// evidence even while that write is still running. `false` proves nothing.
    pub(crate) async fn inspect_proof_unguarded(&self, proof: &UsageProof) -> Result<bool> {
        proof.validate()?;
        self.store.readable()?;
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
        // Read only after reconciliation, which may have re-derived it.
        let validated = self
            .store
            .shared
            .usage_validated
            .lock()
            .expect("usage validated lock")
            .clone()
            .ok_or(UsageLedgerStateChanged)?;
        let store = self.store.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let operation = Uuid::new_v4().to_string();
            // One budget, taken before the acquisition, bounds the
            // acquisition, the write and its session's return.
            let deadline = write_deadline();
            let (mut connection, id) = write_session(&store.pool, deadline).await?;
            *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: store.pool.clone(),
                connection: id,
                receipt: Receipt::UsageOperation(operation.clone()),
            });
            let result = crate::pool::within_until(
                deadline,
                apply_change(&mut connection, &operation, &validated, change),
            )
            .await;
            match result {
                // Nothing changed and the transaction rolled back: the
                // session is clean and the outcome receipted.
                Ok(Ok(None)) => {
                    *store.shared.uncertain.lock().expect("uncertain lock") = None;
                    connection.settle_receipted(deadline).await;
                    Ok(())
                }
                // Committed: the session is clean and the outcome receipted.
                Ok(Ok(Some(produced))) => {
                    *store.shared.uncertain.lock().expect("uncertain lock") = None;
                    // Published only after COMMIT returned.
                    *store
                        .shared
                        .usage_validated
                        .lock()
                        .expect("usage validated lock") = Some(produced);
                    connection.settle_receipted(deadline).await;
                    Ok(())
                }
                Ok(Err(error)) if error.is::<UsageLedgerStateChanged>() => {
                    // The precondition rolled back before any read or write;
                    // settle the receipt, then fail closed after its
                    // re-derivation so later writes refuse until a reopen.
                    // Not a receipted success: end the session first.
                    drop(connection);
                    let settled = store.resolve_uncertain().await;
                    *store
                        .shared
                        .usage_validated
                        .lock()
                        .expect("usage validated lock") = None;
                    ensure!(
                        settled? == Some(false),
                        "a refused usage ledger write produced a receipt"
                    );
                    Err(error)
                }
                other => {
                    // Not a receipted success: end the session first.
                    drop(connection);
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
    let exists: Vec<String> = crate::pool::within(
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
        crate::pool::within(
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
    open_timeline::stamp(Event::UsagePool);
    // Pre-upgrade validation: an invalid ledger is never migrated.
    validate_branch_state(pool.as_ref()).await?;
    let before = bound_check(pool.as_ref()).await?;
    open_timeline::stamp(Event::UsageBound);
    let scanned = if before.bound {
        None
    } else {
        Some(validate_owned_rows(pool.as_ref(), &before.state_hash).await?)
    };
    // 0 on a recorded reopen: the rows decoded between usage-bound and here.
    open_timeline::usage_rows(scanned.unwrap_or(0));
    open_timeline::stamp(Event::UsageScan1);
    migrations::upgrade_usage(&store.shared.server, pool.as_ref()).await?;
    open_timeline::stamp(Event::UsageUpgrade);
    migrations::validate_usage(pool.as_ref()).await?;
    open_timeline::stamp(Event::UsageValidate);
    // D: the old second scan's flat checks stay; its owned walk runs only
    // when the upgrade changed `state` (no usage migration does today).
    validate_branch_state(pool.as_ref()).await?;
    let after = bound_check(pool.as_ref()).await?;
    let rescanned = if after.state_hash == before.state_hash {
        None
    } else {
        Some(validate_owned_rows(pool.as_ref(), &after.state_hash).await?)
    };
    open_timeline::stamp(Event::UsageScan2);
    let recorded = if after.bound {
        false
    } else {
        let owned = match rescanned.or(scanned) {
            Some(rows) => rows > 0,
            None => any_owned(pool.as_ref()).await?,
        };
        if owned {
            record_validation(store, &pool, &after).await?;
        }
        owned
    };
    open_timeline::stamp(Event::UsageRecord);
    #[cfg(test)]
    {
        *store.shared.usage_open.lock().expect("usage open lock") = Some(UsageOpen {
            bound: before.bound,
            scanned,
            rescanned,
            recorded,
        });
    }
    #[cfg(not(test))]
    let _ = recorded;
    *store
        .shared
        .usage_validated
        .lock()
        .expect("usage validated lock") = Some(after.state_hash);
    *store.shared.usage_pool.lock().expect("usage pool lock") = Some(pool);
    Ok(())
}

/// What one writable open's usage establishment did.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsageOpen {
    /// HEAD recorded this validator and the live content before the upgrade.
    pub(crate) bound: bool,
    /// Rows the pre-upgrade full scan decoded; `None` when Bound.
    pub(crate) scanned: Option<u64>,
    /// Rows a post-upgrade rescan decoded; `None` when `state` was unchanged.
    pub(crate) rescanned: Option<u64>,
    /// Whether the open wrote a record commit.
    pub(crate) recorded: bool,
}

/// Record validated content on the usage head the way `establish` does,
/// through `store`'s pool (a lost-reply fixture routes it), under the write
/// guard. The head must lack its record.
#[cfg(test)]
pub(super) async fn record_unrecorded_head(store: &MemoryStore) -> Result<()> {
    let _guard = store.shared.write.lock().await;
    let check = bound_check(store.pool.as_ref()).await?;
    ensure!(!check.bound, "the usage head already records its content");
    record_validation(store, &store.pool, &check).await
}

/// Whether the usage head records this validator and the live content.
#[cfg(test)]
pub(super) async fn head_records_live_state(pool: &MemoryPool) -> Result<bool> {
    Ok(bound_check(pool).await?.bound)
}

/// The live `state` content hash.
#[cfg(test)]
pub(super) async fn live_state_hash(pool: &MemoryPool) -> Result<String> {
    state_hash(pool).await
}

/// The branch's working set, schema history and old receipts: flat checks
/// that stay on every open.
async fn validate_branch_state(pool: &MemoryPool) -> Result<()> {
    let dirty: i64 = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(pool),
    )
    .await
    .context("usage ledger working-set validation deadline exceeded")??;
    ensure!(dirty == 0, "usage ledger branch has uncommitted changes");
    let version = migrations::validate_historical(pool).await?;
    if version <= 3 {
        let old_receipts: Vec<(String, String)> = crate::pool::within(
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
    Ok(())
}

/// The full scan: every owned row through [`validate_owned_row`]. The scan
/// validates exactly the content named `expected`, read before it and
/// checked again after it. Returns the number of owned rows decoded.
async fn validate_owned_rows(pool: &MemoryPool, expected: &str) -> Result<u64> {
    let owned = KeyRange::prefix(OWNED_PREFIX)?;
    let mut after: Option<Vec<u8>> = None;
    let mut decoded = 0_u64;
    loop {
        let rows = owned_state_page(pool, &owned, after.as_deref()).await?;
        if rows.is_empty() {
            break;
        }
        decoded = decoded.saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
        for (key, value) in &rows {
            validate_owned_row(key, value)?;
        }
        after = rows.last().map(|(key, _)| key.clone());
    }
    ensure!(
        state_hash(pool).await? == expected,
        "usage ledger state changed while it was being validated"
    );
    Ok(decoded)
}

/// Whether any owned row exists: one primary-key range probe.
async fn any_owned(pool: &MemoryPool) -> Result<bool> {
    let owned = KeyRange::prefix(OWNED_PREFIX)?;
    Ok(crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query(RANGE_ANY)
            .bind(owned.start.as_slice())
            .bind(owned.end.as_slice())
            .fetch_optional(pool),
    )
    .await
    .context("usage ledger owned-row probe deadline exceeded")??
    .is_some())
}

/// Record validated content on a head that lacks its record: one empty
/// commit in a short transaction, registered as an uncertain outcome and
/// resolved inline under the open's write guard. Either definite outcome
/// keeps the open: the scan already validated this content, and a missing
/// record only means the next open scans again.
async fn record_validation(
    store: &MemoryStore,
    pool: &Arc<MemoryPool>,
    check: &BoundCheck,
) -> Result<()> {
    let deadline = write_deadline();
    let (mut connection, id) = owned_connection(pool, deadline).await?;
    *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
        pool: pool.clone(),
        connection: id,
        receipt: Receipt::UsageValidation {
            base_head: check.head.clone(),
            state_hash: check.state_hash.clone(),
        },
    });
    let result = crate::pool::within_until(deadline, async {
        let mut transaction = connection.begin().await?;
        let head: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
            .fetch_one(&mut *transaction)
            .await?;
        let current = state_hash(&mut *transaction).await?;
        if head != check.head || current != check.state_hash {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query("CALL DOLT_COMMIT('--allow-empty', '--message', ?, '--author', ?)")
            .bind(record_message(&check.state_hash))
            .bind(AUTHOR)
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok::<_, anyhow::Error>(true)
    })
    .await;
    drop(connection);
    match result {
        Ok(Ok(true)) => {
            *store.shared.uncertain.lock().expect("uncertain lock") = None;
            Ok(())
        }
        Ok(Ok(false)) => {
            store.resolve_uncertain().await?;
            bail!("usage ledger state changed while it was being validated")
        }
        _ => {
            store.resolve_uncertain().await?;
            Ok(())
        }
    }
}

/// The validation record line for `state_hash` under this binary's validator.
fn record_trailer(state_hash: &str) -> String {
    format!("{RECORD_TRAILER} {VALIDATOR} {state_hash}")
}

/// A ledger write's commit message: its unchanged subject, then the record of
/// the content it produced.
fn write_message(operation: &str, state_hash: &str) -> String {
    format!(
        "usage ledger v1 [{operation}]\n\n{}",
        record_trailer(state_hash)
    )
}

/// The open's empty record commit's message.
fn record_message(state_hash: &str) -> String {
    format!("{RECORD_SUBJECT}\n\n{}", record_trailer(state_hash))
}

/// A validation record parsed from one commit message.
#[derive(Debug, PartialEq, Eq)]
struct UsageRecord<'a> {
    validator: &'a str,
    state_hash: &'a str,
}

/// Parse a commit message's validation record strictly. `None` for every
/// malformed form: an oversize message, no or more than one record line, a
/// record that is not the final line, a validator that is not short printable
/// ASCII, or a hash that is not exactly 32 characters of Dolt's `[0-9a-v]`.
fn parse_record(message: &str) -> Option<UsageRecord<'_>> {
    if message.len() > RECORD_MESSAGE_MAX {
        return None;
    }
    let body = message.strip_suffix('\n').unwrap_or(message);
    let mut lines = body
        .split('\n')
        .filter(|line| line.contains(RECORD_TRAILER));
    let line = lines.next()?;
    if lines.next().is_some() || !body.ends_with(line) {
        return None;
    }
    let mut fields = line
        .strip_prefix(RECORD_TRAILER)?
        .strip_prefix(' ')?
        .split(' ');
    let (validator, state_hash) = (fields.next()?, fields.next()?);
    if fields.next().is_some()
        || validator.is_empty()
        || validator.len() > RECORD_VALIDATOR_MAX
        || !validator.bytes().all(|byte| byte.is_ascii_graphic())
        || !state_hash_valid(state_hash)
    {
        return None;
    }
    Some(UsageRecord {
        validator,
        state_hash,
    })
}

fn state_hash_valid(hash: &str) -> bool {
    hash.len() == STATE_HASH_LEN
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
}

/// The content hash of the `state` table as `executor` sees it, including a
/// transaction's own uncommitted writes.
async fn state_hash<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::MySql>,
) -> Result<String> {
    let hash: String = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT DOLT_HASHOF_TABLE('state')").fetch_one(executor),
    )
    .await
    .context("usage ledger state hash deadline exceeded")??;
    ensure!(
        state_hash_valid(&hash),
        "usage ledger state hash is not a Dolt hash"
    );
    Ok(hash)
}

/// The usage branch head as the validation record sees it.
struct BoundCheck {
    head: String,
    state_hash: String,
    /// True only when HEAD's message records exactly this binary's
    /// validator and the live `state` hash (Bound); false is Missing.
    bound: bool,
}

/// Read HEAD, HEAD's message and the live `state` hash. Never an error for a
/// missing, foreign or malformed record: those read as Missing.
async fn bound_check(pool: &MemoryPool) -> Result<BoundCheck> {
    let head = revision(pool).await?;
    // HEAD-first order is observed, not documented; the equality check
    // below is what makes the first row HEAD's message. The message is
    // truncated past the record limit so a foreign message cannot grow the
    // read, and parse_record then refuses it as oversize.
    let (logged, message): (String, String) = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_as("SELECT commit_hash, LEFT(message, ?) FROM dolt_log LIMIT 1")
            .bind(i64::try_from(RECORD_MESSAGE_MAX + 1).unwrap_or(i64::MAX))
            .fetch_one(pool),
    )
    .await
    .context("usage ledger head message deadline exceeded")??;
    let live = state_hash(pool).await?;
    let bound = logged == head
        && parse_record(&message)
            .is_some_and(|record| record.validator == VALIDATOR && record.state_hash == live);
    Ok(BoundCheck {
        head,
        state_hash: live,
        bound,
    })
}

/// Reconcile the open's record commit, after its SQL session has ended:
/// committed only when HEAD records exactly this content under this
/// validator and HEAD's only parent is the base. Anything else, HEAD still at
/// the base or a head that diverged from both, is not proven and reads as a
/// missing record, never an error: the record commit is empty, so its
/// outcome changes no row and nothing replays it, and a missing record only
/// means the next open scans and records again.
pub(super) async fn validation_committed(
    pool: &MemoryPool,
    base_head: &str,
    state_hash: &str,
) -> Result<bool> {
    let check = bound_check(pool).await?;
    if check.head == base_head || !check.bound || check.state_hash != state_hash {
        return Ok(false);
    }
    let parents: Vec<String> = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? ORDER BY parent_index LIMIT 2",
        )
        .bind(&check.head)
        .fetch_all(pool),
    )
    .await
    .context("usage validation record parent deadline exceeded")??;
    Ok(parents == [base_head])
}

/// After reconciliation settled a usage write or record commit, re-derive
/// the validated content: a clean branch whose head records the live content
/// (Bound), or whose live content is still exactly the content already
/// validated, keeps the ledger writable; anything else refuses writes until a
/// reopen.
pub(super) async fn rederive_validated(shared: &Shared, pool: &MemoryPool) -> Result<()> {
    let dirty: i64 = crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status").fetch_one(pool),
    )
    .await
    .context("usage ledger working-set validation deadline exceeded")??;
    let check = bound_check(pool).await?;
    let mut validated = shared.usage_validated.lock().expect("usage validated lock");
    let unchanged = validated.as_deref() == Some(check.state_hash.as_str());
    *validated = (dirty == 0 && (check.bound || unchanged)).then_some(check.state_hash);
    Ok(())
}

/// The one owned-row validator: the open scan calls it for every row under
/// `kuru.usage.v1/`, and every ledger write calls it for each row it is about
/// to `INSERT`. It is row-local by contract (see the module documentation):
/// its verdict depends only on this key and value.
fn validate_owned_row(key: &[u8], value: &str) -> Result<()> {
    let key = std::str::from_utf8(key).context("usage ledger key is not UTF-8")?;
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
            key == observation_key(&observation.invocation_id, observation.observation.sequence),
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
    Ok(())
}

/// Apply one ledger change in one short transaction. Returns the `state`
/// hash the committed write produced, or `None` when nothing changed.
/// `validated` is the content this open validated: the write refuses with
/// [`UsageLedgerStateChanged`] before any other read or write when the
/// branch's `state` no longer has that hash.
async fn apply_change(
    connection: &mut MySqlConnection,
    operation: &str,
    validated: &str,
    change: Change,
) -> Result<Option<String>> {
    let mut transaction = connection.begin().await?;
    let current = state_hash(&mut *transaction).await?;
    if current != validated {
        transaction.rollback().await?;
        return Err(UsageLedgerStateChanged.into());
    }
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
        return Ok(None);
    }
    let version: i32 = sqlx::query_scalar("SELECT version FROM kuru_schema WHERE id = 1")
        .fetch_one(&mut *transaction)
        .await?;
    ensure!(
        version == migrations::USAGE_CURRENT_VERSION,
        "usage ledger branch requires retained receipt schema before writing"
    );
    sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
        .bind(operation)
        .bind("usage ledger v1")
        .execute(&mut *transaction)
        .await?;
    // The content this write produced, including its own uncommitted puts;
    // `state` is independent of the `operations` receipt.
    let produced = state_hash(&mut *transaction).await?;
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(write_message(operation, &produced))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Some(produced))
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
    // The same function the open scan applies: a write can never commit a
    // row that a later full scan would refuse.
    validate_owned_row(key.as_bytes(), &encoded)?;
    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)")
        .bind(key.as_bytes())
        .bind(encoded)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn read_marker(pool: &MemoryPool, session_id: &str) -> Result<Option<SessionMarker>> {
    let value: Option<String> = crate::pool::within(
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
    transaction: &mut MySqlConnection,
    session_id: &str,
) -> Result<bool> {
    let index = KeyRange::prefix(&session_index_prefix(session_id))?;
    let (sql, [low, high]) = index.any_for_update();
    Ok(sqlx::query(sql)
        .bind(low)
        .bind(high)
        .fetch_optional(&mut *transaction)
        .await?
        .is_some())
}

async fn read_state(pool: &MemoryPool, key: &str) -> Result<Option<String>> {
    crate::pool::within(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
            .bind(key.as_bytes())
            .fetch_optional(pool),
    )
    .await
    .context("usage ledger state read deadline exceeded")?
    .map_err(Into::into)
}

/// The half-open byte range `[start, end)` that holds exactly the keys
/// beginning with one prefix.
struct KeyRange {
    start: Vec<u8>,
    end: Vec<u8>,
}

impl KeyRange {
    fn prefix(prefix: &str) -> Result<Self> {
        let start = prefix.as_bytes().to_vec();
        let end =
            prefix_upper_bound(&start).context("usage ledger key prefix has no upper bound")?;
        Ok(Self { start, end })
    }

    /// One page's exact SQL and its two byte arguments, in placeholder order;
    /// `PAGE_SIZE` binds last.
    fn page<'a>(&'a self, after: Option<&'a [u8]>) -> (&'static str, [&'a [u8]; 2]) {
        match after {
            None => (
                RANGE_FIRST_PAGE,
                [self.start.as_slice(), self.end.as_slice()],
            ),
            Some(after) => (RANGE_NEXT_PAGE, [after, self.end.as_slice()]),
        }
    }

    /// The exact SQL and byte arguments that lock-read whether any key lies
    /// in the range, without counting the rows.
    fn any_for_update(&self) -> (&'static str, [&[u8]; 2]) {
        (
            RANGE_ANY_FOR_UPDATE,
            [self.start.as_slice(), self.end.as_slice()],
        )
    }
}

/// The least byte string above every string that begins with `prefix`: the
/// prefix up to its last byte below 0xFF, with that byte incremented. None
/// when no byte can be incremented (empty or all 0xFF).
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let last = prefix.iter().rposition(|byte| *byte != u8::MAX)?;
    let mut end = prefix[..=last].to_vec();
    end[last] += 1;
    Some(end)
}

async fn range_page(
    pool: &MemoryPool,
    range: &KeyRange,
    after: Option<&[u8]>,
) -> sqlx::Result<Vec<(Vec<u8>, String)>> {
    let (sql, [low, high]) = range.page(after);
    sqlx::query_as(sql)
        .bind(low)
        .bind(high)
        .bind(PAGE_SIZE)
        .fetch_all(pool)
        .await
}

async fn session_index_page(
    pool: &MemoryPool,
    index: &KeyRange,
    after: Option<&[u8]>,
) -> Result<Vec<(Vec<u8>, String)>> {
    crate::pool::within(QUERY_TIMEOUT, range_page(pool, index, after))
        .await
        .context("usage ledger session index deadline exceeded")?
        .map_err(Into::into)
}

async fn owned_state_page(
    pool: &MemoryPool,
    owned: &KeyRange,
    after: Option<&[u8]>,
) -> Result<Vec<(Vec<u8>, String)>> {
    crate::pool::within(QUERY_TIMEOUT, range_page(pool, owned, after))
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
        MemoryStore::open(
            crate::test_support::warmed_open_options(
                root.path().to_owned(),
                format!("project/{}", "0".repeat(64)),
            )
            .await?,
        )
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

    fn validated(store: &MemoryStore) -> Option<String> {
        store
            .shared
            .usage_validated
            .lock()
            .expect("usage validated lock")
            .clone()
    }

    fn set_pending(ledger: &UsageLedger, connection: u64, receipt: Receipt) {
        *ledger
            .store
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock") = Some(Pending {
            pool: ledger.store.pool.clone(),
            connection,
            receipt,
        });
    }

    // T13 for a ledger write, committed case: the reply is lost after the
    // commit; reconciliation proves the receipt, re-derives the validated
    // content from HEAD's record and the next write succeeds.
    #[tokio::test]
    async fn uncertain_committed_receipt_reconciles_without_replaying_usage() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let start = start("resumed", "invocation-1");
        let operation = Uuid::new_v4().to_string();
        let before = validated(&store).context("open validated nothing")?;
        let (mut connection, id) = owned_connection(&ledger.store.pool, write_deadline()).await?;
        let produced = apply_change(
            &mut connection,
            &operation,
            &before,
            Change::Admit(Box::new(start.clone())),
        )
        .await?
        .context("the admission changed nothing")?;
        drop(connection);
        ensure!(produced != before);
        ensure!(
            validated(&store).as_ref() == Some(&before),
            "the in-memory hash moved without the writer's answer"
        );
        set_pending(&ledger, id, Receipt::UsageOperation(operation));
        assert_eq!(ledger.store.reconcile().await?, Some(true));
        assert_eq!(validated(&store), Some(produced));
        ledger.admit(start).await?;
        assert_eq!(ledger.session("resumed").await?.invocation_count, 1);
        ledger.admit(self::start("resumed", "invocation-2")).await?;
        assert_eq!(ledger.session("resumed").await?.invocation_count, 2);
        drop(ledger);
        store.close().await
    }

    // T13 for a ledger write, not-committed case: the transaction never
    // commits; reconciliation proves no receipt, keeps the validated content
    // (the live hash is unchanged) and the retry succeeds.
    #[tokio::test]
    async fn uncertain_uncommitted_receipt_keeps_the_ledger_writable() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let before = validated(&store).context("open validated nothing")?;
        let operation = Uuid::new_v4().to_string();
        let (mut connection, id) = owned_connection(&ledger.store.pool, write_deadline()).await?;
        {
            let mut transaction = connection.begin().await?;
            put_state_tx(
                &mut transaction,
                &session_key("lost"),
                &SessionMarker {
                    format: FORMAT,
                    session_id: "lost".into(),
                    historical_complete: true,
                },
            )
            .await?;
            // Dropped without COMMIT, as a write whose connection died.
        }
        drop(connection);
        set_pending(&ledger, id, Receipt::UsageOperation(operation));
        assert_eq!(ledger.store.reconcile().await?, Some(false));
        assert_eq!(validated(&store), Some(before));
        ledger.mark_new_session("lost").await?;
        ensure!(ledger.session("lost").await?.historical_complete);
        drop(ledger);
        store.close().await
    }

    // T6 (design T12): a foreign commit changes `state` between the open and
    // a write. The write refuses with the typed error, commits nothing and
    // inserts no receipt; later writes refuse too; a reopen scans and the
    // ledger writes again.
    #[tokio::test]
    async fn writes_refuse_after_state_changed_outside_the_writer() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let store = reopen(&root).await?;
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session("before").await?;
        let pool = ledger.store.pool.clone();
        // A non-owned key: any change to `state` breaks the precondition.
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(b"outside/the/ledger".as_slice())
            .bind("foreign")
            .execute(pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'foreign state change', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        let head = revision(pool.as_ref()).await?;
        let receipts = |pool: Arc<MemoryPool>| async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM operations")
                .fetch_one(pool.as_ref())
                .await
        };
        let receipts_before = receipts(pool.clone()).await?;
        for attempt in ["first", "later"] {
            let error = ledger.mark_new_session(attempt).await.unwrap_err();
            ensure!(
                error.is::<UsageLedgerStateChanged>(),
                "{attempt}: unexpected refusal {error:#}"
            );
            ensure!(
                format!("{error:#}").contains(
                    "usage ledger state changed outside its writer since validation; reopen to revalidate"
                ),
                "{error:#}"
            );
            ensure!(validated(&store).is_none());
            ensure!(
                revision(pool.as_ref()).await? == head,
                "{attempt} committed"
            );
            ensure!(receipts(pool.clone()).await? == receipts_before);
            ensure!(
                ledger
                    .store
                    .shared
                    .uncertain
                    .lock()
                    .expect("uncertain lock")
                    .is_none()
            );
        }
        drop(pool);
        drop(ledger);
        store.close().await?;

        let reopened = reopen(&root).await?;
        let ledger = reopened.usage_ledger()?;
        ledger.mark_new_session("after").await?;
        ensure!(ledger.session("after").await?.historical_complete);
        drop(ledger);
        reopened.close().await
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

    const SEEDED_SESSION: &str = "seeded-session";
    const SEEDED_INVOCATION: &str = "seeded-invocation";

    /// A closed store whose usage branch holds one row of each owned class,
    /// all written by the ledger's own write path, and the values it wrote.
    struct Seeded {
        _root: crate::test_support::TempDir,
        options: OpenOptions,
        usage_head: String,
        marker: String,
        record: String,
        observation: String,
        index: String,
    }

    async fn seeded() -> Result<Seeded> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", "0".repeat(64)),
        )
        .await?;
        let store = MemoryStore::open(options.clone()).await?;
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session(SEEDED_SESSION).await?;
        ledger
            .admit(start(SEEDED_SESSION, SEEDED_INVOCATION))
            .await?;
        ledger
            .observe(
                SEEDED_INVOCATION,
                UsageObservation {
                    sequence: 1,
                    terminal: true,
                    usage: Usage {
                        input_tokens: Some(5),
                        ..Usage::default()
                    },
                },
            )
            .await?;
        ledger
            .settle(SEEDED_INVOCATION, InvocationOutcome::Succeeded)
            .await?;
        let pool = ledger.store.pool.clone();
        let owned = |key: String| {
            let pool = pool.clone();
            async move {
                read_state(pool.as_ref(), &key)
                    .await?
                    .with_context(|| format!("the write path did not produce {key}"))
            }
        };
        let seeded = Seeded {
            usage_head: revision(pool.as_ref()).await?,
            marker: owned(session_key(SEEDED_SESSION)).await?,
            record: owned(record_key(SEEDED_INVOCATION)).await?,
            observation: owned(observation_key(SEEDED_INVOCATION, 1)).await?,
            index: owned(session_index_key(SEEDED_SESSION, SEEDED_INVOCATION)).await?,
            options,
            _root: root,
        };
        drop(pool);
        drop(ledger);
        store.close().await?;
        Ok(seeded)
    }

    /// Reset the closed store's usage branch to the seeded head, then commit
    /// `rows` there as a foreign writer would: through a released server and
    /// raw SQL, never through `UsageLedger`.
    async fn plant(seeded: &Seeded, rows: &[(Vec<u8>, String)]) -> Result<()> {
        plant_with_message(seeded, rows, "foreign usage write").await
    }

    /// [`plant`] with the foreign commit's message chosen by the caller.
    async fn plant_with_message(
        seeded: &Seeded,
        rows: &[(Vec<u8>, String)],
        message: &str,
    ) -> Result<()> {
        let server = super::super::tests::released_server(&seeded.options).await?;
        let pool = server.pool(BRANCH).await?;
        let planted = async {
            sqlx::query("CALL DOLT_RESET('--hard', ?)")
                .bind(&seeded.usage_head)
                .fetch_all(pool.as_ref())
                .await?;
            ensure!(
                revision(pool.as_ref()).await? == seeded.usage_head,
                "the usage branch did not return to its seeded head"
            );
            if rows.is_empty() {
                return Ok(());
            }
            for (key, value) in rows {
                sqlx::query(
                    "INSERT INTO state (`key`, value) VALUES (?, ?) \
                     ON DUPLICATE KEY UPDATE value = VALUES(value)",
                )
                .bind(key.as_slice())
                .bind(value)
                .execute(pool.as_ref())
                .await?;
            }
            sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                .bind(message)
                .bind(AUTHOR)
                .fetch_all(pool.as_ref())
                .await?;
            // A dirty working set would refuse the reopen before the owned
            // scan, so every refusal below must come from the scan itself.
            let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
                .fetch_one(pool.as_ref())
                .await?;
            ensure!(
                dirty == 0,
                "the foreign usage write left a dirty working set"
            );
            Ok(())
        }
        .await;
        pool.close().await;
        let closed = server.close().await;
        planted.and(closed)
    }

    /// Plant `rows`, then require the writable reopen to refuse with
    /// `expected` while a read-only open of the same store still succeeds.
    async fn assert_reopen_refuses(
        seeded: &Seeded,
        case: &str,
        rows: &[(Vec<u8>, String)],
        expected: &str,
    ) -> Result<()> {
        assert_reopen_refuses_with(seeded, case, rows, expected, "foreign usage write").await
    }

    /// [`assert_reopen_refuses`] with the foreign commit's message chosen by
    /// the caller, for instance a copy of a valid validation record.
    async fn assert_reopen_refuses_with(
        seeded: &Seeded,
        case: &str,
        rows: &[(Vec<u8>, String)],
        expected: &str,
        message: &str,
    ) -> Result<()> {
        plant_with_message(seeded, rows, message).await?;
        match MemoryStore::open(seeded.options.clone()).await {
            Ok(store) => {
                store.close().await?;
                bail!("{case}: the writable reopen activated an invalid usage ledger");
            }
            Err(error) => {
                let error = format!("{error:#}");
                ensure!(
                    error.contains(expected),
                    "{case}: expected a refusal containing {expected:?}, got: {error}"
                );
            }
        }
        let mut read_only = seeded.options.clone();
        read_only.read_only = true;
        let reader = MemoryStore::open(read_only)
            .await
            .with_context(|| format!("{case}: the read-only open was refused"))?;
        reader.revision().await?;
        ensure!(
            reader.usage_ledger().is_err(),
            "{case}: a read-only open exposed the usage ledger"
        );
        reader.close().await
    }

    /// The seeded rows are intact: a writable reopen activates the ledger and
    /// folds the seeded usage back.
    async fn assert_seeded_usage_reads_back(seeded: &Seeded) -> Result<()> {
        let store = MemoryStore::open(seeded.options.clone()).await?;
        let ledger = store.usage_ledger()?;
        let usage = ledger.session(SEEDED_SESSION).await?;
        ensure!(usage.historical_complete && usage.invocation_count == 1);
        ensure!(usage.known_usage.input_tokens == Some(5));
        drop(ledger);
        store.close().await
    }

    fn row(key: impl Into<Vec<u8>>, value: impl Into<String>) -> Vec<(Vec<u8>, String)> {
        vec![(key.into(), value.into())]
    }

    fn with_field(value: &str, field: &str, replacement: Value) -> Result<String> {
        let mut value: Value = serde_json::from_str(value)?;
        value
            .as_object_mut()
            .context("an owned usage value is not an object")?
            .insert(field.into(), replacement);
        Ok(serde_json::to_string(&value)?)
    }

    #[tokio::test]
    async fn malformed_owned_values_refuse_writable_reopen() -> Result<()> {
        let seeded = seeded().await?;
        let mut observation_zero: Value = serde_json::from_str(&seeded.observation)?;
        observation_zero["observation"]["sequence"] = Value::from(0);
        let classes = [
            (
                "session marker",
                session_key(SEEDED_SESSION),
                &seeded.marker,
                "usage session marker is malformed",
                with_field(&seeded.marker, "format", Value::from(2))?,
                "usage session marker format is unsupported",
            ),
            (
                "invocation record",
                record_key(SEEDED_INVOCATION),
                &seeded.record,
                "usage ledger record is malformed",
                with_field(&seeded.record, "last_usage_sequence", Value::from(0))?,
                "usage ledger sequence is invalid",
            ),
            (
                "observation",
                observation_key(SEEDED_INVOCATION, 1),
                &seeded.observation,
                "usage observation is malformed",
                serde_json::to_string(&observation_zero)?,
                "usage sequence must start at one",
            ),
            (
                "session index",
                session_index_key(SEEDED_SESSION, SEEDED_INVOCATION),
                &seeded.index,
                "usage session index is malformed",
                with_field(&seeded.index, "format", Value::from(2))?,
                "usage session index format is unsupported",
            ),
        ];
        for (class, key, value, malformed, invalid, invalid_message) in classes {
            assert_reopen_refuses(
                &seeded,
                &format!("{class}: not JSON"),
                &row(key.clone(), "{\"format\":"),
                malformed,
            )
            .await?;
            assert_reopen_refuses(
                &seeded,
                &format!("{class}: unknown field"),
                &row(
                    key.clone(),
                    with_field(value, "unrecognized", Value::Bool(true))?,
                ),
                malformed,
            )
            .await?;
            assert_reopen_refuses(
                &seeded,
                &format!("{class}: failed field validation"),
                &row(key, invalid),
                invalid_message,
            )
            .await?;
        }
        plant(&seeded, &[]).await?;
        assert_seeded_usage_reads_back(&seeded).await
    }

    #[tokio::test]
    async fn owned_values_under_noncanonical_keys_refuse_writable_reopen() -> Result<()> {
        let seeded = seeded().await?;
        let uppercase_record = format!(
            "{RECORD_PREFIX}{}",
            key_digest(SEEDED_INVOCATION).to_uppercase()
        );
        // Each value is the write path's own bytes; only its key is foreign.
        // The canonical row stays in place beside the copy.
        let cases = [
            (
                "session marker under another session's key",
                session_key("other-session"),
                &seeded.marker,
                "usage marker key is not canonical",
            ),
            (
                "invocation record under another invocation's key",
                record_key("other-invocation"),
                &seeded.record,
                "usage ledger record key does not match its invocation",
            ),
            (
                "invocation record under an uppercase digest",
                uppercase_record,
                &seeded.record,
                "usage ledger record key does not match its invocation",
            ),
            (
                "observation under another sequence",
                observation_key(SEEDED_INVOCATION, 2),
                &seeded.observation,
                "usage observation key does not match its value",
            ),
            (
                "session index under another session",
                session_index_key("other-session", SEEDED_INVOCATION),
                &seeded.index,
                "usage session index is not canonical",
            ),
        ];
        for (case, key, value, expected) in cases {
            assert_reopen_refuses(&seeded, case, &row(key, value.clone()), expected).await?;
        }
        plant(&seeded, &[]).await?;
        assert_seeded_usage_reads_back(&seeded).await
    }

    #[tokio::test]
    async fn foreign_keys_under_the_owned_prefix_refuse_writable_reopen() -> Result<()> {
        let seeded = seeded().await?;
        let foreign = "{\"foreign\":true}";
        for key in [
            "kuru.usage.v1/future/x",
            "kuru.usage.v1/",
            "kuru.usage.v1/record",
        ] {
            assert_reopen_refuses(
                &seeded,
                &format!("unknown class {key:?}"),
                &row(key, foreign),
                "usage ledger owns an unrecognized state key",
            )
            .await?;
        }
        assert_reopen_refuses(
            &seeded,
            "non-UTF-8 key under the prefix",
            &row(b"kuru.usage.v1/\xff".to_vec(), foreign),
            "usage ledger key is not UTF-8",
        )
        .await?;
        plant(&seeded, &[]).await?;
        assert_seeded_usage_reads_back(&seeded).await
    }

    #[tokio::test]
    async fn keys_beside_the_owned_prefix_are_not_refused() -> Result<()> {
        let seeded = seeded().await?;
        // Not even JSON: rows outside the prefix are never decoded. They sort
        // before (`.` is 0x2e), at (a proper prefix) and after (`0` is 0x30)
        // the owned range `kuru.usage.v1/`.
        let beside: Vec<(Vec<u8>, String)> =
            ["kuru.usage.v1", "kuru.usage.v1.x", "kuru.usage.v10x"]
                .into_iter()
                .map(|key| (key.as_bytes().to_vec(), "not JSON".to_owned()))
                .collect();
        plant(&seeded, &beside).await?;
        assert_seeded_usage_reads_back(&seeded).await?;
        let mut read_only = seeded.options.clone();
        read_only.read_only = true;
        MemoryStore::open(read_only).await?.close().await?;
        // The same bytes one key inside the range are refused, so the edge
        // sits exactly at the prefix.
        let mut inside = beside;
        inside.push((b"kuru.usage.v1/".to_vec(), "not JSON".to_owned()));
        assert_reopen_refuses(
            &seeded,
            "the bare owned prefix beside the boundary keys",
            &inside,
            "usage ledger owns an unrecognized state key",
        )
        .await
    }

    #[tokio::test]
    async fn establish_publishes_no_usage_pool_over_an_invalid_owned_row() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let usage_pool = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .take()
            .context("usage ledger pool is unavailable")?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(b"kuru.usage.v1/future/x".as_slice())
            .bind("{\"foreign\":true}")
            .execute(usage_pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'foreign usage write', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(usage_pool.as_ref())
            .await?;
        drop(usage_pool);
        let error = match establish(&store).await {
            Ok(()) => {
                store.close().await?;
                bail!("establish activated an invalid usage ledger");
            }
            Err(error) => format!("{error:#}"),
        };
        ensure!(
            error.contains("usage ledger owns an unrecognized state key"),
            "establish refused for another reason: {error}"
        );
        ensure!(
            store
                .shared
                .usage_pool
                .lock()
                .expect("usage pool lock")
                .is_none(),
            "a refused establish published its usage pool"
        );
        let unavailable = format!("{:#}", store.usage_ledger().unwrap_err());
        ensure!(
            unavailable.contains("usage ledger is unavailable"),
            "unexpected ledger refusal: {unavailable}"
        );
        store.close().await
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

    // The paging queries before range paging, verbatim. Kept as the baseline
    // the range queries must match key for key, and as the plan test's
    // negative control.
    const OLD_PAGE: &str = "SELECT `key`, value FROM state WHERE LEFT(BINARY `key`, ?) = BINARY ? AND (? IS NULL OR BINARY `key` > BINARY ?) ORDER BY BINARY `key` LIMIT ?";
    const OLD_ANY_FOR_UPDATE: &str =
        "SELECT COUNT(*) FROM state WHERE LEFT(BINARY `key`, ?) = BINARY ? LIMIT 1 FOR UPDATE";

    #[test]
    fn prefix_upper_bound_increments_the_last_byte_below_ff() -> Result<()> {
        assert_eq!(
            prefix_upper_bound(b"kuru.usage.v1/"),
            Some(b"kuru.usage.v10".to_vec())
        );
        assert_eq!(prefix_upper_bound(b"a\xff"), Some(b"b".to_vec()));
        assert_eq!(
            prefix_upper_bound(b"a\xfe\xff\xff"),
            Some(b"a\xff".to_vec())
        );
        assert_eq!(prefix_upper_bound(b"\x00"), Some(b"\x01".to_vec()));
        assert_eq!(prefix_upper_bound(b"\xff\xff"), None);
        assert_eq!(prefix_upper_bound(b""), None);
        // Every prefix this module pages ends in `/`, whose bound is `0`.
        for prefix in [
            OWNED_PREFIX.to_owned(),
            RECORD_PREFIX.to_owned(),
            OBSERVATION_PREFIX.to_owned(),
            SESSION_PREFIX.to_owned(),
            SESSION_INDEX_PREFIX.to_owned(),
            session_index_prefix("session"),
        ] {
            let range = KeyRange::prefix(&prefix)?;
            let stem = prefix.strip_suffix('/').context("prefix without `/`")?;
            ensure!(range.start == prefix.as_bytes() && range.end == format!("{stem}0").as_bytes());
        }
        let error = format!("{:#}", KeyRange::prefix("").err().context("empty prefix")?);
        ensure!(error.contains("usage ledger key prefix has no upper bound"));
        Ok(())
    }

    /// Walk `prefix` to exhaustion with the old query. Production bound the
    /// cursor as UTF-8 text; `BINARY ?` casts the argument to the same bytes
    /// either way, and binding bytes lets this walk pass non-UTF-8 keys.
    async fn old_walk(pool: &MemoryPool, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        let mut keys = Vec::new();
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page: Vec<(Vec<u8>, String)> = sqlx::query_as(OLD_PAGE)
                .bind(prefix.len() as i64)
                .bind(prefix)
                .bind(after.as_deref())
                .bind(after.as_deref())
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await?;
            let Some((last, _)) = page.last() else {
                return Ok(keys);
            };
            after = Some(last.clone());
            keys.extend(page.into_iter().map(|(key, _)| key));
        }
    }

    /// Walk `range` to exhaustion through the production page function.
    async fn range_walk(
        pool: &MemoryPool,
        range: &KeyRange,
        session_index: bool,
    ) -> Result<Vec<Vec<u8>>> {
        let mut keys = Vec::new();
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = if session_index {
                session_index_page(pool, range, after.as_deref()).await?
            } else {
                owned_state_page(pool, range, after.as_deref()).await?
            };
            let Some((last, _)) = page.last() else {
                return Ok(keys);
            };
            ensure!(page.len() as i64 <= PAGE_SIZE, "a page exceeded PAGE_SIZE");
            after = Some(last.clone());
            keys.extend(page.into_iter().map(|(key, _)| key));
        }
    }

    async fn put_raw_keys(pool: &MemoryPool, keys: &[Vec<u8>]) -> Result<()> {
        let mut transaction = pool.begin().await?;
        for key in keys {
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(key.as_slice())
                .bind("{}")
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// The old and the range walk of `prefix` both equal the stored keys that
    /// begin with it, in byte order.
    async fn assert_same_walk(
        pool: &MemoryPool,
        stored: &std::collections::BTreeSet<Vec<u8>>,
        prefix: &str,
        session_index: bool,
        expected_len: usize,
    ) -> Result<()> {
        let expected: Vec<Vec<u8>> = stored
            .iter()
            .filter(|key| key.starts_with(prefix.as_bytes()))
            .cloned()
            .collect();
        ensure!(
            expected.len() == expected_len,
            "{prefix}: fixture holds {} keys, expected {expected_len}",
            expected.len()
        );
        let old = old_walk(pool, prefix.as_bytes()).await?;
        let new = range_walk(pool, &KeyRange::prefix(prefix)?, session_index).await?;
        ensure!(
            old == expected,
            "{prefix}: the old walk differs from the stored keys"
        );
        ensure!(
            new == old,
            "{prefix}: the range walk differs from the old walk"
        );
        Ok(())
    }

    #[tokio::test]
    async fn range_paging_visits_exactly_the_old_key_sequence() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let pool = ledger.store.pool.clone();
        let mut stored: std::collections::BTreeSet<Vec<u8>> =
            sqlx::query_scalar::<_, Vec<u8>>("SELECT `key` FROM state")
                .fetch_all(pool.as_ref())
                .await?
                .into_iter()
                .collect();
        let owned_before = stored
            .iter()
            .filter(|key| key.starts_with(OWNED_PREFIX.as_bytes()))
            .count();
        let mut long = OWNED_PREFIX.as_bytes().to_vec();
        long.resize(1024, b'z');
        let inside: Vec<Vec<u8>> = vec![
            b"kuru.usage.v1/".to_vec(),
            b"kuru.usage.v1/\x00".to_vec(),
            b"kuru.usage.v1/a\x00b".to_vec(),
            b"kuru.usage.v1/\x7f".to_vec(),
            b"kuru.usage.v1/\x80".to_vec(),
            b"kuru.usage.v1/\xff".to_vec(),
            b"kuru.usage.v1/\xff\xff\x00".to_vec(),
            b"kuru.usage.v1/ ".to_vec(),
            b"kuru.usage.v1/record/x".to_vec(),
            b"kuru.usage.v1/record/x ".to_vec(),
            b"kuru.usage.v1/RECORD/x".to_vec(),
            long,
        ];
        // Keys that only resemble the prefix: proper prefixes of it, and keys
        // that sort immediately before, at and after the range's edges.
        let beside: Vec<Vec<u8>> = vec![
            b"\x00".to_vec(),
            b"kuru.usage.v1".to_vec(),
            b"kuru.usage.v1\x00".to_vec(),
            b"kuru.usage.v1.x".to_vec(),
            b"kuru.usage.v1.\xff".to_vec(),
            b"kuru.usage.v10".to_vec(),
            b"kuru.usage.v10x".to_vec(),
            b"kuru.usage.v0\xff\xff".to_vec(),
            b"kuru.usage.v1\\x".to_vec(),
            b"KURU.USAGE.V1/x".to_vec(),
            b"\xff\xff".to_vec(),
        ];
        let mut fixture: Vec<Vec<u8>> = inside.into_iter().chain(beside).collect();
        // Exactly one full page of owned keys, so the first page ends on the
        // last key in range and the next page must come back empty.
        let owned_now = |fixture: &[Vec<u8>]| {
            owned_before
                + fixture
                    .iter()
                    .filter(|key| key.starts_with(OWNED_PREFIX.as_bytes()))
                    .count()
        };
        let mut filler = 0;
        while owned_now(&fixture) < PAGE_SIZE as usize {
            fixture.push(format!("{OWNED_PREFIX}filler/{filler:05}").into_bytes());
            filler += 1;
        }
        put_raw_keys(pool.as_ref(), &fixture).await?;
        stored.extend(fixture);
        assert_same_walk(pool.as_ref(), &stored, OWNED_PREFIX, false, 128).await?;

        // 129 owned keys: one past the first page.
        let mut add_filler = |count: usize| -> Vec<Vec<u8>> {
            let keys = (filler..filler + count)
                .map(|filler| format!("{OWNED_PREFIX}filler/{filler:05}").into_bytes())
                .collect();
            filler += count;
            keys
        };
        let one = add_filler(1);
        put_raw_keys(pool.as_ref(), &one).await?;
        stored.extend(one);
        assert_same_walk(pool.as_ref(), &stored, OWNED_PREFIX, false, 129).await?;

        // Exactly two full pages: the second page ends on the last key in
        // range, and the third must come back empty.
        let to_two_pages = add_filler(127);
        put_raw_keys(pool.as_ref(), &to_two_pages).await?;
        stored.extend(to_two_pages);
        assert_same_walk(pool.as_ref(), &stored, OWNED_PREFIX, false, 256).await?;

        // 257 owned keys: one past the second page.
        let one_more = add_filler(1);
        put_raw_keys(pool.as_ref(), &one_more).await?;
        stored.extend(one_more);
        assert_same_walk(pool.as_ref(), &stored, OWNED_PREFIX, false, 257).await?;

        // A session index spanning three pages, with its own edge keys, and
        // sessions whose only neighbours sit just outside their range.
        let indexed = session_index_prefix("ordering-session");
        let stem = indexed
            .strip_suffix('/')
            .context("index prefix without `/`")?;
        let edge = session_index_prefix("edge-session");
        let edge_stem = edge.strip_suffix('/').context("index prefix without `/`")?;
        let bare = session_index_prefix("bare-session");
        let mut index_keys: Vec<Vec<u8>> = (0..257)
            .map(|n| format!("{indexed}{}", key_digest(&n.to_string())).into_bytes())
            .collect();
        index_keys.extend([
            indexed.as_bytes().to_vec(),
            [indexed.as_bytes(), b"\x00"].concat(),
            [indexed.as_bytes(), b"\xff"].concat(),
            stem.as_bytes().to_vec(),
            format!("{stem}0").into_bytes(),
            format!("{stem}0x").into_bytes(),
            format!("{stem}.x").into_bytes(),
            edge_stem.as_bytes().to_vec(),
            format!("{edge_stem}0").into_bytes(),
            [edge_stem.as_bytes(), b".\xff"].concat(),
            bare.as_bytes().to_vec(),
        ]);
        put_raw_keys(pool.as_ref(), &index_keys).await?;
        stored.extend(index_keys);
        assert_same_walk(pool.as_ref(), &stored, &indexed, true, 260).await?;
        let owned_total = stored
            .iter()
            .filter(|key| key.starts_with(OWNED_PREFIX.as_bytes()))
            .count();
        assert_same_walk(pool.as_ref(), &stored, OWNED_PREFIX, false, owned_total).await?;

        // The existence probe agrees with the old COUNT(*) on each session.
        let mut transaction = pool.begin().await?;
        for (session, expected) in [
            ("ordering-session", true),
            ("bare-session", true),
            ("edge-session", false),
            ("empty-session", false),
        ] {
            let prefix = session_index_prefix(session);
            let old: i64 = sqlx::query_scalar(OLD_ANY_FOR_UPDATE)
                .bind(prefix.len() as i64)
                .bind(prefix.as_bytes())
                .fetch_one(&mut *transaction)
                .await?;
            let new = session_has_records_tx(&mut transaction, session).await?;
            ensure!(
                (old > 0) == expected && new == expected,
                "{session}: old count {old}, new probe {new}, expected {expected}"
            );
        }
        transaction.rollback().await?;

        sqlx::query("CALL DOLT_RESET('--hard')")
            .fetch_all(pool.as_ref())
            .await?;
        drop(pool);
        drop(ledger);
        store.close().await
    }

    #[tokio::test]
    async fn session_pages_past_one_index_page_and_marks_refuse_recorded_sessions() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let invocations = PAGE_SIZE as usize + 2;
        for n in 0..invocations {
            ledger
                .admit(start("paged-session", &format!("invocation-{n}")))
                .await?;
        }
        let usage = ledger.session("paged-session").await?;
        ensure!(
            usage.invocation_count == invocations as u64,
            "session() folded {} of {invocations} invocations",
            usage.invocation_count
        );
        // The only arm that reads the probe: no marker, yet index rows. A
        // foreign commit plants an orphan index row for one session and,
        // for another, keys just outside its index range on either side.
        let orphan = session_index_key("orphan-session", "orphan-invocation");
        let edge = session_index_prefix("edge-session");
        let edge_stem = edge.strip_suffix('/').context("index prefix without `/`")?;
        let pool = ledger.store.pool.clone();
        for key in [
            orphan.into_bytes(),
            edge_stem.as_bytes().to_vec(),
            format!("{edge_stem}0").into_bytes(),
        ] {
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(key)
                .bind("{}")
                .execute(pool.as_ref())
                .await?;
        }
        sqlx::query("CALL DOLT_COMMIT('-Am', 'foreign usage write', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        // These rows exist only to put the probe's range edges next to real
        // keys; a reopen's scan would refuse them, and the write precondition
        // refuses any foreign change. Adopt the planted content as validated
        // so the probe itself is what this test exercises.
        *store
            .shared
            .usage_validated
            .lock()
            .expect("usage validated lock") = Some(state_hash(pool.as_ref()).await?);
        drop(pool);
        let refused = format!(
            "{:#}",
            ledger
                .mark_new_session("orphan-session")
                .await
                .err()
                .context("a session with index rows was marked new")?
        );
        ensure!(
            refused.contains("usage session already has invocation records"),
            "unexpected refusal: {refused}"
        );
        ledger.mark_new_session("edge-session").await?;
        ensure!(ledger.session("edge-session").await?.historical_complete);
        // Another session's records do not refuse a fresh mark.
        ledger.mark_new_session("fresh-session").await?;
        ensure!(ledger.session("fresh-session").await?.historical_complete);
        drop(ledger);
        store.close().await
    }

    enum PlanArg<'a> {
        Bytes(&'a [u8]),
        Text(Option<&'a str>),
        Int(i64),
    }

    /// `EXPLAIN PLAN` for `sql` with `args` bound through sqlx as production
    /// binds them. Dolt 2.3.5 prepares `EXPLAIN` with zero parameters and has
    /// no `EXPLAIN` for an executed statement, so the bound values reach the
    /// planner as session variables set by one bound `SET`, in place of each
    /// `?`. A `LIMIT ?` takes its integer literally: Dolt refuses a variable
    /// there.
    async fn explain_bound(pool: &MemoryPool, sql: &str, args: &[PlanArg<'_>]) -> Result<String> {
        ensure!(
            sql.matches('?').count() == args.len(),
            "argument count does not match {sql}"
        );
        let mut pieces = sql.split('?');
        let mut explained = format!("EXPLAIN PLAN {}", pieces.next().unwrap_or_default());
        let mut variables = Vec::new();
        let mut bound = Vec::new();
        for (n, (arg, piece)) in args.iter().zip(pieces).enumerate() {
            match arg {
                PlanArg::Int(value) if explained.ends_with("LIMIT ") => {
                    explained.push_str(&value.to_string());
                }
                arg => {
                    let variable = format!("@kuru_plan_{n}");
                    explained.push_str(&variable);
                    variables.push(format!("{variable} = ?"));
                    bound.push(arg);
                }
            }
            explained.push_str(piece);
        }
        let mut connection = pool.acquire().await?;
        let set = format!("SET {}", variables.join(", "));
        let mut query = sqlx::query(sqlx::AssertSqlSafe(set));
        for arg in bound {
            query = match arg {
                PlanArg::Bytes(value) => query.bind(*value),
                PlanArg::Text(value) => query.bind(*value),
                PlanArg::Int(value) => query.bind(*value),
            };
        }
        query.execute(&mut *connection).await?;
        let rows = sqlx::raw_sql(sqlx::AssertSqlSafe(explained))
            .fetch_all(&mut *connection)
            .await?;
        Ok(rows
            .iter()
            .map(|row| row.try_get::<String, _>(0))
            .collect::<sqlx::Result<Vec<_>>>()?
            .join("\n"))
    }

    /// The node names of an `EXPLAIN PLAN` tree, without the tree drawing.
    fn plan_nodes(plan: &str) -> impl Iterator<Item = &str> {
        plan.lines()
            .map(|line| line.trim_start_matches(|c: char| c.is_whitespace() || "└├│─".contains(c)))
    }

    fn plan_bytes(bytes: &[u8]) -> String {
        let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
        format!("[{}]", bytes.join(" "))
    }

    fn ensure_range_plan(
        label: &str,
        plan: &str,
        low: &[u8],
        low_inclusive: bool,
        high: &[u8],
    ) -> Result<()> {
        let filter = format!(
            "filters: [{{{}{}, {})}}]",
            if low_inclusive { '[' } else { '(' },
            plan_bytes(low),
            plan_bytes(high)
        );
        ensure!(
            plan_nodes(plan).any(|node| node == "IndexedTableAccess(state)")
                && plan.contains("index: [state.key]")
                && plan.contains(&filter)
                && !plan.contains("TopN")
                && !plan_nodes(plan).any(|node| node == "Table"),
            "{label}: expected a bounded primary-key range read {filter} on the pinned Dolt; \
             a Dolt upgrade that changes plan text must update this test deliberately. Plan:\n{plan}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn bound_range_queries_plan_as_primary_key_ranges() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let pool = ledger.store.pool.clone();
        let after = session_key("plan-session").into_bytes();
        let owned = KeyRange::prefix(OWNED_PREFIX)?;
        let indexed = KeyRange::prefix(&session_index_prefix("plan-session"))?;
        let index_after = session_index_key("plan-session", "plan-invocation").into_bytes();
        for (label, range, after) in [
            ("owned first page", &owned, None),
            ("owned later page", &owned, Some(after.as_slice())),
            ("session index first page", &indexed, None),
            (
                "session index later page",
                &indexed,
                Some(index_after.as_slice()),
            ),
        ] {
            let (sql, [low, high]) = range.page(after);
            let plan = explain_bound(
                pool.as_ref(),
                sql,
                &[
                    PlanArg::Bytes(low),
                    PlanArg::Bytes(high),
                    PlanArg::Int(PAGE_SIZE),
                ],
            )
            .await?;
            ensure_range_plan(label, &plan, low, after.is_none(), high)?;
        }
        let (sql, [low, high]) = indexed.any_for_update();
        let plan = explain_bound(
            pool.as_ref(),
            sql,
            &[PlanArg::Bytes(low), PlanArg::Bytes(high)],
        )
        .await?;
        ensure_range_plan("session record probe", &plan, low, true, high)?;

        // Negative control: the old queries, bound as production bound them,
        // sort or read the whole table.
        let after = std::str::from_utf8(&after)?;
        for (label, after) in [("old first page", None), ("old later page", Some(after))] {
            let plan = explain_bound(
                pool.as_ref(),
                OLD_PAGE,
                &[
                    PlanArg::Int(OWNED_PREFIX.len() as i64),
                    PlanArg::Bytes(OWNED_PREFIX.as_bytes()),
                    PlanArg::Text(after),
                    PlanArg::Text(after),
                    PlanArg::Int(PAGE_SIZE),
                ],
            )
            .await?;
            ensure!(
                plan.contains("TopN") && plan_nodes(&plan).any(|node| node == "Table"),
                "{label}: the old query no longer sorts a full table scan:\n{plan}"
            );
        }
        let prefix = session_index_prefix("plan-session");
        let plan = explain_bound(
            pool.as_ref(),
            OLD_ANY_FOR_UPDATE,
            &[
                PlanArg::Int(prefix.len() as i64),
                PlanArg::Bytes(prefix.as_bytes()),
            ],
        )
        .await?;
        ensure!(
            plan_nodes(&plan).any(|node| node == "Table"),
            "old record probe no longer reads the full table:\n{plan}"
        );
        drop(pool);
        drop(ledger);
        store.close().await
    }

    /// Every owned row of `pool`'s state, in key order.
    async fn owned_rows(pool: &MemoryPool) -> Result<Vec<(Vec<u8>, String)>> {
        let owned = KeyRange::prefix(OWNED_PREFIX)?;
        let mut rows = Vec::new();
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = owned_state_page(pool, &owned, after.as_deref()).await?;
            let Some((last, _)) = page.last() else {
                return Ok(rows);
            };
            after = Some(last.clone());
            rows.extend(page);
        }
    }

    // T14 (same function): every row a scripted session writes passes the
    // scan's validator, and a row it would refuse never reaches a commit.
    #[tokio::test]
    async fn writes_validate_each_row_with_the_scan_validator() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session("scripted").await?;
        ledger.admit(start("scripted", "scripted-1")).await?;
        for (sequence, terminal) in [(1, false), (2, false), (3, true)] {
            ledger
                .observe(
                    "scripted-1",
                    UsageObservation {
                        sequence,
                        terminal,
                        usage: Usage {
                            input_tokens: Some(sequence),
                            ..Usage::default()
                        },
                    },
                )
                .await?;
        }
        ledger
            .settle("scripted-1", InvocationOutcome::Succeeded)
            .await?;
        let pool = ledger.store.pool.clone();
        let rows = owned_rows(pool.as_ref()).await?;
        // One marker, one record, one index and three observations.
        ensure!(rows.len() == 6, "{} owned rows", rows.len());
        for (key, value) in &rows {
            validate_owned_row(key, value)?;
        }

        // A value the scan would refuse (a marker under another session's
        // key) is refused inside the write transaction, before its INSERT.
        let head = revision(pool.as_ref()).await?;
        let (mut connection, _) = owned_connection(&pool, write_deadline()).await?;
        let mut transaction = connection.begin().await?;
        let refused = put_state_tx(
            &mut transaction,
            &session_key("another-session"),
            &SessionMarker {
                format: FORMAT,
                session_id: "scripted".into(),
                historical_complete: true,
            },
        )
        .await
        .unwrap_err();
        ensure!(
            format!("{refused:#}").contains("usage marker key is not canonical"),
            "{refused:#}"
        );
        transaction.rollback().await?;
        drop(connection);
        ensure!(
            read_state(pool.as_ref(), &session_key("another-session"))
                .await?
                .is_none()
        );
        ensure!(revision(pool.as_ref()).await? == head);
        drop(pool);
        drop(ledger);
        store.close().await
    }

    const GOLDEN_SESSION_KEY: &str =
        "kuru.usage.v1/session/1e591f4cbe50d9d30081aea051bd034c41979b37bf2ac3f58a2dc611807ab76a";
    const GOLDEN_MARKER: &str =
        r#"{"format":1,"session_id":"golden-session","historical_complete":true}"#;
    const GOLDEN_RECORD_KEY: &str =
        "kuru.usage.v1/record/279d6880e15f0baec51570ca907d2b220e0fae60d18eab62a9d8f2c402309c6b";
    const GOLDEN_RECORD: &str = r#"{"start":{"session_id":"golden-session","invocation_id":"golden-invocation","operation_id":"turn-1","phase":"speak","actor_id":"speaker","route":"responses","model":"model","price_at_invocation":null},"usage":{"input_tokens":null,"output_tokens":null,"cached_input_tokens":null,"reasoning_output_tokens":null},"last_usage_sequence":1,"terminal_usage":null,"outcome":null,"incomplete":true}"#;
    const GOLDEN_OBSERVATION_KEY: &str = "kuru.usage.v1/observation/279d6880e15f0baec51570ca907d2b220e0fae60d18eab62a9d8f2c402309c6b/00000000000000000001";
    const GOLDEN_OBSERVATION: &str = r#"{"invocation_id":"golden-invocation","observation":{"sequence":1,"terminal":true,"usage":{"input_tokens":3,"output_tokens":null,"cached_input_tokens":null,"reasoning_output_tokens":null}}}"#;
    const GOLDEN_INDEX_KEY: &str = "kuru.usage.v1/session-index/1e591f4cbe50d9d30081aea051bd034c41979b37bf2ac3f58a2dc611807ab76a/279d6880e15f0baec51570ca907d2b220e0fae60d18eab62a9d8f2c402309c6b";
    const GOLDEN_INDEX: &str = r#"{"format":1,"session_id":"golden-session","invocation_id":"golden-invocation","record_key":"kuru.usage.v1/record/279d6880e15f0baec51570ca907d2b220e0fae60d18eab62a9d8f2c402309c6b"}"#;

    // T18, the tripwire beside `VALIDATOR`: fixed bytes, one accepted row per
    // class and refused near-misses. A decoder or key change that flips any
    // verdict here must update this corpus and bump the `v1` stem together,
    // or a recorded ledger would skip the changed check until the next
    // release.
    #[test]
    fn golden_corpus_pins_the_validator_verdicts() -> Result<()> {
        assert_eq!(
            VALIDATOR,
            format!("kuru.usage.state.v1+{}", env!("CARGO_PKG_VERSION"))
        );
        for (key, value) in [
            (GOLDEN_SESSION_KEY, GOLDEN_MARKER),
            (GOLDEN_RECORD_KEY, GOLDEN_RECORD),
            (GOLDEN_OBSERVATION_KEY, GOLDEN_OBSERVATION),
            (GOLDEN_INDEX_KEY, GOLDEN_INDEX),
        ] {
            validate_owned_row(key.as_bytes(), value)
                .with_context(|| format!("golden row {key} was refused"))?;
        }
        let long_id = "x".repeat(257);
        let refused: [(&str, Vec<u8>, String, &str); 8] = [
            (
                "unknown field",
                GOLDEN_SESSION_KEY.into(),
                with_field(GOLDEN_MARKER, "unrecognized", Value::Bool(true))?,
                "usage session marker is malformed",
            ),
            (
                "wrong key",
                GOLDEN_SESSION_KEY.replace("/1e59", "/1e58").into_bytes(),
                GOLDEN_MARKER.into(),
                "usage marker key is not canonical",
            ),
            (
                "unknown class",
                b"kuru.usage.v1/future/1e591f4cbe50".to_vec(),
                GOLDEN_MARKER.into(),
                "usage ledger owns an unrecognized state key",
            ),
            (
                "non-UTF-8 key",
                [GOLDEN_SESSION_KEY.as_bytes(), b"\xff"].concat(),
                GOLDEN_MARKER.into(),
                "usage ledger key is not UTF-8",
            ),
            (
                "zero sequence",
                GOLDEN_RECORD_KEY.into(),
                with_field(GOLDEN_RECORD, "last_usage_sequence", Value::from(0))?,
                "usage ledger sequence is invalid",
            ),
            (
                "bad format",
                GOLDEN_INDEX_KEY.into(),
                with_field(GOLDEN_INDEX, "format", Value::from(2))?,
                "usage session index format is unsupported",
            ),
            (
                "over-long session id",
                GOLDEN_SESSION_KEY.into(),
                with_field(GOLDEN_MARKER, "session_id", Value::from(long_id.as_str()))?,
                "usage session ID",
            ),
            (
                "over-long invocation id",
                GOLDEN_OBSERVATION_KEY.into(),
                with_field(
                    GOLDEN_OBSERVATION,
                    "invocation_id",
                    Value::from(long_id.as_str()),
                )?,
                "usage invocation ID",
            ),
        ];
        for (case, key, value, expected) in refused {
            let error = match validate_owned_row(&key, &value) {
                Ok(()) => bail!("{case}: the validator accepted a near-miss"),
                Err(error) => format!("{error:#}"),
            };
            ensure!(
                error.contains(expected),
                "{case}: expected {expected:?}, got {error}"
            );
        }
        Ok(())
    }

    // T11, row-locality: the validator's only inputs are one key and its
    // value (pinned by the fn-pointer type), and rows that are individually
    // valid pass whatever else the table holds or lacks: an observation and
    // an index whose record is absent are accepted. A cross-row check would
    // fail this test and requires bumping the `v1` stem of `VALIDATOR`.
    #[test]
    fn the_row_validator_is_row_local() -> Result<()> {
        let validator: fn(&[u8], &str) -> Result<()> = validate_owned_row;
        let orphan_record = record_key("no-such-invocation");
        let orphan_index = SessionIndex {
            format: FORMAT,
            session_id: "no-such-session".into(),
            invocation_id: "no-such-invocation".into(),
            record_key: orphan_record.clone(),
        };
        validator(
            session_index_key("no-such-session", "no-such-invocation").as_bytes(),
            &serde_json::to_string(&orphan_index)?,
        )?;
        let orphan_observation = StoredObservation {
            invocation_id: "no-such-invocation".into(),
            observation: UsageObservation {
                sequence: 9,
                terminal: false,
                usage: Usage::default(),
            },
        };
        validator(
            observation_key("no-such-invocation", 9).as_bytes(),
            &serde_json::to_string(&orphan_observation)?,
        )?;
        // The verdict is a function of the row alone: the same row gives the
        // same verdict before and after unrelated rows are validated.
        let before = validator(GOLDEN_RECORD_KEY.as_bytes(), GOLDEN_RECORD).is_ok();
        validator(GOLDEN_SESSION_KEY.as_bytes(), GOLDEN_MARKER)?;
        ensure!(before && validator(GOLDEN_RECORD_KEY.as_bytes(), GOLDEN_RECORD).is_ok());
        Ok(())
    }

    const HASH: &str = "0123456789abcdefghijklmnopqrstuv";

    // T8 (design T17): the record encoders round-trip, and every malformed
    // form reads as no record.
    #[test]
    fn the_validation_record_parser_is_strict() {
        let expected = Some(UsageRecord {
            validator: VALIDATOR,
            state_hash: HASH,
        });
        let write = write_message("op", HASH);
        assert_eq!(
            write,
            format!("usage ledger v1 [op]\n\nKuru-Usage-State: {VALIDATOR} {HASH}")
        );
        assert_eq!(parse_record(&write), expected);
        assert_eq!(parse_record(&format!("{write}\n")), expected);
        let record = record_message(HASH);
        assert!(record.starts_with("usage ledger validation v1\n\n"));
        assert_eq!(parse_record(&record), expected);
        let foreign = format!("subject\n\nKuru-Usage-State: kuru.usage.state.v0+0.0.0 {HASH}");
        assert_eq!(
            parse_record(&foreign),
            Some(UsageRecord {
                validator: "kuru.usage.state.v0+0.0.0",
                state_hash: HASH,
            })
        );

        let oversize = format!(
            "{}\n\n{}",
            "s".repeat(RECORD_MESSAGE_MAX),
            record_trailer(HASH)
        );
        let at_limit = format!(
            "{}\n\n{}",
            "s".repeat(RECORD_MESSAGE_MAX - 2 - record_trailer(HASH).len()),
            record_trailer(HASH)
        );
        assert_eq!(at_limit.len(), RECORD_MESSAGE_MAX);
        assert_eq!(parse_record(&at_limit), expected);
        let long_validator = format!("subject\n\nKuru-Usage-State: {} {HASH}", "v".repeat(129));
        let max_validator = format!("subject\n\nKuru-Usage-State: {} {HASH}", "v".repeat(128));
        assert!(parse_record(&max_validator).is_some());
        for (case, message) in [
            ("empty", String::new()),
            ("no record", "usage ledger v1 [op]".to_owned()),
            ("oversize", oversize),
            ("two records", format!("{write}\n{}", record_trailer(HASH))),
            (
                "a record line before the subject's record",
                format!("{}\n\n{write}", record_trailer(HASH)),
            ),
            ("trailing line", format!("{write}\nmore")),
            ("trailing garbage", format!("{write} extra")),
            ("indented", format!("subject\n\n {}", record_trailer(HASH))),
            (
                "no space",
                format!("subject\n\nKuru-Usage-State:{VALIDATOR} {HASH}"),
            ),
            (
                "double space",
                format!("subject\n\nKuru-Usage-State: {VALIDATOR}  {HASH}"),
            ),
            (
                "no hash",
                format!("subject\n\nKuru-Usage-State: {VALIDATOR}"),
            ),
            ("short hash", record_message(&HASH[1..])),
            ("long hash", record_message(&format!("{HASH}0"))),
            (
                "hash outside [0-9a-v]",
                record_message(&HASH.replace('v', "w")),
            ),
            ("uppercase hash", record_message(&HASH.to_uppercase())),
            ("long validator", long_validator),
            (
                "non-ASCII validator",
                format!("subject\n\nKuru-Usage-State: kuru.usage.state.v1+0.9.0\u{e9} {HASH}"),
            ),
            (
                "control in validator",
                format!("subject\n\nKuru-Usage-State: kuru\tusage {HASH}"),
            ),
            (
                "lowercase key",
                format!("subject\n\nkuru-usage-state: {VALIDATOR} {HASH}"),
            ),
        ] {
            assert_eq!(parse_record(&message), None, "{case}: {message:?}");
        }
    }

    /// The usage branch's HEAD and HEAD's full message, through a released
    /// server on the closed store.
    async fn closed_usage_head(options: &OpenOptions) -> Result<(String, String, Vec<String>)> {
        let server = super::super::tests::released_server(options).await?;
        let pool = server.pool(BRANCH).await?;
        let read = async {
            let head = revision(pool.as_ref()).await?;
            let (logged, message): (String, String) =
                sqlx::query_as("SELECT commit_hash, message FROM dolt_log LIMIT 1")
                    .fetch_one(pool.as_ref())
                    .await?;
            ensure!(logged == head, "dolt_log's first row is not HEAD");
            let parents: Vec<String> = sqlx::query_scalar(
                "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? ORDER BY parent_index",
            )
            .bind(&head)
            .fetch_all(pool.as_ref())
            .await?;
            Ok((head, message, parents))
        }
        .await;
        pool.close().await;
        let closed = server.close().await;
        read.and_then(|read| closed.map(|()| read))
    }

    /// Commit an empty foreign commit with `message` on the closed store's
    /// usage branch, as an older binary or a manual edit would leave HEAD.
    async fn foreign_empty_commit(options: &OpenOptions, message: &str) -> Result<()> {
        let server = super::super::tests::released_server(options).await?;
        let pool = server.pool(BRANCH).await?;
        let committed =
            sqlx::query("CALL DOLT_COMMIT('--allow-empty', '--message', ?, '--author', ?)")
                .bind(message)
                .bind(AUTHOR)
                .fetch_all(pool.as_ref())
                .await;
        pool.close().await;
        let closed = server.close().await;
        committed?;
        closed
    }

    fn usage_open(store: &MemoryStore) -> Result<UsageOpen> {
        store
            .shared
            .usage_open
            .lock()
            .expect("usage open lock")
            .clone()
            .context("the open recorded no usage establishment")
    }

    async fn receipt_ids(store: &MemoryStore) -> Result<Vec<String>> {
        let pool = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("usage pool missing")?;
        Ok(sqlx::query_scalar("SELECT id FROM operations ORDER BY id")
            .fetch_all(pool.as_ref())
            .await?)
    }

    // T1 and T2 (design T10, T4): a head without a record (an older binary's
    // write) gets one full scan and one record commit; the next reopen is
    // Bound, decodes 0 rows and adds nothing; after one write, a reopen is
    // Bound again with no record commit.
    #[tokio::test]
    async fn an_unrecorded_head_is_scanned_once_then_reopens_bound() -> Result<()> {
        let seeded = seeded().await?;
        let (written, message, _) = closed_usage_head(&seeded.options).await?;
        ensure!(
            parse_record(&message).is_some_and(|record| record.validator == VALIDATOR),
            "a ledger write carried no record: {message:?}"
        );
        foreign_empty_commit(&seeded.options, "usage ledger v1 [older binary]").await?;
        let (foreign, _, _) = closed_usage_head(&seeded.options).await?;
        ensure!(foreign != written);

        let store = MemoryStore::open(seeded.options.clone()).await?;
        ensure!(
            usage_open(&store)?
                == UsageOpen {
                    bound: false,
                    scanned: Some(4),
                    rescanned: None,
                    recorded: true,
                },
            "{:?}",
            usage_open(&store)?
        );
        let receipts = receipt_ids(&store).await?;
        ensure!(receipts.len() == 4, "{receipts:?}");
        let validated_hash = validated(&store).context("no validated content")?;
        store.close().await?;
        let (recorded, message, parents) = closed_usage_head(&seeded.options).await?;
        ensure!(parents == [foreign.clone()], "{parents:?}");
        ensure!(
            message.starts_with("usage ledger validation v1\n\n")
                && parse_record(&message)
                    == Some(UsageRecord {
                        validator: VALIDATOR,
                        state_hash: &validated_hash,
                    }),
            "{message:?}"
        );

        let store = MemoryStore::open(seeded.options.clone()).await?;
        ensure!(
            usage_open(&store)?
                == UsageOpen {
                    bound: true,
                    scanned: None,
                    rescanned: None,
                    recorded: false,
                }
        );
        ensure!(receipt_ids(&store).await? == receipts);
        let ledger = store.usage_ledger()?;
        ledger
            .admit(start(SEEDED_SESSION, "after-the-record"))
            .await?;
        drop(ledger);
        store.close().await?;
        let (written_again, _, parents) = closed_usage_head(&seeded.options).await?;
        ensure!(parents == [recorded]);

        let store = MemoryStore::open(seeded.options.clone()).await?;
        ensure!(
            usage_open(&store)?
                == UsageOpen {
                    bound: true,
                    scanned: None,
                    rescanned: None,
                    recorded: false,
                }
        );
        store.close().await?;
        ensure!(closed_usage_head(&seeded.options).await?.0 == written_again);
        Ok(())
    }

    // T3 (design T9): after a record, a foreign commit corrupts one owned
    // row, (a) without a record, (b) copying HEAD's exact message with its
    // record, (c) with a valid value under a non-canonical key and the copied
    // record. Every reopen refuses: the stale record forces the scan.
    #[tokio::test]
    async fn corruption_after_a_record_refuses_the_reopen() -> Result<()> {
        let seeded = seeded().await?;
        let (_, recorded, _) = closed_usage_head(&seeded.options).await?;
        ensure!(parse_record(&recorded).is_some(), "{recorded:?}");
        let malformed = row(record_key(SEEDED_INVOCATION), "{\"start\":");
        assert_reopen_refuses(
            &seeded,
            "(a) malformed, no record",
            &malformed,
            "usage ledger record is malformed",
        )
        .await?;
        assert_reopen_refuses_with(
            &seeded,
            "(b) malformed, copied record",
            &malformed,
            "usage ledger record is malformed",
            &recorded,
        )
        .await?;
        assert_reopen_refuses_with(
            &seeded,
            "(c) non-canonical key, copied record",
            &row(record_key("other-invocation"), seeded.record.clone()),
            "usage ledger record key does not match its invocation",
            &recorded,
        )
        .await?;
        plant(&seeded, &[]).await?;
        assert_seeded_usage_reads_back(&seeded).await
    }

    // T5 (design T11): another validator's record over the live content is
    // Missing: the open scans and re-records under this validator. The same
    // foreign record over an unknown class (the downgrade case) refuses.
    #[tokio::test]
    async fn another_validators_record_is_scanned_and_rerecorded() -> Result<()> {
        let seeded = seeded().await?;
        let store = MemoryStore::open(seeded.options.clone()).await?;
        let live = validated(&store).context("no validated content")?;
        store.close().await?;
        let older = format!(
            "usage ledger v1 [older]\n\nKuru-Usage-State: kuru.usage.state.v0+0.0.0 {live}"
        );
        foreign_empty_commit(&seeded.options, &older).await?;
        let store = MemoryStore::open(seeded.options.clone()).await?;
        ensure!(
            usage_open(&store)?
                == UsageOpen {
                    bound: false,
                    scanned: Some(4),
                    rescanned: None,
                    recorded: true,
                }
        );
        store.close().await?;
        let (_, message, _) = closed_usage_head(&seeded.options).await?;
        ensure!(
            parse_record(&message)
                == Some(UsageRecord {
                    validator: VALIDATOR,
                    state_hash: &live,
                })
        );

        // The downgrade case: a newer validator's record over content with a
        // class this binary does not know. The record names the planted
        // content's own hash, so only the validator differs.
        let server = super::super::tests::released_server(&seeded.options).await?;
        let pool = server.pool(BRANCH).await?;
        let planted = async {
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(b"kuru.usage.v1/future/x".as_slice())
                .bind("{\"future\":true}")
                .execute(pool.as_ref())
                .await?;
            let hash = state_hash(pool.as_ref()).await?;
            sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
                .bind(format!(
                    "usage ledger v1 [newer]\n\nKuru-Usage-State: kuru.usage.state.v2+9.9.9 {hash}"
                ))
                .bind(AUTHOR)
                .fetch_all(pool.as_ref())
                .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        pool.close().await;
        server.close().await?;
        planted?;
        let error = match MemoryStore::open(seeded.options.clone()).await {
            Ok(store) => {
                store.close().await?;
                bail!("a newer validator's record activated an unknown class");
            }
            Err(error) => format!("{error:#}"),
        };
        ensure!(
            error.contains("usage ledger owns an unrecognized state key"),
            "{error}"
        );
        Ok(())
    }

    // T15: an empty ledger writes no record commit at open, so a fresh
    // store's usage head is its creation head across reopens, and the first
    // ledger write carries the record.
    #[tokio::test]
    async fn an_empty_ledger_writes_no_record() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", "0".repeat(64)),
        )
        .await?;
        let store = MemoryStore::open(options.clone()).await?;
        let first = usage_open(&store)?;
        ensure!(
            first
                == UsageOpen {
                    bound: false,
                    scanned: Some(0),
                    rescanned: None,
                    recorded: false,
                },
            "{first:?}"
        );
        store.close().await?;
        let (head, _, _) = closed_usage_head(&options).await?;
        let store = MemoryStore::open(options.clone()).await?;
        ensure!(usage_open(&store)? == first);
        let ledger = store.usage_ledger()?;
        ledger.mark_new_session("first").await?;
        drop(ledger);
        store.close().await?;
        let (written, message, parents) = closed_usage_head(&options).await?;
        ensure!(
            parents == [head.clone()],
            "the empty ledger gained a commit at open"
        );
        ensure!(written != head);
        ensure!(
            parse_record(&message).is_some_and(|record| record.validator == VALIDATOR),
            "{message:?}"
        );
        let store = MemoryStore::open(options).await?;
        ensure!(usage_open(&store)?.bound);
        store.close().await
    }

    // T13 for the record commit: its receipt reconciles as not committed
    // while HEAD is its base, as committed when HEAD records exactly its
    // content on top of the base, and as not committed (a missing record,
    // never an error) when HEAD diverged from both.
    #[tokio::test]
    async fn the_validation_record_receipt_reconciles() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let store = reopen(&root).await?;
        let ledger = store.usage_ledger()?;
        let pool = ledger.store.pool.clone();
        let base = revision(pool.as_ref()).await?;
        let live = state_hash(pool.as_ref()).await?;
        let receipt = |base_head: &str| Receipt::UsageValidation {
            base_head: base_head.into(),
            state_hash: live.clone(),
        };
        let finished = || async {
            let (_, id) = owned_connection(&pool, write_deadline()).await?;
            Ok::<_, anyhow::Error>(id)
        };

        set_pending(&ledger, finished().await?, receipt(&base));
        assert_eq!(ledger.store.reconcile().await?, Some(false));
        assert_eq!(validated(&store), Some(live.clone()));

        sqlx::query("CALL DOLT_COMMIT('--allow-empty', '--message', ?, '--author', ?)")
            .bind(record_message(&live))
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        set_pending(&ledger, finished().await?, receipt(&base));
        assert_eq!(ledger.store.reconcile().await?, Some(true));
        assert_eq!(validated(&store), Some(live.clone()));
        ledger.mark_new_session("after-record").await?;
        let written = validated(&store).context("the write published no hash")?;

        // Diverged from both, on a recorded head: settled as not committed,
        // the Pending is cleared and the head's own record keeps the ledger
        // writable.
        set_pending(&ledger, finished().await?, receipt(&base));
        assert_eq!(ledger.store.reconcile().await?, Some(false));
        let pending = || {
            ledger
                .store
                .shared
                .uncertain
                .lock()
                .expect("uncertain lock")
                .is_some()
        };
        ensure!(!pending(), "a diverged record outcome stayed pending");
        assert_eq!(validated(&store), Some(written));
        ledger.mark_new_session("after-divergence").await?;
        let written = validated(&store).context("the write published no hash")?;

        // Diverged from both, on a head without a record (a foreign commit
        // on top): still not committed and never an error. The live content
        // is the validated content, so the ledger stays writable; the record
        // is Missing, so the next open scans and records again.
        sqlx::query("CALL DOLT_COMMIT('--allow-empty', '--message', ?, '--author', ?)")
            .bind("foreign commit without a record")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        ensure!(!bound_check(pool.as_ref()).await?.bound);
        set_pending(&ledger, finished().await?, receipt(&base));
        assert_eq!(ledger.store.reconcile().await?, Some(false));
        ensure!(!pending(), "a diverged record outcome stayed pending");
        assert_eq!(validated(&store), Some(written));
        drop(pool);
        drop(ledger);
        store.close().await?;

        let reopened = reopen(&root).await?;
        // 2 markers: "after-record" and "after-divergence".
        assert_eq!(
            usage_open(&reopened)?,
            UsageOpen {
                bound: false,
                scanned: Some(2),
                rescanned: None,
                recorded: true,
            }
        );
        reopened
            .usage_ledger()?
            .mark_new_session("after-rescan")
            .await?;
        reopened.close().await
    }

    // The Bound-path probe: a single primary-key range read that sees owned
    // rows and nothing beside the prefix.
    #[tokio::test]
    async fn the_owned_row_probe_sees_only_the_owned_range() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let ledger = store.usage_ledger()?;
        let pool = ledger.store.pool.clone();
        ensure!(!any_owned(pool.as_ref()).await?);
        for key in ["kuru.usage.v1", "kuru.usage.v10"] {
            sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                .bind(key.as_bytes())
                .bind("beside")
                .execute(pool.as_ref())
                .await?;
        }
        ensure!(!any_owned(pool.as_ref()).await?);
        sqlx::query("CALL DOLT_COMMIT('-Am', 'beside the owned prefix', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        *store
            .shared
            .usage_validated
            .lock()
            .expect("usage validated lock") = Some(state_hash(pool.as_ref()).await?);
        ledger.mark_new_session("probed").await?;
        ensure!(any_owned(pool.as_ref()).await?);
        drop(pool);
        drop(ledger);
        store.close().await
    }

    /// origin/main `d17dfe40`'s `validate_branch`, verbatim but for its name:
    /// the check a binary that predates the validation record runs on every
    /// writable open (twice, around `upgrade_usage`). It is deliberately not
    /// `validate_owned_row`, so a change to the shared validator cannot hide
    /// a difference here.
    async fn older_validate_branch(pool: &MemoryPool) -> Result<u64> {
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
        let owned = KeyRange::prefix(OWNED_PREFIX)?;
        let mut after: Option<Vec<u8>> = None;
        let mut decoded = 0_u64;
        loop {
            let rows = owned_state_page(pool, &owned, after.as_deref()).await?;
            if rows.is_empty() {
                break;
            }
            decoded = decoded.saturating_add(u64::try_from(rows.len()).unwrap_or(u64::MAX));
            for (key, value) in &rows {
                let key =
                    String::from_utf8(key.clone()).context("usage ledger key is not UTF-8")?;
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
            after = rows.last().map(|(key, _)| key.clone());
        }
        Ok(decoded)
    }

    /// An older binary's `mark_new_session` write, as origin/main `d17dfe40`
    /// commits it: no state precondition, no row validator, and a commit
    /// message without a record.
    async fn older_mark_new_session(pool: &MemoryPool, session_id: &str) -> Result<()> {
        let operation = Uuid::new_v4().to_string();
        let mut transaction = pool.begin().await?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)")
            .bind(session_key(session_id).as_bytes())
            .bind(serde_json::to_string(&SessionMarker {
                format: FORMAT,
                session_id: session_id.into(),
                historical_complete: true,
            })?)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
            .bind(&operation)
            .bind("usage ledger v1")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind(format!("usage ledger v1 [{operation}]"))
            .bind(AUTHOR)
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    // T13 (older binary): a ledger this binary wrote, with trailers on its
    // writes and an extra record commit on HEAD, runs every read path an
    // older binary's writable open runs (version, dolt_status,
    // validate_historical, the scan, upgrade_usage, validate_usage, the scan
    // again) without a difference; the older binary then writes as before,
    // without a record, and this binary's next open scans once and records.
    #[tokio::test]
    async fn an_older_binary_reads_and_writes_a_recorded_ledger() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::warmed_open_options(
            root.path().to_owned(),
            format!("project/{}", "0".repeat(64)),
        )
        .await?;
        let store = reopen(&root).await?;
        let ledger = store.usage_ledger()?;
        ledger.admit(start("older", "invocation-1")).await?;
        ledger
            .observe(
                "invocation-1",
                UsageObservation {
                    sequence: 1,
                    terminal: true,
                    usage: Usage {
                        input_tokens: Some(5),
                        ..Usage::default()
                    },
                },
            )
            .await?;
        ledger
            .settle("invocation-1", InvocationOutcome::Succeeded)
            .await?;
        ledger.mark_new_session("newer").await?;
        drop(ledger);
        store.close().await?;
        // An extra record commit on HEAD: a foreign head, then a reopen that
        // scans and records.
        foreign_empty_commit(&options, "foreign head").await?;
        let store = reopen(&root).await?;
        ensure!(usage_open(&store)?.recorded, "the reopen wrote no record");
        store.close().await?;
        let (head, message, _) = closed_usage_head(&options).await?;
        ensure!(
            parse_record(&message).is_some_and(|record| record.validator == VALIDATOR),
            "HEAD is not a record commit: {message:?}"
        );

        let server = super::super::tests::released_server(&options).await?;
        let pool = server.pool(BRANCH).await?;
        let older = async {
            ensure!(
                migrations::version(pool.as_ref()).await? == migrations::USAGE_CURRENT_VERSION,
                "the usage schema version changed"
            );
            let rows = older_validate_branch(pool.as_ref()).await?;
            // Marker x2 (pre-ledger "older" and "newer"), record, index,
            // observation.
            ensure!(rows == 5, "the older scan decoded {rows} rows");
            migrations::upgrade_usage(&server, pool.as_ref()).await?;
            ensure!(
                revision(pool.as_ref()).await? == head,
                "the older upgrade step moved the usage head"
            );
            migrations::validate_usage(pool.as_ref()).await?;
            ensure!(older_validate_branch(pool.as_ref()).await? == rows);
            older_mark_new_session(pool.as_ref(), "written-by-older").await?;
            ensure!(older_validate_branch(pool.as_ref()).await? == rows + 1);
            ensure!(!bound_check(pool.as_ref()).await?.bound);
            Ok(rows)
        }
        .await;
        pool.close().await;
        let closed = server.close().await;
        let rows = older?;
        closed?;

        let store = reopen(&root).await?;
        assert_eq!(
            usage_open(&store)?,
            UsageOpen {
                bound: false,
                scanned: Some(rows + 1),
                rescanned: None,
                recorded: true,
            }
        );
        let ledger = store.usage_ledger()?;
        ensure!(
            ledger
                .session("written-by-older")
                .await?
                .historical_complete
        );
        assert_eq!(ledger.session("older").await?.invocation_count, 1);
        ledger.mark_new_session("after-older").await?;
        drop(ledger);
        store.close().await
    }

    /// Every SQL text in the workspace's product sources that reads a commit
    /// message: a `dolt_log` or `dolt_commits` read naming `message`. Test
    /// modules (`*tests.rs` files and each file's trailing `mod tests`) are
    /// not product readers.
    fn commit_message_readers() -> Vec<(String, String)> {
        fn sources(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(directory) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    sources(&path, files);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    files.push(path);
                }
            }
        }
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        for group in ["apps", "packages"] {
            for crate_dir in std::fs::read_dir(workspace.join(group))
                .expect("workspace group")
                .flatten()
            {
                sources(&crate_dir.path().join("src"), &mut files);
            }
        }
        files.sort();
        assert!(files.len() > 50, "too few sources scanned: {}", files.len());
        let mut readers = Vec::new();
        for file in files {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name.ends_with("tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).expect("read source file");
            let product = text
                .find("#[cfg(test)]\nmod tests {")
                .map_or(text.as_str(), |end| &text[..end]);
            for line in product.lines() {
                let lower = line.to_ascii_lowercase();
                if (lower.contains("dolt_log") || lower.contains("dolt_commits"))
                    && lower.contains("message")
                {
                    // Joined with `/` on every OS, so the inventory below
                    // compares equal on Windows too.
                    let relative = file
                        .strip_prefix(&workspace)
                        .unwrap_or(&file)
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/");
                    readers.push((relative, line.trim().to_owned()));
                }
            }
        }
        readers
    }

    // T13 (parsers): the only product readers of commit messages are the
    // opaque `revisions()` listing (main or candidate pools; the usage branch
    // is reachable only through `UsageLedger`, which does not expose it), the
    // template shape check (template builds only, before any ledger write)
    // and this module's record reader. Other crates reach commit messages
    // only through `revisions()`. A new reader must be added here deliberately
    // and must ignore or strictly parse the record line.
    #[test]
    fn only_known_readers_parse_commit_messages() {
        let readers = commit_message_readers();
        let mut expected = [
            (
                "packages/kuru-memory/src/store.rs",
                "\"SELECT commit_hash, message FROM dolt_log ORDER BY commit_order DESC, commit_hash ASC LIMIT ?\",",
            ),
            (
                "packages/kuru-memory/src/store/migrations/template_shape.rs",
                "\"SELECT CAST(committer AS CHAR), CAST(email AS CHAR), CAST(author AS CHAR), CAST(author_email AS CHAR), CAST(message AS CHAR) FROM dolt_log ORDER BY commit_order LIMIT ?\",",
            ),
            (
                "packages/kuru-memory/src/store/usage_ledger.rs",
                "sqlx::query_as(\"SELECT commit_hash, LEFT(message, ?) FROM dolt_log LIMIT 1\")",
            ),
        ];
        let mut found: Vec<(&str, &str)> = readers
            .iter()
            .map(|(file, line)| (file.as_str(), line.as_str()))
            .collect();
        found.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            found, expected,
            "commit-message readers changed; review each against the usage validation record"
        );
    }

    // T13 (parsers, behaviour): the one generic reader returns usage commit
    // messages verbatim, record line included, and parses nothing from them.
    #[tokio::test]
    async fn the_revision_listing_returns_record_messages_verbatim() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        store.usage_ledger()?.mark_new_session("listed").await?;
        let pool = store
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("usage pool missing")?;
        let usage = MemoryStore {
            shared: store.shared.clone(),
            pool,
            branch: BRANCH.into(),
            logical_receipt: None,
        };
        let revisions = usage.revisions(1).await?;
        let live = state_hash(usage.pool.as_ref()).await?;
        ensure!(revisions.len() == 1);
        let message = &revisions[0].message;
        ensure!(
            message.starts_with("usage ledger v1 [")
                && message.ends_with(&format!("\n\n{}", record_trailer(&live))),
            "{message:?}"
        );
        drop(usage);
        store.close().await
    }
}
