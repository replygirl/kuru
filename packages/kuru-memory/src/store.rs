use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use futures::TryStreamExt;
use kuru_core::{ContentBlock, MemoryConfig, Message, Mode};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Connection, MySqlConnection, MySqlPool, Row};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit};
use uuid::Uuid;
#[cfg(any(test, feature = "test-support"))]
use {std::sync::OnceLock, tokio::sync::Semaphore};

use crate::{
    MemoryOpenProgress, MemoryOpenStage, files,
    migration::{self, LegacyImport, MigrationReceipt},
    progress::ProgressReporter,
    provision,
    server::{LifecycleLease, Server, ServerOptions},
};

#[cfg(test)]
#[path = "store/recovery_tests.rs"]
mod recovery_tests;

#[cfg(test)]
#[path = "store/migration_lifecycle_tests.rs"]
mod migration_lifecycle_tests;
#[cfg(test)]
#[path = "store/operational_gc_tests.rs"]
mod operational_gc_tests;

pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const AUTHOR: &str = "Kuru <memory@kuru.local>";
const CANDIDATE_PREFIX: &str = "candidate_";
const PROMOTING_PREFIX: &str = "kuru_candidate_promoting_";
const ABANDONED_PREFIX: &str = "kuru_candidate_abandoned_";
const CANDIDATE_RECOVERY_BATCH: i64 = 16;
const TEXT_FORMAT: &str = "text-v1";
const TYPED_FORMAT: &str = "typed-v1";
const MAX_TYPED_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_REASONING_SUMMARY_BATCH_STRING_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CONTEXT_SUMMARY_CHECKPOINT_BYTES: usize = 64 * 1024 * 1024;
const PRIVATE_REASONING_SUMMARY_PREFIX: &str = "kuru/private/reasoning-summary/v1/";
const PRIVATE_REASONING_SUMMARY_OPERATION_PREFIX: &str =
    "kuru/private/reasoning-summary/operation/v2/";
const CONTEXT_SUMMARY_FORMAT: &str = "context_summary.v1";
const CONTEXT_SUMMARY_OPERATION_FORMAT: &str = "context_summary.operation.v2";
pub const MAX_SESSION_SOURCE_ROWS: usize = 1024;
pub const MAX_SESSION_SOURCE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_SESSION_LABEL_BYTES: usize = 1024;
pub const SESSION_CATALOG_RECORD_FORMAT: &str = "session_catalog.v1";
pub const SESSION_LIFECYCLE_OUTCOME_FORMAT: &str = "session_lifecycle_outcome.v1";
pub const PUBLIC_TURN_RECORD_FORMAT: &str = "public_turn.v1";
pub const LEGACY_PREFIX_RECORD_FORMAT: &str = "legacy_prefix.v1";
pub const FORK_PROVENANCE_RECORD_FORMAT: &str = "session_fork.v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReasoningSummaryRecord {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub actor_id: String,
    pub invocation_id: String,
    pub item_id: Option<String>,
    pub output_index: Option<u64>,
    pub summary_index: u64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReasoningSummaryConflict;

impl std::fmt::Display for ReasoningSummaryConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("private reasoning summary conflicts with its settled identity")
    }
}

impl std::error::Error for ReasoningSummaryConflict {}

#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub data_dir: PathBuf,
    pub project_scope: String,
    pub config: MemoryConfig,
    pub read_only: bool,
    pub supervisor: Option<PathBuf>,
    #[cfg(test)]
    migration_hooks: Option<Arc<migrations::MigrationRunnerHooks>>,
    #[cfg(test)]
    candidate_recovery_pause: Option<Arc<CandidateRecoveryPause>>,
    #[cfg(test)]
    candidate_cleanup_failure: Option<Arc<AtomicBool>>,
}
impl OpenOptions {
    pub fn new(data_dir: PathBuf, project_scope: String) -> Self {
        Self {
            data_dir,
            project_scope,
            config: MemoryConfig::default(),
            read_only: false,
            supervisor: None,
            #[cfg(test)]
            migration_hooks: None,
            #[cfg(test)]
            candidate_recovery_pause: None,
            #[cfg(test)]
            candidate_cleanup_failure: None,
        }
    }
}

#[derive(Debug)]
struct Shared {
    server: Server,
    directory: PathBuf,
    project_scope: String,
    fixture_unbound_scope: bool,
    read_only: bool,
    write: Arc<Mutex<()>>,
    dream: Arc<Mutex<()>>,
    uncertain: StdMutex<Option<Pending>>,
    usage_pool: StdMutex<Option<Arc<MySqlPool>>>,
    #[cfg(test)]
    candidate_recovery_pause: Option<Arc<CandidateRecoveryPause>>,
    #[cfg(test)]
    candidate_cleanup_failure: Option<Arc<AtomicBool>>,
    _permit: Option<OwnedSemaphorePermit>,
}

#[cfg(test)]
#[derive(Debug)]
struct CandidateRecoveryPause {
    reached: Arc<Semaphore>,
    resume: Arc<Semaphore>,
}

#[cfg(test)]
fn fail_candidate_cleanup_once(store: &MemoryStore) -> Result<()> {
    if store
        .shared
        .candidate_cleanup_failure
        .as_ref()
        .is_some_and(|failure| failure.swap(false, Ordering::SeqCst))
    {
        bail!("injected candidate cleanup failure");
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Pending {
    pool: Arc<MySqlPool>,
    connection: u64,
    receipt: Receipt,
}

#[derive(Clone, Debug)]
enum Receipt {
    Operation(String),
    CandidateCreation {
        branch: String,
        base: String,
    },
    Promotion {
        base: String,
        target: String,
    },
    CandidateTransition {
        source: String,
        status: String,
        expected: String,
    },
    CandidateDeletion {
        branch: String,
        expected: String,
    },
    CandidateExclusion {
        branch: String,
        expected: String,
    },
}

/// A cloneable view whose SQL connections always select the same Dolt branch.
/// Namespace access policy remains the caller's responsibility.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    shared: Arc<Shared>,
    pool: Arc<MySqlPool>,
    branch: String,
    logical_receipt: Option<LogicalReceipt>,
}

/// Compact request identity for one server-dispatched mutation. The branch is
/// pinned by the store view; this does not grant cross-branch UUID authority.
#[derive(Clone, Debug)]
pub(crate) struct LogicalReceipt {
    physical_id: String,
    method: String,
    digest: String,
}

#[derive(Debug)]
pub(crate) struct LogicalReceiptConflict;

impl std::fmt::Display for LogicalReceiptConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "logical mutation ID conflicts with a different operation on this memory view",
        )
    }
}

impl std::error::Error for LogicalReceiptConflict {}
/// One durable message row with its stable store sequence.
///
/// Callers that present selected active notes use the sequence only with an
/// already-authorized namespace; it is not a global message identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNote {
    pub sequence: i64,
    pub role: String,
    pub content: String,
}

/// A bounded suffix of one retained namespace together with its exact durable
/// row count. Callers can report omission from the actual source, rather than
/// inferring it from a requested limit.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct HistoryWindow {
    pub messages: Vec<Message>,
    pub total_rows: u64,
}

/// One typed raw-history row in the global durable sequence domain.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SequencedMessage {
    pub sequence: i64,
    pub message: Message,
}

/// One bounded source page captured from an exact pinned view and revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSourceSnapshot {
    pub actor_namespace: String,
    pub session_id: String,
    pub source_namespace: String,
    pub view: String,
    pub revision: String,
    pub after_exclusive: i64,
    pub through_inclusive: Option<i64>,
    pub rows: Vec<SequencedMessage>,
}

/// The bounded newest sequenced suffix after one durable session cursor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionHistoryWindowAfter {
    pub namespace: String,
    pub session_id: String,
    pub view: String,
    pub revision: String,
    pub after_exclusive: i64,
    pub rows: Vec<SequencedMessage>,
    pub total_rows: u64,
}

/// Strict producer input for one atomic compaction-summary checkpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSummaryRecord {
    pub actor_namespace: String,
    pub session_id: String,
    pub source_namespace: String,
    pub summary_namespace: String,
    pub source_view: String,
    pub source_revision: String,
    pub after_sequence: i64,
    pub through_sequence: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_actor_id: Option<String>,
    pub invocation_id: String,
    pub summary: String,
}

/// One revision-checked context checkpoint and its producer-private sidecar.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSummaryCheckpoint {
    pub record: ContextSummaryRecord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_reasoning: Vec<ReasoningSummaryRecord>,
}

/// Durable cursor returned without exposing the summary tables or SQL handle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextSummaryCursor {
    pub actor_namespace: String,
    pub session_id: String,
    pub source_namespace: String,
    pub through_sequence: i64,
    pub summary_id: String,
    pub source_view: String,
    pub source_revision: String,
}

/// One current cursor-selected summary and its durable identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextSummaryItem {
    pub summary_id: String,
    pub record: ContextSummaryRecord,
}

/// Bounded current-summary projection from one exact pinned memory view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextSummaryWindow {
    pub actor_namespace: String,
    pub summary_namespace: String,
    pub session_id: Option<String>,
    pub source_namespace: Option<String>,
    pub view: String,
    pub revision: String,
    pub records: Vec<ContextSummaryItem>,
    pub total_rows: u64,
}

/// Reversible durable lifecycle for one public conversation identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleState {
    Active,
    Removed,
}

/// Immutable legacy transcript prefix proved from retained pre-v7 rows.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyTranscriptPrefix {
    pub namespace: String,
    pub source_session_id: String,
    pub source_revision: String,
    pub first_sequence: i64,
    pub through_sequence: i64,
    pub row_count: u64,
    pub record_format: String,
}

/// Exact source identity recorded when one settled public prefix is forked.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionForkProvenance {
    pub source_session_id: String,
    pub source_node_id: String,
    pub source_turn_id: String,
    pub source_label: String,
    pub shares_current_project_memory: bool,
    pub record_format: String,
}

/// One versioned public session catalog row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCatalogRecord {
    pub session_id: String,
    pub mode: Mode,
    pub label: String,
    pub created_order: u64,
    pub updated_order: u64,
    pub lifecycle_generation: u64,
    pub lifecycle_state: SessionLifecycleState,
    pub head_node_id: Option<String>,
    pub pending_node_id: Option<String>,
    pub legacy_prefix: Option<LegacyTranscriptPrefix>,
    pub fork_provenance: Option<SessionForkProvenance>,
    pub record_format: String,
}

/// Stable newest-first continuation for one catalog projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCatalogCursor {
    pub updated_order: u64,
    pub session_id: String,
}

/// One bounded catalog page captured from an exact memory view and revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCatalogPage {
    pub lifecycle_state: Option<SessionLifecycleState>,
    pub view: String,
    pub revision: String,
    pub records: Vec<SessionCatalogRecord>,
    pub total_rows: u64,
    pub next: Option<SessionCatalogCursor>,
}

/// Definite lifecycle refusal proved before a catalog mutation commits.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleRefusal {
    AlreadyExists,
    Missing,
    GenerationChanged,
    AlreadyActive,
    AlreadyRemoved,
    SourceRemoved,
    SourceNodeMissing,
    SourceNodePending,
    SourceNodeUnreachable,
}

/// Typed definite-no-effect result for one checked session lifecycle request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLifecycleRejected(pub SessionLifecycleRefusal);

impl std::fmt::Display for SessionLifecycleRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "session lifecycle request was rejected: {:?}",
            self.0
        )
    }
}

impl std::error::Error for SessionLifecycleRejected {}

/// Immutable receipt result for one committed lifecycle mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionLifecycleOutcome {
    pub session_id: String,
    pub lifecycle_generation: u64,
    pub lifecycle_state: SessionLifecycleState,
    pub updated_order: u64,
    pub record_format: String,
}

/// Exact pre-v7 journal and immutable prefix proof for one safe legacy resume.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LegacySessionTurnResume {
    pub legacy_prefix: LegacyTranscriptPrefix,
    pub journal_key: String,
    pub expected_journal: Value,
}

/// One public-turn transition folded into the existing session checkpoint.
/// The checkpoint's messages provide the exact public entry bytes; this value
/// carries only the catalog authority and transition identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "transition", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionTurnCheckpoint {
    Admit {
        expected_generation: u64,
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// Raw transcript row count observed before this admission. The store
        /// checks it inside the user/public/journal transaction.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_transcript_rows: Option<u64>,
    },
    MarkRetryableInterruption {
        expected_generation: u64,
        turn_id: String,
        speaker_id: String,
    },
    Resume {
        expected_generation: u64,
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        legacy: Option<LegacySessionTurnResume>,
    },
    Settle {
        expected_generation: u64,
        turn_id: String,
        settlement: PublicTurnSettlement,
        speaker_id: String,
    },
}

/// Checked current-mode change committed with the retained runtime state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionModeCheckpoint {
    pub expected_generation: u64,
    pub expected_mode: Mode,
    pub mode: Mode,
}

/// Definite no-effect refusal for a checked public-turn transition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionTurnRefusal {
    SessionMissing,
    SessionRemoved,
    GenerationChanged,
    PendingTurnExists,
    PendingTurnMissing,
    PendingTurnChanged,
    TurnAlreadyExists,
    TurnAlreadySettled,
    RetryableInterruptionAlreadyMarked,
    LegacyPrefixChanged,
    LegacyJournalChanged,
    LabelChanged,
    TranscriptChanged,
}

/// Typed public-turn rejection proved before the checkpoint commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionTurnRejected(pub SessionTurnRefusal);

impl std::fmt::Display for SessionTurnRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "session turn checkpoint was rejected: {:?}",
            self.0
        )
    }
}

impl std::error::Error for SessionTurnRejected {}

/// The sole mutable phase transition of a public turn row.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicTurnSettlement {
    Pending,
    Completed,
    Interrupted,
}

/// Whether a public node admits one user entry or continues an older exact ID.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicTurnKind {
    Primary,
    Continuation,
    LegacyContinuation,
}

/// One pending or settled node in the immutable public predecessor chain.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTurnRecord {
    pub node_id: String,
    pub origin_session_id: String,
    pub turn_id: String,
    pub kind: PublicTurnKind,
    pub continuation_of_node_id: Option<String>,
    pub predecessor_node_id: Option<String>,
    pub settlement: PublicTurnSettlement,
    pub user_entry: Option<Message>,
    pub speaker_id: Option<String>,
    pub terminal_entries: Vec<Message>,
    pub record_format: String,
}

/// One honest public transcript entry from either the immutable v7 turn chain
/// or the proven pre-v7 legacy prefix whose turn and speaker are unknown.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicTranscriptEntry {
    Turn { record: PublicTurnRecord },
    Legacy { sequence: i64, message: Message },
}

/// Typed continuation within one captured public transcript revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublicTranscriptPosition {
    Turn { node_id: String },
    Legacy { sequence: i64 },
}

/// Opaque-to-callers continuation bound to one session, head and revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTranscriptCursor {
    pub session_id: String,
    pub revision: String,
    pub head_node_id: Option<String>,
    pub next: PublicTranscriptPosition,
}

/// One bounded newest-first public transcript page.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTranscriptPage {
    pub session_id: String,
    pub view: String,
    pub revision: String,
    pub head_node_id: Option<String>,
    pub pending: Option<PublicTurnRecord>,
    pub records: Vec<PublicTranscriptEntry>,
    pub total_rows: u64,
    pub next: Option<PublicTranscriptCursor>,
}

/// A conditional checkpoint was proved stale before any effect began.
#[derive(Debug)]
pub struct ContextSummaryStale;

impl std::fmt::Display for ContextSummaryStale {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("context summary source snapshot is stale")
    }
}

impl std::error::Error for ContextSummaryStale {}

#[derive(Clone, Debug)]
pub struct Candidate {
    live: MemoryStore,
    view: MemoryStore,
    base: String,
    promoted: Arc<StdMutex<Option<String>>>,
}

pub(crate) enum CandidateLookup {
    Open(Box<Candidate>),
    Resolved,
    Missing,
}

/// A conservative projection of one exact Kuru candidate ref. A missing ref
/// says nothing about the outcome of an earlier accepted private write.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CandidateRefStatus {
    pub branch: String,
    pub head: Option<String>,
    pub base: Option<String>,
    pub state: CandidateRefState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRefState {
    OpenUnchanged,
    OpenConflict,
    TransitionUncertain,
    Resolved,
    Missing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CandidateInventoryPage {
    pub candidates: Vec<CandidateRefStatus>,
    pub next: Option<String>,
}

/// A selected-ref request rejected before any branch transition begins.
/// Errors after dispatching a transition never use this definite-no-effect type.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRefRefusal {
    Invalid,
    Changed,
    Active,
    SchemaUnverified,
}

#[derive(Debug)]
pub struct CandidateRefRejected(pub CandidateRefRefusal);

impl std::fmt::Display for CandidateRefRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "candidate ref was rejected before transition: {:?}",
            self.0
        )
    }
}

impl std::error::Error for CandidateRefRejected {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidateTransitionObservation {
    Promoted,
    Abandoned,
    AbandonedReclaimed,
    OpenUnchanged,
    OpenConflict,
    Indeterminate,
}

/// A validated candidate cannot fast-forward after another writer moves main.
/// The service maps this one domain error without exposing private SQL detail.
#[derive(Debug)]
pub struct CandidateConflict;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CandidateFailureStage {
    RefInspection,
    SchemaValidation,
    PoolRetirement,
    WorkingSetInspection,
    BranchRename,
    OutcomeReconciliation,
    Cleanup,
    MainMerge,
}

impl CandidateFailureStage {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::RefInspection => "ref_inspection",
            Self::SchemaValidation => "schema_validation",
            Self::PoolRetirement => "pool_retirement",
            Self::WorkingSetInspection => "working_set_inspection",
            Self::BranchRename => "branch_rename",
            Self::OutcomeReconciliation => "outcome_reconciliation",
            Self::Cleanup => "cleanup",
            Self::MainMerge => "main_merge",
        }
    }
}

impl std::fmt::Display for CandidateFailureStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for CandidateFailureStage {}

#[cfg(any(test, feature = "test-support"))]
fn candidate_branch_rename_reason(
    stage: CandidateFailureStage,
    sqlstate: &str,
    vendor: u16,
    message: &str,
) -> &'static str {
    // Dolt 2.3.3's branch procedure returns this fixed message only when an
    // active session prevents the checked rename. Keep its text private.
    const BRANCH_IN_USE: &str = "unsafe to delete or rename branches in use in other sessions; use --force to force the change";
    if matches!(stage, CandidateFailureStage::BranchRename)
        && sqlstate == "HY000"
        && vendor == 1105
        && message == BRANCH_IN_USE
    {
        "branch_in_use"
    } else {
        "other"
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn candidate_failure_record(error: &anyhow::Error) -> Option<String> {
    let stage = error.downcast_ref::<CandidateFailureStage>()?;
    let mut class = "non_sql";
    let mut sqlstate = "none";
    let mut vendor = 0;
    let mut reason = "other";
    if let Some(sqlx_error) = error.downcast_ref::<sqlx::Error>() {
        class = "sqlx";
        if let sqlx::Error::Database(database) = sqlx_error {
            class = "database";
            if let Some(mysql) = database.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
                sqlstate = mysql
                    .code()
                    .filter(|code| {
                        code.len() == 5
                            && code
                                .bytes()
                                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                    })
                    .unwrap_or("other");
                vendor = mysql.number();
                reason =
                    candidate_branch_rename_reason(*stage, sqlstate, vendor, database.message());
            }
        }
    }
    Some(format!(
        "candidate_owner stage={} class={class} sqlstate={sqlstate} vendor={vendor} reason={reason}",
        stage.label()
    ))
}

impl std::fmt::Display for CandidateConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("dream candidate is stale: live memory changed since its base")
    }
}

impl std::error::Error for CandidateConflict {}

impl Candidate {
    pub fn view(&self) -> MemoryStore {
        self.view.clone()
    }
    pub(crate) fn branch(&self) -> &str {
        self.view.pinned_view()
    }
    pub fn base(&self) -> &str {
        &self.base
    }
    pub async fn promote(&self) -> Result<String> {
        self.promote_checked(None).await
    }

    pub(crate) async fn promote_exact(&self, expected_target: &str) -> Result<String> {
        self.promote_checked(Some(expected_target)).await
    }

    async fn promote_checked(&self, expected_target: Option<&str>) -> Result<String> {
        if let Some(target) = self.promoted.lock().expect("candidate result lock").clone() {
            ensure!(
                expected_target.is_none_or(|expected| expected == target),
                CandidateConflict
            );
            return Ok(target);
        }
        self.live.writable()?;
        let guard = self.live.shared.write.clone().lock_owned().await;
        self.live.resolve_uncertain().await?;
        if let Some(target) = self.promoted.lock().expect("candidate result lock").clone() {
            ensure!(
                expected_target.is_none_or(|expected| expected == target),
                CandidateConflict
            );
            return Ok(target);
        }
        let live = self.live.clone();
        let base = self.base.clone();
        let promoted = self.promoted.clone();
        let names = CandidateNames::from_open(&self.view.branch)?;
        let expected_target = expected_target.map(str::to_owned);
        // Keep accepted promotion alive if the UI cancels while awaiting its reply.
        tokio::spawn(async move {
            let _guard = guard;
            let before = candidate_heads(&live.pool, &names)
                .await
                .context(CandidateFailureStage::RefInspection)?;
            ensure!(
                !before.contains_key(&names.abandoned),
                "dream candidate was already abandoned"
            );
            if let Some(expected) = &expected_target {
                ensure!(
                    !before.is_empty() && before.values().all(|head| head == expected),
                    CandidateConflict
                );
            }
            let current = live.revision().await?;
            let target = match before.get(&names.promoting) {
                Some(target) => {
                    validate_candidate_pair(&before, &names.open, &names.promoting, target)?;
                    target.clone()
                }
                None => {
                    let target = before
                        .get(&names.open)
                        .context("dream candidate ref is missing")?
                        .clone();
                    if current != base {
                        return Err(CandidateConflict.into());
                    }
                    ensure_branch_clean(&live, &names.open)
                        .await
                        .context(CandidateFailureStage::WorkingSetInspection)?;
                    transition_candidate(&live, &names.open, &names.promoting, &target).await?;
                    target
                }
            };
            if current == target {
                #[cfg(test)]
                fail_candidate_cleanup_once(&live)?;
                live.shared
                    .server
                    .retire_pool(&names.open)
                    .await
                    .context(CandidateFailureStage::PoolRetirement)?;
                cleanup_promoted_candidate(&live, &names, &target)
                    .await
                    .context(CandidateFailureStage::Cleanup)?;
                *promoted.lock().expect("candidate result lock") = Some(target.clone());
                return Ok(target);
            }
            if current != base {
                return Err(CandidateConflict.into());
            }
            let (mut connection, id) = owned_connection(&live.pool).await?;
            *live.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: live.pool.clone(),
                connection: id,
                receipt: Receipt::Promotion {
                    base,
                    target: target.clone(),
                },
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query("CALL DOLT_MERGE(?, '--ff-only')")
                    .bind(&names.promoting)
                    .fetch_all(&mut connection),
            )
            .await;
            drop(connection);
            let committed = live
                .resolve_uncertain()
                .await
                .context(CandidateFailureStage::OutcomeReconciliation)?
                == Some(true);
            if !committed {
                result
                    .context("Dolt promotion deadline exceeded")
                    .context(CandidateFailureStage::MainMerge)?
                    .context(CandidateFailureStage::MainMerge)?;
                bail!("Dolt did not fast-forward to the candidate revision");
            }
            #[cfg(test)]
            fail_candidate_cleanup_once(&live)?;
            live.shared
                .server
                .retire_pool(&names.open)
                .await
                .context(CandidateFailureStage::PoolRetirement)?;
            cleanup_promoted_candidate(&live, &names, &target)
                .await
                .context(CandidateFailureStage::Cleanup)?;
            *promoted.lock().expect("candidate result lock") = Some(target.clone());
            Ok(target)
        })
        .await
        .context("memory promotion worker failed")?
    }

    pub async fn abandon(&self) -> Result<()> {
        self.abandon_checked(None).await
    }

    pub(crate) async fn abandon_exact(&self, expected_target: &str) -> Result<()> {
        self.abandon_checked(Some(expected_target)).await
    }

    async fn abandon_checked(&self, expected_target: Option<&str>) -> Result<()> {
        if self
            .promoted
            .lock()
            .expect("candidate result lock")
            .is_some()
        {
            return Ok(());
        }
        self.live.writable()?;
        let live = self.live.clone();
        let promoted = self.promoted.clone();
        let names = CandidateNames::from_open(&self.view.branch)?;
        let expected_target = expected_target.map(str::to_owned);
        tokio::spawn(async move {
            let guard = live.shared.write.clone().lock_owned().await;
            let _guard = guard;
            live.resolve_uncertain()
                .await
                .context(CandidateFailureStage::OutcomeReconciliation)?;
            if promoted.lock().expect("candidate result lock").is_some() {
                return Ok(());
            }
            if let Some(expected) = &expected_target {
                let heads = candidate_heads(&live.pool, &names)
                    .await
                    .context(CandidateFailureStage::RefInspection)?;
                ensure!(
                    heads.values().all(|head| head == expected),
                    CandidateConflict
                );
                ensure!(!heads.is_empty(), CandidateConflict);
            }
            live.shared
                .server
                .retire_pool(&names.open)
                .await
                .context(CandidateFailureStage::PoolRetirement)?;
            abandon_candidate(&live, &names).await
        })
        .await
        .context("memory candidate abandonment worker failed")?
    }
}

#[derive(Debug)]
struct CandidateNames {
    open: String,
    promoting: String,
    abandoned: String,
}

impl CandidateNames {
    fn from_open(open: &str) -> Result<Self> {
        let suffix = open
            .strip_prefix(CANDIDATE_PREFIX)
            .context("candidate branch has an invalid name")?;
        let id = Uuid::parse_str(suffix).context("candidate branch has an invalid identity")?;
        ensure!(
            id.simple().to_string() == suffix,
            "candidate branch identity is not canonical"
        );
        Ok(Self {
            open: open.to_owned(),
            promoting: format!("{PROMOTING_PREFIX}{suffix}"),
            abandoned: format!("{ABANDONED_PREFIX}{suffix}"),
        })
    }

    fn from_status(status: &str) -> Result<Self> {
        let suffix = status
            .strip_prefix(PROMOTING_PREFIX)
            .or_else(|| status.strip_prefix(ABANDONED_PREFIX))
            .context("candidate status branch has an invalid name")?;
        Self::from_open(&format!("{CANDIDATE_PREFIX}{suffix}"))
    }

    fn from_status_or_open(branch: &str) -> Result<Self> {
        if branch.starts_with(CANDIDATE_PREFIX) {
            Self::from_open(branch)
        } else {
            Self::from_status(branch)
        }
    }
}

async fn candidate_heads(
    pool: &MySqlPool,
    names: &CandidateNames,
) -> Result<BTreeMap<String, String>> {
    let rows: Vec<(String, String)> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_as(
            "SELECT name, hash FROM dolt_branches WHERE BINARY name = BINARY ? OR BINARY name = BINARY ? OR BINARY name = BINARY ? ORDER BY BINARY name LIMIT 4",
        )
        .bind(&names.open)
        .bind(&names.promoting)
        .bind(&names.abandoned)
        .fetch_all(pool),
    )
    .await
    .context("candidate branch observation deadline exceeded")??;
    ensure!(rows.len() <= 3, "candidate branch observation is ambiguous");
    Ok(rows.into_iter().collect())
}

fn validate_candidate_pair(
    heads: &BTreeMap<String, String>,
    first: &str,
    status: &str,
    expected: &str,
) -> Result<()> {
    ensure!(
        heads.get(status).is_some_and(|head| head == expected),
        "candidate status branch does not retain its expected head"
    );
    if let Some(head) = heads.get(first) {
        ensure!(
            head == expected,
            "candidate transition retained divergent source and status refs"
        );
    }
    Ok(())
}

async fn candidate_branch_is_clean(store: &MemoryStore, branch: &str) -> Result<bool> {
    let pool = store
        .shared
        .server
        .pool(branch)
        .await
        .context(CandidateFailureStage::WorkingSetInspection)?;
    let result = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status").fetch_one(pool.as_ref()),
    )
    .await
    .context("candidate working-set inspection deadline exceeded")
    .context(CandidateFailureStage::WorkingSetInspection)?;
    let result = result
        .map_err(anyhow::Error::from)
        .context(CandidateFailureStage::WorkingSetInspection);
    let cleanup = store
        .shared
        .server
        .retire_pool(branch)
        .await
        .context(CandidateFailureStage::PoolRetirement);
    match (result, cleanup) {
        (Ok(dirty), Ok(())) => Ok(dirty == 0),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("candidate pool cleanup also failed: {cleanup:#}")))
        }
    }
}

async fn ensure_branch_clean(store: &MemoryStore, branch: &str) -> Result<()> {
    ensure!(
        candidate_branch_is_clean(store, branch).await?,
        "candidate working set is not clean"
    );
    Ok(())
}

async fn preserve_resolved_cleanup(
    store: &MemoryStore,
    names: &CandidateNames,
    authority: &str,
    expected: &str,
    error: anyhow::Error,
) -> Result<()> {
    if store
        .shared
        .uncertain
        .lock()
        .expect("uncertain lock")
        .is_some()
    {
        return Err(error);
    }
    let heads = candidate_heads(&store.pool, names).await?;
    for head in heads.values() {
        ensure!(
            head == expected,
            "candidate cleanup retained divergent refs"
        );
    }
    ensure!(
        heads.is_empty() || heads.contains_key(authority),
        "candidate cleanup lost its durable status authority"
    );
    Ok(())
}

async fn transition_candidate(
    store: &MemoryStore,
    source: &str,
    status: &str,
    expected: &str,
) -> Result<()> {
    transition_candidate_with_retirement_deadline(store, source, status, expected, QUERY_TIMEOUT)
        .await
}

async fn transition_candidate_with_retirement_deadline(
    store: &MemoryStore,
    source: &str,
    status: &str,
    expected: &str,
    retirement_deadline: Duration,
) -> Result<()> {
    let source_admission = store
        .shared
        .server
        .fence_pool(source)
        .await
        .context(CandidateFailureStage::PoolRetirement)?;
    store
        .shared
        .server
        .retire_pool(source)
        .await
        .context(CandidateFailureStage::PoolRetirement)?;
    await_branch_sessions_end(store, source, retirement_deadline)
        .await
        .context(CandidateFailureStage::PoolRetirement)?;
    let (mut connection, id) = owned_connection(&store.pool).await?;
    *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::CandidateTransition {
            source: source.to_owned(),
            status: status.to_owned(),
            expected: expected.to_owned(),
        },
    });
    let result = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH('-m', ?, ?)")
            .bind(source)
            .bind(status)
            .fetch_all(&mut connection),
    )
    .await;
    drop(connection);
    let settled = store
        .resolve_uncertain()
        .await
        .context(CandidateFailureStage::OutcomeReconciliation)?
        == Some(true);
    let names = CandidateNames::from_status(status)?;
    let heads = candidate_heads(&store.pool, &names)
        .await
        .context(CandidateFailureStage::RefInspection)?;
    // A successful transition may inspect the source again to verify its clean
    // working set. Release admission only after the rename is reconciled and
    // the exact refs are read, before that later inspection can reopen a pool.
    drop(source_admission);
    if settled {
        for branch in [source, status] {
            if heads.contains_key(branch) {
                ensure_branch_clean(store, branch).await?;
            }
        }
        return Ok(());
    }
    if let Some(head) = heads.get(source) {
        ensure!(
            head == expected,
            "candidate transition changed the source ref unexpectedly"
        );
    }
    result
        .context("candidate status transition deadline exceeded")
        .context(CandidateFailureStage::BranchRename)?
        .context(CandidateFailureStage::BranchRename)?;
    bail!("candidate status transition did not retain its durable ref")
}

async fn delete_candidate_ref(
    store: &MemoryStore,
    branch: &str,
    expected: &str,
    force: bool,
) -> Result<()> {
    let before =
        candidate_heads(&store.pool, &CandidateNames::from_status_or_open(branch)?).await?;
    let Some(head) = before.get(branch) else {
        return Ok(());
    };
    ensure!(
        head == expected,
        "candidate cleanup found an unexpected ref head"
    );
    store.shared.server.retire_pool(branch).await?;
    if force {
        confirm_no_live_candidate_session(store, branch, expected).await?;
    }
    let (mut connection, id) = owned_connection(&store.pool).await?;
    *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::CandidateDeletion {
            branch: branch.to_owned(),
            expected: expected.to_owned(),
        },
    });
    let flag = if force { "-D" } else { "-d" };
    let result = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind(flag)
            .bind(branch)
            .fetch_all(&mut connection),
    )
    .await;
    drop(connection);
    let settled = store.resolve_uncertain().await? == Some(true);
    let after = candidate_heads(&store.pool, &CandidateNames::from_status_or_open(branch)?).await?;
    if settled {
        return Ok(());
    }
    ensure!(
        after.get(branch).is_some_and(|head| head == expected),
        "candidate cleanup changed the ref unexpectedly"
    );
    result.context("candidate deletion deadline exceeded")??;
    bail!("Dolt did not delete the resolved candidate branch")
}

async fn confirm_no_live_candidate_session(
    store: &MemoryStore,
    branch: &str,
    expected: &str,
) -> Result<()> {
    let (mut connection, id) = owned_connection(&store.pool).await?;
    *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
        pool: store.pool.clone(),
        connection: id,
        receipt: Receipt::CandidateExclusion {
            branch: branch.to_owned(),
            expected: expected.to_owned(),
        },
    });
    let result = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH('-m', ?, ?)")
            .bind(branch)
            .bind(branch)
            .fetch_all(&mut connection),
    )
    .await;
    drop(connection);
    let settled = store.resolve_uncertain().await? == Some(true);
    result.context("candidate session exclusion deadline exceeded")??;
    ensure!(settled, "candidate session exclusion was not confirmed");
    let heads = candidate_heads(&store.pool, &CandidateNames::from_status_or_open(branch)?).await?;
    ensure!(
        heads.get(branch).is_some_and(|head| head == expected),
        "candidate session exclusion changed the resolved ref"
    );
    Ok(())
}

async fn cleanup_promoted_candidate(
    store: &MemoryStore,
    names: &CandidateNames,
    target: &str,
) -> Result<()> {
    let heads = candidate_heads(&store.pool, names).await?;
    ensure!(
        !heads.contains_key(&names.abandoned),
        "promoted candidate also has an abandoned ref"
    );
    validate_candidate_pair(&heads, &names.open, &names.promoting, target)?;
    for branch in [&names.open, &names.promoting] {
        if heads.contains_key(branch) {
            ensure_branch_clean(store, branch).await?;
        }
    }
    if heads.contains_key(&names.open) {
        delete_candidate_ref(store, &names.open, target, false).await?;
    }
    delete_candidate_ref(store, &names.promoting, target, false).await
}

async fn cleanup_abandoned_candidate(
    store: &MemoryStore,
    names: &CandidateNames,
    target: &str,
    heads: &BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        heads
            .get(&names.abandoned)
            .is_some_and(|head| head == target),
        "abandoned candidate did not retain its durable status ref"
    );
    for branch in [&names.open, &names.promoting, &names.abandoned] {
        if let Some(head) = heads.get(branch) {
            ensure!(head == target, "abandoned candidate refs diverged");
            ensure_branch_clean(store, branch).await?;
        }
    }
    // Delete duplicates before the status authority so any partial cleanup
    // remains explicitly recoverable.
    for branch in [&names.open, &names.promoting, &names.abandoned] {
        if heads.contains_key(branch) {
            delete_candidate_ref(store, branch, target, true).await?;
        }
    }
    Ok(())
}

async fn abandon_candidate(store: &MemoryStore, names: &CandidateNames) -> Result<()> {
    let mut heads = candidate_heads(&store.pool, names)
        .await
        .context(CandidateFailureStage::RefInspection)?;
    let target = if let Some(target) = heads.get(&names.abandoned) {
        target.clone()
    } else if let Some(target) = heads.get(&names.promoting) {
        validate_candidate_pair(&heads, &names.open, &names.promoting, target)?;
        let target = target.clone();
        transition_candidate(store, &names.promoting, &names.abandoned, &target).await?;
        target
    } else {
        let target = heads
            .get(&names.open)
            .context("dream candidate ref is missing")?
            .clone();
        ensure_branch_clean(store, &names.open)
            .await
            .context(CandidateFailureStage::WorkingSetInspection)?;
        transition_candidate(store, &names.open, &names.abandoned, &target).await?;
        target
    };
    heads = candidate_heads(&store.pool, names)
        .await
        .context(CandidateFailureStage::RefInspection)?;
    cleanup_abandoned_candidate(store, names, &target, &heads)
        .await
        .context(CandidateFailureStage::Cleanup)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Revision {
    pub hash: String,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub struct MemoryStatus {
    pub engine: &'static str,
    pub engine_version: &'static str,
    pub project: String,
    pub directory: PathBuf,
    pub branch: String,
    pub revision: String,
    pub read_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    format: u32,
    project_scope: String,
    initial_revision: String,
    migration: Option<MigrationReceipt>,
}

struct StoppedStage {
    // Hold the existing lifecycle inode through rename and directory fsync.
    _lease: LifecycleLease,
}

mod export;
pub(crate) mod marker_fixture;
mod migrations;
pub(crate) mod purge;
mod usage_ledger;
pub use export::{ActiveExportSnapshot, ExportCursor, ExportPage, ExportProvenance, StorageRecord};
pub use usage_ledger::{UsageLedger, UsageProof};

impl MemoryStore {
    pub(crate) fn ensure_project_scope(&self, scope: &str) -> Result<()> {
        ensure!(
            self.shared.fixture_unbound_scope || self.shared.project_scope == scope,
            "memory view belongs to a different canonical project"
        );
        Ok(())
    }

    pub(crate) fn service_instance(&self) -> &str {
        self.shared.server.instance()
    }

    pub(crate) fn pinned_view(&self) -> &str {
        &self.branch
    }

    /// Attach a caller-retained identity to this one view clone. The compact
    /// digest binds the method, pinned branch, store and encoded arguments;
    /// no request body is copied into the receipt row.
    pub(crate) fn with_logical_receipt(
        &self,
        id: Uuid,
        method: &'static str,
        encoded_arguments: &[u8],
    ) -> Self {
        let argument_digest = Sha256::digest(encoded_arguments);
        let argument_digest = argument_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let logical = self.logical_receipt_for_view(&self.branch, id, method, &argument_digest);
        let mut view = self.clone();
        view.logical_receipt = Some(logical);
        view
    }

    fn logical_receipt_for_view(
        &self,
        branch: &str,
        id: Uuid,
        method: &str,
        argument_digest: &str,
    ) -> LogicalReceipt {
        let mut hash = Sha256::new();
        for field in [
            b"kuru-memory-operation-v1".as_slice(),
            self.shared.server.instance().as_bytes(),
            branch.as_bytes(),
            method.as_bytes(),
            argument_digest.as_bytes(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        let digest = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        // Dolt branches inherit the operation rows present at their base.
        // Mapping the caller UUID through the pinned view gives each branch a
        // distinct physical PK, even when an inherited main receipt has the
        // same logical UUID. The original UUID remains the query/retry key.
        let mut physical = Sha256::new();
        for field in [
            b"kuru-memory-receipt-view-v1".as_slice(),
            self.shared.server.instance().as_bytes(),
            branch.as_bytes(),
            id.as_bytes(),
        ] {
            physical.update((field.len() as u64).to_be_bytes());
            physical.update(field);
        }
        let physical_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &physical.finalize()).to_string();
        LogicalReceipt {
            physical_id,
            method: method.to_owned(),
            digest,
        }
    }

    /// Inspect only the indexed row for an exact original view. A candidate
    /// receipt may still be on its open/status ref or may have reached main by
    /// checked promotion; another view's inherited rows never prove it.
    pub(crate) async fn indexed_logical_outcome(
        &self,
        branch: &str,
        id: Uuid,
        method: &str,
        argument_digest: &str,
    ) -> Result<Option<bool>> {
        self.readable()?;
        ensure!(
            !method.is_empty() && method.len() <= 64 && method.is_ascii(),
            "invalid logical mutation method"
        );
        ensure!(
            argument_digest.len() == 64
                && argument_digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "invalid logical mutation argument digest"
        );
        let names = if branch == "main" {
            None
        } else {
            Some(CandidateNames::from_open(branch)?)
        };
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let expected = self.logical_receipt_for_view(branch, id, method, argument_digest);
        if operation_receipt_matches(&self.pool, &expected).await? {
            return Ok(Some(true));
        }
        if let Some(names) = names {
            let heads = candidate_heads(&self.pool, &names).await?;
            if heads.is_empty() {
                // A formerly writable candidate may have been explicitly
                // abandoned and reclaimed. A missing ref cannot prove its
                // earlier accepted write never committed on that view.
                return Ok(None);
            }
            for branch in [&names.open, &names.promoting, &names.abandoned] {
                if heads.contains_key(branch) {
                    let pool = self.shared.server.pool(branch).await?;
                    let found = operation_receipt_matches(&pool, &expected).await;
                    pool.close().await;
                    if found? {
                        return Ok(Some(true));
                    }
                }
            }
        }
        Ok(Some(false))
    }

    pub fn exists(data_dir: &Path, project_scope: &str) -> Result<bool> {
        purge::ensure_open_allowed(data_dir, project_scope)?;
        let path = project_directory(data_dir, project_scope)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "project memory must be a regular directory"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        read_activation(&path, project_scope)?;
        Ok(true)
    }

    pub async fn open(options: OpenOptions) -> Result<Self> {
        let mut progress = ProgressReporter::silent();
        Self::open_inner(options, None, None, None, &mut progress).await
    }

    /// Open memory and return bounded, optional observations of startup work.
    ///
    /// The returned future owns the ordinary open operation. Dropping the
    /// progress receiver changes only whether observations are delivered.
    pub fn open_observed(
        options: OpenOptions,
    ) -> (
        MemoryOpenProgress,
        impl std::future::Future<Output = Result<Self>> + Send + 'static,
    ) {
        let (progress, reporter) = ProgressReporter::observed();
        let opening = async move {
            let mut reporter = reporter;
            Self::open_inner(options, None, None, None, &mut reporter).await
        };
        (progress, opening)
    }

    async fn open_inner(
        options: OpenOptions,
        temporary: Option<Arc<tempfile::TempDir>>,
        permit: Option<OwnedSemaphorePermit>,
        mut marker_pause: Option<marker_fixture::ReadyMarkerPause>,
        progress: &mut ProgressReporter,
    ) -> Result<Self> {
        options.config.validate()?;
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        #[cfg(not(windows))]
        private_dir(&options.data_dir)?;
        #[cfg(windows)]
        private_dir(&options.data_dir)
            .map_err(|error| migration::data_directory_error(&options.data_dir, error))?;
        let parent = directory.parent().context("project store has no parent")?;
        private_dir(parent)?;
        let locks = parent.join("locks");
        private_dir(&locks)?;
        let lock_directory =
            files::open_directory(&locks, Privacy::OwnerOnly, NameRetention::Pinned)?;
        let name = directory.file_name().context("project store has no name")?;
        let lock = lock_directory.lock_file(name)?;
        progress.report(MemoryOpenStage::WaitingForProjectOwnership);
        let mut lock = Some(
            acquire_lock(
                lock,
                Duration::from_secs(options.config.startup_timeout_secs),
            )
            .await?,
        );
        lock_directory.verify(name, lock.as_ref().expect("startup lock"))?;
        purge::ensure_open_allowed(&options.data_dir, &options.project_scope)?;
        let binary = provision::provision_observed(
            &options.config,
            &options.data_dir.join("tools/dolt"),
            progress,
        )
        .await?;
        let supervisor = options
            .supervisor
            .clone()
            .unwrap_or(std::env::current_exe()?);
        let timeout = Duration::from_secs(options.config.startup_timeout_secs);
        #[cfg(unix)]
        let lifecycle_root = None;
        #[cfg(windows)]
        let lifecycle_root = Some(options.data_dir.join("memory/lifecycles"));
        let make_options = |path: PathBuf, read_only| ServerOptions {
            binary: binary.clone(),
            directory: path,
            project_scope: options.project_scope.clone(),
            supervisor: supervisor.clone(),
            timeout,
            read_only,
            retained: temporary.clone(),
            lifecycle_root: lifecycle_root.clone(),
        };
        progress.report(MemoryOpenStage::PreparingDatabase);
        if !Self::exists(&options.data_dir, &options.project_scope)? {
            ensure!(
                !options.read_only,
                "project memory has not been migrated or initialized; open Kuru normally first"
            );
            let data = options.data_dir.clone();
            let scope = options.project_scope.clone();
            let legacy =
                if Self::legacy_import_suppressed(&options.data_dir, &options.project_scope)? {
                    None
                } else {
                    tokio::task::spawn_blocking(move || migration::prepare(&data, &scope)).await??
                };
            let recovered = recover_staging(
                &directory,
                &options.project_scope,
                legacy.as_ref(),
                &make_options,
                &mut lock,
                progress,
            )
            .await?;
            let mut staging = if let Some(staging) = recovered {
                staging
            } else {
                let staging = parent.join(format!(
                    "{}.staging-{}",
                    name.to_string_lossy(),
                    Uuid::new_v4()
                ));
                private_dir(&staging)?;
                progress.report(MemoryOpenStage::OpeningDatabase);
                let server = Server::open_with_guard(
                    make_options(staging.clone(), false),
                    lock.take().expect("startup lock"),
                )
                .await
                .context("open staged memory server")?;
                let pool = server.pool("main").await.context("open staged main pool")?;
                let initialized = async {
                    initialize(&pool).await?;
                    if let Some(legacy) = &legacy {
                        import(&pool, legacy).await?;
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                match (initialized, close_migration_worker(server, pool).await) {
                    (Ok(()), Ok(returned_lock)) => lock = Some(returned_lock),
                    (Err(error), Ok(returned_lock)) => {
                        let retained_lock = returned_lock;
                        if let Err(preserve) = preserve_unready_stage(
                            &staging,
                            parent,
                            lifecycle_root.as_deref(),
                            timeout,
                        )
                        .await
                        {
                            return Err(error.context(format!(
                                "memory staging initialization preservation also failed: {preserve:#}"
                            )));
                        }
                        drop(retained_lock);
                        return Err(error);
                    }
                    (Ok(()), Err(cleanup)) => return Err(cleanup),
                    (Err(error), Err(cleanup)) => {
                        return Err(error.context(format!(
                            "memory staging initialization cleanup also failed: {cleanup:#}"
                        )));
                    }
                }

                // Migration itself is an accepted worker just as it is for an
                // existing project.  A cancelled stage opener cannot abandon
                // DDL or release its writer lock before the supervisor reaps.
                progress.report(MemoryOpenStage::OpeningDatabase);
                let server = Server::open_with_guard(
                    make_options(staging.clone(), false),
                    lock.take().expect("startup lock"),
                )
                .await
                .context("reopen staged memory server for migration")?;
                let pool = server.pool("main").await.context("open staged main pool")?;
                #[cfg(test)]
                let (returned_lock, migrated) =
                    run_migration_worker(server, pool, options.migration_hooks.clone()).await?;
                #[cfg(not(test))]
                let (returned_lock, migrated) = run_migration_worker(server, pool).await?;
                lock = Some(returned_lock);
                if let Err(error) = migrated {
                    if let Err(preserve) =
                        preserve_unready_stage(&staging, parent, lifecycle_root.as_deref(), timeout)
                            .await
                    {
                        return Err(error.context(format!(
                            "memory staging migration preservation also failed: {preserve:#}"
                        )));
                    }
                    return Err(error);
                }

                progress.report(MemoryOpenStage::OpeningDatabase);
                let server = Server::open_with_guard(
                    make_options(staging.clone(), false),
                    lock.take().expect("startup lock"),
                )
                .await
                .context("reopen migrated staged memory server")?;
                let pool = server
                    .pool("main")
                    .await
                    .context("open migrated staged main pool")?;
                let activated = async {
                    migrations::validate_active(&server, &pool).await?;
                    let initial_revision = revision(&pool).await?;
                    let activation = Activation {
                        format: 1,
                        project_scope: options.project_scope.clone(),
                        initial_revision,
                        migration: legacy.as_ref().map(|legacy| legacy.receipt.clone()),
                    };
                    marker_fixture::reach(
                        &mut marker_pause,
                        marker_fixture::Boundary::Before,
                        &staging,
                        &activation,
                    )
                    .await?;
                    write_json(&staging.join("ready.json"), &activation)?;
                    marker_fixture::reach(
                        &mut marker_pause,
                        marker_fixture::Boundary::After,
                        &staging,
                        &activation,
                    )
                    .await?;
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                match (activated, close_migration_worker(server, pool).await) {
                    (Ok(()), Ok(returned_lock)) => lock = Some(returned_lock),
                    (Err(error), Ok(returned_lock)) => {
                        let retained_lock = returned_lock;
                        if !staging.join("ready.json").exists()
                            && let Err(preserve) = preserve_unready_stage(
                                &staging,
                                parent,
                                lifecycle_root.as_deref(),
                                timeout,
                            )
                            .await
                        {
                            return Err(error.context(format!(
                                "memory staging activation preservation also failed: {preserve:#}"
                            )));
                        }
                        drop(retained_lock);
                        return Err(error);
                    }
                    (Ok(()), Err(cleanup)) => return Err(cleanup),
                    (Err(error), Err(cleanup)) => {
                        return Err(error.context(format!(
                            "memory staging activation cleanup also failed: {cleanup:#}"
                        )));
                    }
                }
                let lease =
                    Server::quiescence_at(&staging, lifecycle_root.as_deref(), timeout).await?;
                StoppedStage { _lease: lease }
            };
            // A live Dolt data directory must never be renamed.
            staging
                ._lease
                .move_to(&directory)
                .context("cannot activate validated Dolt memory")?;
            drop(staging);
        }
        read_activation(&directory, &options.project_scope)?;
        progress.report(MemoryOpenStage::OpeningDatabase);
        let server = Server::open_with_guard(
            make_options(directory.clone(), options.read_only),
            lock.take().expect("startup lock"),
        )
        .await
        .context("open active memory server")?;
        let pool = server.pool("main").await.context("open active main pool")?;
        let found = migrations::version(&pool).await?;
        if options.read_only && found < migrations::CURRENT_VERSION {
            migrations::validate_supported(&pool).await?;
            let lock: File = server.close_installed_guard().await?;
            drop(lock);
            bail!(
                "memory schema version {found} requires writable upgrade to {}",
                migrations::CURRENT_VERSION
            );
        }
        if found > migrations::CURRENT_VERSION {
            migrations::validate_supported(&pool).await?;
            bail!("unsupported Dolt memory schema version {found}");
        }
        let (server, pool) = if found < migrations::CURRENT_VERSION {
            #[cfg(test)]
            let (lock, migrated) =
                run_migration_worker(server, pool, options.migration_hooks.clone()).await?;
            #[cfg(not(test))]
            let (lock, migrated) = run_migration_worker(server, pool).await?;
            migrated?;
            progress.report(MemoryOpenStage::OpeningDatabase);
            let server = Server::open_with_guard(make_options(directory.clone(), false), lock)
                .await
                .context("reopen migrated memory server")?;
            let pool = server
                .pool("main")
                .await
                .context("open migrated main pool")?;
            (server, pool)
        } else {
            (server, pool)
        };
        if options.read_only {
            migrations::validate_inspection(&server, &pool).await?;
        } else {
            migrations::validate_active(&server, &pool).await?;
        }
        let shared = Arc::new(Shared {
            server,
            directory,
            project_scope: options.project_scope,
            fixture_unbound_scope: temporary.is_some(),
            read_only: options.read_only,
            write: Arc::new(Mutex::new(())),
            dream: Arc::new(Mutex::new(())),
            uncertain: StdMutex::new(None),
            usage_pool: StdMutex::new(None),
            #[cfg(test)]
            candidate_recovery_pause: options.candidate_recovery_pause,
            #[cfg(test)]
            candidate_cleanup_failure: options.candidate_cleanup_failure,
            _permit: permit,
        });
        let store = Self {
            shared,
            pool,
            branch: "main".into(),
            logical_receipt: None,
        };
        if !options.read_only {
            run_candidate_recovery_worker(&store).await?;
            usage_ledger::establish(&store).await?;
        } else {
            let lock: File = store.shared.server.take_reap_guard();
            drop(lock);
        }
        progress.report(MemoryOpenStage::Ready);
        Ok(store)
    }

    /// Real isolated Dolt fixture. Missing runtime/helper is an error, never a skip.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn temporary() -> Result<Self> {
        static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let permit = PERMITS
            .get_or_init(|| Arc::new(Semaphore::new(4)))
            .clone()
            .acquire_owned()
            .await?;
        let directory = tempfile::Builder::new().prefix("kuru-memory-").tempdir()?;
        let data = directory.path().join("private");
        let mut options = OpenOptions::new(data, format!("project/{}", "0".repeat(64)));
        options.config.cache_dir = Some(test_cache());
        options.config.offline = true;
        options.supervisor = Some(test_supervisor()?);
        let mut progress = ProgressReporter::silent();
        let retained = Arc::new(directory);
        let fixture_options = options.clone();
        let opened = Self::open_inner(
            options,
            Some(retained.clone()),
            Some(permit),
            None,
            &mut progress,
        )
        .await;
        let result = opened
            .map_err(|error| crate::test_support::fixture_startup_error(&fixture_options, error));
        // The stage and its server.log must still exist while the error is annotated.
        drop(retained);
        result
    }

    fn readable(&self) -> Result<()> {
        ensure!(!self.pool.is_closed(), "memory store is closed");
        Ok(())
    }

    fn writable(&self) -> Result<()> {
        ensure!(!self.shared.read_only, "this memory view is read-only");
        self.readable()
    }

    /// Return the memory-owned, permanent operational usage ledger. The
    /// opaque handle does not expose its Dolt branch or SQL connection.
    pub fn usage_ledger(&self) -> Result<UsageLedger> {
        self.readable()?;
        let pool = self
            .shared
            .usage_pool
            .lock()
            .expect("usage pool lock")
            .clone()
            .context("usage ledger is unavailable in this read-only or pre-ledger store")?;
        Ok(UsageLedger::new(MemoryStore {
            shared: self.shared.clone(),
            pool,
            branch: usage_ledger::BRANCH.into(),
            logical_receipt: self.logical_receipt.clone(),
        }))
    }

    pub async fn append(&self, namespace: &str, role: &str, content: &str) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        identifier("role", role, 128)?;
        self.mutate(
            "message",
            Mutation::Append {
                namespace: namespace.into(),
                session_id: None,
                role: role.into(),
                content: content.into(),
                format: None,
            },
        )
        .await
    }
    /// Append one typed message without interpreting any legacy text as JSON.
    pub async fn append_message(&self, namespace: &str, message: &Message) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        identifier("role", &message.role, 128)?;
        ensure!(
            self.schema_version().await? >= 3,
            "typed messages require an upgraded memory view"
        );
        self.mutate(
            "message",
            Mutation::Append {
                namespace: namespace.into(),
                session_id: None,
                role: message.role.clone(),
                content: encode_typed_message(message)?,
                format: Some(TYPED_FORMAT),
            },
        )
        .await
    }

    /// Append one raw private-history row with durable session provenance.
    pub async fn append_session_message(
        &self,
        namespace: &str,
        session_id: &str,
        message: &Message,
    ) -> Result<()> {
        validate_session_message(namespace, session_id, message)?;
        ensure!(
            self.schema_version().await? >= 5,
            "session-attributed messages require an upgraded memory view"
        );
        self.mutate(
            "session message",
            Mutation::Append {
                namespace: namespace.into(),
                session_id: Some(session_id.into()),
                role: message.role.clone(),
                content: encode_typed_message(message)?,
                format: Some(TYPED_FORMAT),
            },
        )
        .await
    }

    pub(crate) async fn schema_version(&self) -> Result<i32> {
        self.readable()?;
        if self.branch == "main" {
            Ok(migrations::CURRENT_VERSION)
        } else {
            tokio::time::timeout(QUERY_TIMEOUT, migrations::validate_historical(&self.pool))
                .await
                .context("historical memory reader validation deadline exceeded")?
        }
    }
    pub async fn history(&self, namespace: &str, limit: usize) -> Result<Vec<Message>> {
        identifier("namespace", namespace, 1024)?;
        // Candidate branches can intentionally retain an older schema after
        // main advances. Main was validated at open; only historical views
        // need the version-dispatched reader check before their query.
        let version = self.schema_version().await?;
        let limit = i64::try_from(limit).context("history limit exceeds integer range")?;
        let query = if version >= 3 {
            "SELECT sequence, role, content_format, content FROM (SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        } else {
            "SELECT sequence, role, content FROM (SELECT sequence, role, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        };
        let rows = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query(query)
                .bind(namespace.as_bytes())
                .bind(limit)
                .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("memory read deadline exceeded")??;
        rows.into_iter()
            .map(|row| {
                let sequence: i64 = row.try_get("sequence")?;
                let role = String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)?;
                let content: String = row.try_get("content")?;
                let format: String = if version >= 3 {
                    row.try_get("content_format")?
                } else {
                    TEXT_FORMAT.into()
                };
                decode_message(role, &format, &content).with_context(|| {
                    format!(
                        "invalid stored message sequence {sequence} on {}",
                        self.branch
                    )
                })
            })
            .collect()
    }

    /// Read the bounded newest suffix and the exact total number of rows in
    /// the same branch-pinned namespace.
    pub async fn history_window(&self, namespace: &str, limit: usize) -> Result<HistoryWindow> {
        identifier("namespace", namespace, 1024)?;
        let version = self.schema_version().await?;
        let limit = i64::try_from(limit).context("history limit exceeds integer range")?;
        let query = if version >= 3 {
            "SELECT sequence, role, content_format, content FROM (SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        } else {
            "SELECT sequence, role, content FROM (SELECT sequence, role, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        };
        // Keep count and suffix in one branch-pinned read transaction. It is a
        // short snapshot query only; provider work never holds it.
        let mut transaction = self.pool.begin().await?;
        let total_rows: i64 = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE namespace = ?")
                .bind(namespace.as_bytes())
                .fetch_one(&mut *transaction),
        )
        .await
        .context("memory history count deadline exceeded")??;
        let rows = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query(query)
                .bind(namespace.as_bytes())
                .bind(limit)
                .fetch_all(&mut *transaction),
        )
        .await
        .context("memory history window deadline exceeded")??;
        transaction.commit().await?;
        let messages = rows
            .into_iter()
            .map(|row| {
                let sequence: i64 = row.try_get("sequence")?;
                let role = String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)?;
                let content: String = row.try_get("content")?;
                let format: String = if version >= 3 {
                    row.try_get("content_format")?
                } else {
                    TEXT_FORMAT.into()
                };
                decode_message(role, &format, &content).with_context(|| {
                    format!(
                        "invalid stored message sequence {sequence} on {}",
                        self.branch
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(HistoryWindow {
            messages,
            total_rows: u64::try_from(total_rows).context("memory history count is negative")?,
        })
    }

    /// Read the bounded newest raw-history suffix attributed to one session.
    /// Namespace-only history remains available for explicit inspection and
    /// export, but runtime continuity uses this physical provenance boundary.
    pub async fn session_history_window(
        &self,
        namespace: &str,
        session_id: &str,
        limit: usize,
    ) -> Result<HistoryWindow> {
        validate_session_history_request(namespace, session_id, limit)?;
        ensure!(
            self.schema_version().await? >= 5,
            "session history requires an upgraded memory view"
        );
        let mut transaction = self.pool.begin().await?;
        let total_rows: i64 = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM messages WHERE namespace = ? AND session_id = ?",
            )
            .bind(namespace.as_bytes())
            .bind(session_id.as_bytes())
            .fetch_one(&mut *transaction),
        )
        .await
        .context("session history count deadline exceeded")??;
        let mut messages = tokio::time::timeout(QUERY_TIMEOUT, async {
            let mut source = sqlx::query("SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? AND session_id = ? ORDER BY sequence DESC LIMIT ?")
                .bind(namespace.as_bytes())
                .bind(session_id.as_bytes())
                .bind(i64::try_from(limit).context("session history limit exceeds integer range")?)
                .fetch(&mut *transaction);
            let mut messages = Vec::new();
            let mut budget = SessionSourceBudget::new(limit);
            while let Some(row) = source.try_next().await? {
                let sequenced = decode_session_source_row(&row, 0)?;
                let row_bytes = serialized_session_source_row_bytes(&sequenced)?;
                ensure!(
                    !messages.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                    "stored session history row exceeds the response byte bound"
                );
                if !budget.try_include(row_bytes)? {
                    break;
                }
                messages.push(sequenced.message);
            }
            messages.reverse();
            Ok::<_, anyhow::Error>(messages)
        })
        .await
        .context("session history window deadline exceeded")??;
        transaction.commit().await?;
        messages.shrink_to_fit();
        Ok(HistoryWindow {
            messages,
            total_rows: u64::try_from(total_rows).context("session history count is negative")?,
        })
    }

    /// Read the bounded newest sequenced suffix attributed to one session and
    /// strictly after one durable cursor on this pinned view.
    pub async fn session_history_window_after(
        &self,
        namespace: &str,
        session_id: &str,
        after_exclusive: i64,
        limit: usize,
    ) -> Result<SessionHistoryWindowAfter> {
        self.readable()?;
        validate_session_history_after_request(namespace, session_id, after_exclusive, limit)?;
        ensure!(
            self.schema_version().await? >= 5,
            "session cursor history requires an upgraded memory view"
        );
        let _guard = self.shared.write.lock().await;
        let mut transaction = self.pool.begin().await?;
        let captured_revision: String = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(&mut *transaction),
        )
        .await
        .context("session cursor history revision deadline exceeded")??;
        let total_rows: i64 = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM messages WHERE namespace = ? AND session_id = ? AND sequence > ?",
            )
            .bind(namespace.as_bytes())
            .bind(session_id.as_bytes())
            .bind(after_exclusive)
            .fetch_one(&mut *transaction),
        )
        .await
        .context("session cursor history count deadline exceeded")??;
        let mut rows = tokio::time::timeout(QUERY_TIMEOUT, async {
            let mut source = sqlx::query("SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? AND session_id = ? AND sequence > ? ORDER BY sequence DESC LIMIT ?")
                .bind(namespace.as_bytes())
                .bind(session_id.as_bytes())
                .bind(after_exclusive)
                .bind(i64::try_from(limit).context("session cursor history limit exceeds integer range")?)
                .fetch(&mut *transaction);
            let mut rows = Vec::new();
            let mut budget = SessionSourceBudget::new(limit);
            while let Some(row) = source.try_next().await? {
                let row = decode_session_source_row(&row, after_exclusive)?;
                let row_bytes = serialized_session_source_row_bytes(&row)?;
                ensure!(
                    !rows.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                    "stored session history row exceeds the response byte bound"
                );
                if !budget.try_include(row_bytes)? {
                    break;
                }
                rows.push(row);
            }
            rows.reverse();
            Ok::<_, anyhow::Error>(rows)
        })
        .await
        .context("session cursor history window deadline exceeded")??;
        transaction.commit().await?;
        rows.shrink_to_fit();
        Ok(SessionHistoryWindowAfter {
            namespace: namespace.into(),
            session_id: session_id.into(),
            view: self.branch.clone(),
            revision: captured_revision,
            after_exclusive,
            rows,
            total_rows: u64::try_from(total_rows)
                .context("session cursor history count is negative")?,
        })
    }

    /// Read one newest-first catalog page from an exact checked view. A
    /// continuation is valid only at the captured revision and names the last
    /// complete row retained in the previous response.
    pub async fn session_catalog_page(
        &self,
        lifecycle_state: Option<SessionLifecycleState>,
        cursor: Option<&SessionCatalogCursor>,
        expected_revision: Option<&str>,
        limit: usize,
    ) -> Result<SessionCatalogPage> {
        self.readable()?;
        ensure!(
            limit <= MAX_SESSION_SOURCE_ROWS,
            "session catalog limit cannot exceed {MAX_SESSION_SOURCE_ROWS}"
        );
        if let Some(cursor) = cursor {
            validate_session_catalog_cursor(cursor)?;
        }
        if let Some(revision) = expected_revision {
            validate_revision_identity("session catalog expected revision", revision)?;
            ensure!(cursor.is_some(), "catalog revision requires a continuation");
        }
        ensure!(
            self.schema_version().await? >= 7,
            "session catalog requires an upgraded memory view"
        );
        let _guard = self.shared.write.lock().await;
        let mut transaction = self.pool.begin().await?;
        let captured_revision: String = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(&mut *transaction),
        )
        .await
        .context("session catalog revision deadline exceeded")??;
        if let Some(expected) = expected_revision {
            ensure!(
                captured_revision == expected,
                "session catalog continuation revision changed"
            );
        }
        let state = lifecycle_state.map(session_lifecycle_sql);
        let total_rows: i64 = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM session_catalog WHERE ? IS NULL OR lifecycle_state = ?",
            )
            .bind(state)
            .bind(state)
            .fetch_one(&mut *transaction),
        )
        .await
        .context("session catalog count deadline exceeded")??;
        let cursor_order = cursor
            .map(|cursor| i64::try_from(cursor.updated_order))
            .transpose()
            .context("session catalog cursor order exceeds SQL range")?;
        let cursor_id = cursor.map(|cursor| cursor.session_id.as_bytes());
        let query_limit = i64::try_from(limit.saturating_add(1))
            .context("session catalog query limit exceeds SQL range")?;
        let mut source = sqlx::query(
            "SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE (? IS NULL OR lifecycle_state = ?) AND (? IS NULL OR updated_order < ? OR (updated_order = ? AND session_id > ?)) ORDER BY updated_order DESC, session_id ASC LIMIT ?",
        )
        .bind(state)
        .bind(state)
        .bind(cursor_order)
        .bind(cursor_order)
        .bind(cursor_order)
        .bind(cursor_id)
        .bind(query_limit)
        .fetch(&mut *transaction);
        let mut records = Vec::new();
        let mut budget = SessionSourceBudget::new(limit);
        let mut has_more = false;
        while let Some(row) = tokio::time::timeout(QUERY_TIMEOUT, source.try_next())
            .await
            .context("session catalog row deadline exceeded")??
        {
            let record = decode_session_catalog_row(&row)?;
            if lifecycle_state.is_some_and(|state| state != record.lifecycle_state) {
                bail!("session catalog query returned a different lifecycle state");
            }
            let row_bytes = serde_json::to_vec(&record)?.len();
            ensure!(
                !records.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                "stored session catalog row exceeds the response byte bound"
            );
            if !budget.try_include(row_bytes)? {
                has_more = true;
                break;
            }
            records.push(record);
        }
        drop(source);
        transaction.commit().await?;
        let next = has_more.then(|| {
            let last = records
                .last()
                .expect("a continuation requires one retained catalog row");
            SessionCatalogCursor {
                updated_order: last.updated_order,
                session_id: last.session_id.clone(),
            }
        });
        let page = SessionCatalogPage {
            lifecycle_state,
            view: self.branch.clone(),
            revision: captured_revision,
            records,
            total_rows: u64::try_from(total_rows).context("session catalog count is negative")?,
            next,
        };
        validate_session_catalog_page(&page)?;
        Ok(page)
    }

    /// Read the current exact catalog row for one session identity.
    pub async fn session_catalog_record(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCatalogRecord>> {
        self.readable()?;
        session_identity("session identity", session_id, 128)?;
        ensure!(
            self.schema_version().await? >= 7,
            "session catalog requires an upgraded memory view"
        );
        let row = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ?")
                .bind(session_id.as_bytes())
                .fetch_optional(self.pool.as_ref()),
        )
        .await
        .context("session catalog record deadline exceeded")??;
        row.map(|row| decode_session_catalog_row(&row)).transpose()
    }

    pub async fn create_session_catalog(
        &self,
        session_id: &str,
        mode: Mode,
        label: &str,
    ) -> Result<SessionLifecycleOutcome> {
        validate_session_lifecycle_input(session_id, None, Some(label))?;
        self.mutate_session_catalog(
            "create session",
            SessionLifecycleMutation::Create {
                session_id: session_id.into(),
                mode,
                label: label.into(),
            },
        )
        .await
    }

    pub async fn rename_session_catalog(
        &self,
        session_id: &str,
        expected_generation: u64,
        label: &str,
    ) -> Result<SessionLifecycleOutcome> {
        validate_session_lifecycle_input(session_id, Some(expected_generation), Some(label))?;
        self.mutate_session_catalog(
            "rename session",
            SessionLifecycleMutation::Rename {
                session_id: session_id.into(),
                expected_generation,
                label: label.into(),
            },
        )
        .await
    }

    pub async fn remove_session_catalog(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<SessionLifecycleOutcome> {
        validate_session_lifecycle_input(session_id, Some(expected_generation), None)?;
        self.mutate_session_catalog(
            "remove session",
            SessionLifecycleMutation::Remove {
                session_id: session_id.into(),
                expected_generation,
            },
        )
        .await
    }

    pub async fn restore_session_catalog(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<SessionLifecycleOutcome> {
        validate_session_lifecycle_input(session_id, Some(expected_generation), None)?;
        self.mutate_session_catalog(
            "restore session",
            SessionLifecycleMutation::Restore {
                session_id: session_id.into(),
                expected_generation,
            },
        )
        .await
    }

    pub async fn fork_session_catalog(
        &self,
        source_session_id: &str,
        expected_source_generation: u64,
        source_node_id: &str,
        child_session_id: &str,
        label: &str,
    ) -> Result<SessionLifecycleOutcome> {
        validate_session_fork_input(
            source_session_id,
            expected_source_generation,
            source_node_id,
            child_session_id,
            label,
        )?;
        self.mutate_session_catalog(
            "fork session",
            SessionLifecycleMutation::Fork {
                source_session_id: source_session_id.into(),
                expected_source_generation,
                source_node_id: source_node_id.into(),
                child_session_id: child_session_id.into(),
                label: label.into(),
                validated: None,
            },
        )
        .await
    }

    async fn mutate_session_catalog(
        &self,
        label: &str,
        mutation: SessionLifecycleMutation,
    ) -> Result<SessionLifecycleOutcome> {
        self.writable()?;
        ensure!(
            self.schema_version().await? >= 7,
            "session lifecycle requires an upgraded memory view"
        );
        let guard = self.shared.write.clone().lock_owned().await;
        self.resolve_uncertain().await?;
        if let Some(receipt) = &self.logical_receipt
            && operation_receipt_matches(&self.pool, receipt).await?
        {
            return load_session_lifecycle_outcome(
                &self.pool,
                &receipt.physical_id,
                mutation.session_id(),
            )
            .await;
        }
        let store = self.clone();
        let label = label.to_owned();
        tokio::spawn(async move {
            let _guard = guard;
            let logical = store.logical_receipt.clone();
            let operation = logical.as_ref().map_or_else(
                || Uuid::new_v4().to_string(),
                |receipt| receipt.physical_id.clone(),
            );
            let session_id = mutation.session_id().to_owned();
            let (mut connection, id) = owned_connection(&store.pool).await?;
            let mutation = match mutation {
                SessionLifecycleMutation::Fork {
                    source_session_id,
                    expected_source_generation,
                    source_node_id,
                    child_session_id,
                    label,
                    validated: None,
                } => {
                    let validated = tokio::time::timeout(
                        QUERY_TIMEOUT,
                        validate_session_fork_source(
                            &mut connection,
                            &source_session_id,
                            expected_source_generation,
                            &source_node_id,
                        ),
                    )
                    .await
                    .context("session fork predecessor traversal deadline exceeded")??;
                    SessionLifecycleMutation::Fork {
                        source_session_id,
                        expected_source_generation,
                        source_node_id,
                        child_session_id,
                        label,
                        validated: Some(Box::new(validated)),
                    }
                }
                mutation => mutation,
            };
            *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: store.pool.clone(),
                connection: id,
                receipt: Receipt::Operation(operation.clone()),
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                apply_session_lifecycle(
                    &mut connection,
                    &operation,
                    &label,
                    mutation,
                    logical.as_ref(),
                ),
            )
            .await;
            drop(connection);
            if let Ok(Ok(outcome)) = result {
                *store.shared.uncertain.lock().expect("uncertain lock") = None;
                return Ok(outcome);
            }
            if store.resolve_uncertain().await? == Some(true) {
                return load_session_lifecycle_outcome(&store.pool, &operation, &session_id).await;
            }
            result.context("session lifecycle write deadline exceeded")??;
            bail!("session lifecycle mutation did not produce its durable receipt")
        })
        .await
        .context("session lifecycle write worker failed")?
    }

    /// Read one newest-first page of the selected session's public transcript.
    /// The continuation is accepted only at the exact captured revision and
    /// after its next turn is proved reachable from the captured session head.
    pub async fn public_transcript_page(
        &self,
        session_id: &str,
        cursor: Option<&PublicTranscriptCursor>,
        limit: usize,
    ) -> Result<PublicTranscriptPage> {
        self.readable()?;
        session_identity("public transcript session", session_id, 128)?;
        ensure!(
            limit <= MAX_SESSION_SOURCE_ROWS,
            "public transcript limit cannot exceed {MAX_SESSION_SOURCE_ROWS}"
        );
        if let Some(cursor) = cursor {
            validate_public_transcript_cursor(cursor)?;
            ensure!(
                cursor.session_id == session_id,
                "public transcript continuation changed session"
            );
        }
        ensure!(
            self.schema_version().await? >= 7,
            "public transcript requires an upgraded memory view"
        );

        let _guard = self.shared.write.lock().await;
        let mut transaction = self.pool.begin().await?;
        let captured_revision: String = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(&mut *transaction),
        )
        .await
        .context("public transcript revision deadline exceeded")??;
        if let Some(cursor) = cursor {
            ensure!(
                cursor.revision == captured_revision,
                "public transcript continuation revision changed"
            );
        }

        let catalog_row = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ?")
                .bind(session_id.as_bytes())
                .fetch_optional(&mut *transaction),
        )
        .await
        .context("public transcript catalog deadline exceeded")??
        .context("public transcript session does not exist")?;
        let catalog = decode_session_catalog_row(&catalog_row)?;
        ensure!(
            catalog.session_id == session_id,
            "public transcript catalog returned a different session"
        );
        if let Some(cursor) = cursor {
            ensure!(
                cursor.head_node_id == catalog.head_node_id,
                "public transcript continuation head changed"
            );
        }

        let pending = if let Some(pending_node_id) = &catalog.pending_node_id {
            let pending = tokio::time::timeout(
                QUERY_TIMEOUT,
                load_public_turn(&mut transaction, pending_node_id),
            )
            .await
            .context("public transcript pending turn deadline exceeded")??
            .context("session catalog pending turn does not exist")?;
            ensure!(
                pending.origin_session_id == session_id
                    && pending.settlement == PublicTurnSettlement::Pending,
                "session catalog pending turn coordinates are invalid"
            );
            Some(pending)
        } else {
            None
        };

        let page_head = catalog.head_node_id.clone();
        let requested = cursor.map(|cursor| cursor.next.clone()).or_else(|| {
            page_head
                .clone()
                .map(|node_id| PublicTranscriptPosition::Turn { node_id })
                .or_else(|| {
                    catalog
                        .legacy_prefix
                        .as_ref()
                        .map(|prefix| PublicTranscriptPosition::Legacy {
                            sequence: prefix.through_sequence,
                        })
                })
        });
        if let Some(PublicTranscriptPosition::Legacy { sequence }) = &requested {
            let prefix = catalog
                .legacy_prefix
                .as_ref()
                .context("public transcript has no legacy prefix")?;
            ensure!(
                *sequence >= prefix.first_sequence && *sequence <= prefix.through_sequence,
                "public transcript legacy continuation is outside the proven prefix"
            );
        }

        let mut records = Vec::new();
        let mut budget = SessionSourceBudget::new(limit);
        let mut next = None;
        let mut turn_count = 0_u64;
        let mut found_requested_turn =
            !matches!(requested, Some(PublicTranscriptPosition::Turn { .. }));
        let mut current = page_head.clone();
        tokio::time::timeout(QUERY_TIMEOUT, async {
            while let Some(node_id) = current {
                let record = load_public_turn(&mut transaction, &node_id)
                    .await?
                    .context("public transcript predecessor turn does not exist")?;
                ensure!(
                    record.node_id == node_id
                        && record.settlement != PublicTurnSettlement::Pending,
                    "public transcript chain contains an invalid settled turn"
                );
                turn_count = turn_count
                    .checked_add(1)
                    .context("public transcript turn count overflowed")?;
                let is_requested = matches!(
                    &requested,
                    Some(PublicTranscriptPosition::Turn { node_id: requested }) if requested == &node_id
                );
                if is_requested {
                    found_requested_turn = true;
                }
                if !matches!(requested, Some(PublicTranscriptPosition::Legacy { .. }))
                    && found_requested_turn
                    && next.is_none()
                {
                    let entry = PublicTranscriptEntry::Turn {
                        record: record.clone(),
                    };
                    let row_bytes = serde_json::to_vec(&entry)?.len();
                    ensure!(
                        !records.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                        "stored public turn exceeds the response byte bound"
                    );
                    if budget.try_include(row_bytes)? {
                        records.push(entry);
                    } else {
                        next = Some(PublicTranscriptPosition::Turn {
                            node_id: node_id.clone(),
                        });
                    }
                }
                current = record.predecessor_node_id;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("public transcript predecessor traversal deadline exceeded")??;
        ensure!(
            found_requested_turn,
            "public transcript continuation is not reachable from the captured head"
        );

        if next.is_none()
            && let Some(prefix) = &catalog.legacy_prefix
        {
            let legacy_start = match &requested {
                Some(PublicTranscriptPosition::Legacy { sequence }) => *sequence,
                _ => prefix.through_sequence,
            };
            let mut source = sqlx::query("SELECT sequence, session_id, role, content_format, content FROM messages WHERE namespace = ? AND sequence >= ? AND sequence <= ? ORDER BY sequence DESC")
                .bind(prefix.namespace.as_bytes())
                .bind(prefix.first_sequence)
                .bind(legacy_start)
                .fetch(&mut *transaction);
            while let Some(row) = tokio::time::timeout(QUERY_TIMEOUT, source.try_next())
                .await
                .context("legacy public transcript row deadline exceeded")??
            {
                let sequence: i64 = row.try_get("sequence")?;
                let stored_session = row
                    .try_get::<Option<Vec<u8>>, _>("session_id")?
                    .map(String::from_utf8)
                    .transpose()
                    .context("legacy transcript session identity is not UTF-8")?;
                ensure!(
                    stored_session
                        .as_deref()
                        .is_none_or(|stored| stored == prefix.source_session_id),
                    "legacy transcript row belongs to a different session"
                );
                let message = decode_message(
                    String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)
                        .context("legacy transcript role is not UTF-8")?,
                    &row.try_get::<String, _>("content_format")?,
                    &row.try_get::<String, _>("content")?,
                )?;
                let entry = PublicTranscriptEntry::Legacy { sequence, message };
                let row_bytes = serde_json::to_vec(&entry)?.len();
                ensure!(
                    !records.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                    "stored legacy transcript row exceeds the response byte bound"
                );
                if budget.try_include(row_bytes)? {
                    records.push(entry);
                } else {
                    next = Some(PublicTranscriptPosition::Legacy { sequence });
                    break;
                }
            }
            drop(source);
        }

        let total_rows = turn_count
            .checked_add(
                catalog
                    .legacy_prefix
                    .as_ref()
                    .map_or(0, |prefix| prefix.row_count),
            )
            .context("public transcript total count overflowed")?;
        transaction.commit().await?;
        let mut page = PublicTranscriptPage {
            session_id: session_id.to_owned(),
            view: self.branch.clone(),
            revision: captured_revision,
            head_node_id: page_head,
            pending,
            records,
            total_rows,
            next: next.map(|next| PublicTranscriptCursor {
                session_id: session_id.to_owned(),
                revision: String::new(),
                head_node_id: None,
                next,
            }),
        };
        if let Some(next) = &mut page.next {
            next.revision = page.revision.clone();
            next.head_node_id = page.head_node_id.clone();
        }
        while serde_json::to_vec(&page)?.len() > MAX_SESSION_SOURCE_BYTES {
            let removed = page
                .records
                .pop()
                .context("public transcript metadata exceeds the response byte bound")?;
            page.next = Some(PublicTranscriptCursor {
                session_id: page.session_id.clone(),
                revision: page.revision.clone(),
                head_node_id: page.head_node_id.clone(),
                next: public_transcript_entry_position(&removed),
            });
        }
        validate_public_transcript_page(&page)?;
        Ok(page)
    }

    /// Capture the next bounded session-private source page at this view's
    /// exact revision. Namespace admission remains the caller's policy duty.
    pub async fn session_source_snapshot(
        &self,
        actor_namespace: &str,
        session_id: &str,
        source_namespace: &str,
        after_exclusive: i64,
        limit: usize,
    ) -> Result<SessionSourceSnapshot> {
        self.readable()?;
        validate_session_source_request(
            actor_namespace,
            session_id,
            source_namespace,
            after_exclusive,
            limit,
        )?;
        ensure!(
            self.schema_version().await? >= 5,
            "session source snapshots require an upgraded memory view"
        );
        let _guard = self.shared.write.lock().await;
        let captured_revision = revision(&self.pool).await?;
        let rows = tokio::time::timeout(QUERY_TIMEOUT, async {
            let mut source = sqlx::query("SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? AND session_id = ? AND sequence > ? ORDER BY sequence LIMIT ?")
                .bind(source_namespace.as_bytes())
                .bind(session_id.as_bytes())
                .bind(after_exclusive)
                .bind(i64::try_from(limit).context("source snapshot limit exceeds integer range")?)
                .fetch(self.pool.as_ref());
            let mut rows = Vec::new();
            let mut budget = SessionSourceBudget::new(limit);
            while let Some(row) = source.try_next().await? {
                let row = decode_session_source_row(&row, after_exclusive)?;
                let row_bytes = serialized_session_source_row_bytes(&row)?;
                ensure!(
                    !rows.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                    "stored session source row exceeds the snapshot byte bound"
                );
                if !budget.try_include(row_bytes)? {
                    break;
                }
                rows.push(row);
            }
            Ok::<_, anyhow::Error>(rows)
        })
        .await
        .context("session source snapshot deadline exceeded")??;
        let through_inclusive = rows.last().map(|row| row.sequence);
        Ok(SessionSourceSnapshot {
            actor_namespace: actor_namespace.into(),
            session_id: session_id.into(),
            source_namespace: source_namespace.into(),
            view: self.branch.clone(),
            revision: captured_revision,
            after_exclusive,
            through_inclusive,
            rows,
        })
    }

    /// Atomically publish a strict context summary and advance its exact
    /// actor/session/source cursor if the captured source is still current.
    pub async fn checkpoint_context_summary(
        &self,
        checkpoint: &ContextSummaryCheckpoint,
    ) -> Result<()> {
        validate_context_summary_checkpoint(checkpoint)?;
        let record = &checkpoint.record;
        ensure!(
            self.schema_version().await? >= 6,
            "operation-aware context summaries require an upgraded memory view"
        );
        ensure!(record.source_view == self.branch, ContextSummaryStale);
        let summary_id = context_summary_id(record)?;
        let record_format = context_summary_format(record)?;
        let private_reasoning = if checkpoint.private_reasoning.is_empty() {
            Vec::new()
        } else {
            encode_reasoning_summaries(&checkpoint.private_reasoning)?
        };
        self.mutate(
            "context summary checkpoint",
            Mutation::ContextSummary {
                record: Box::new(record.clone()),
                summary_id,
                record_format,
                private_reasoning,
            },
        )
        .await
    }

    /// Read the current typed compaction cursor for an authorized source.
    pub async fn context_summary_cursor(
        &self,
        actor_namespace: &str,
        session_id: &str,
        source_namespace: &str,
    ) -> Result<Option<ContextSummaryCursor>> {
        self.readable()?;
        identifier("actor namespace", actor_namespace, 1024)?;
        identifier("session identity", session_id, 128)?;
        identifier("source namespace", source_namespace, 1024)?;
        let schema_version = self.schema_version().await?;
        ensure!(
            schema_version >= 5,
            "context summaries require an upgraded memory view"
        );
        let row = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query("SELECT through_sequence, summary_id, source_view, source_revision FROM context_summary_cursors WHERE actor_namespace = ? AND session_id = ? AND source_namespace = ?")
                .bind(actor_namespace.as_bytes())
                .bind(session_id.as_bytes())
                .bind(source_namespace.as_bytes())
                .fetch_optional(self.pool.as_ref()),
        )
        .await
        .context("context summary cursor deadline exceeded")??;
        row.map(|row| {
            let cursor = ContextSummaryCursor {
                actor_namespace: actor_namespace.into(),
                session_id: session_id.into(),
                source_namespace: source_namespace.into(),
                through_sequence: row.try_get("through_sequence")?,
                summary_id: row.try_get("summary_id")?,
                source_view: row.try_get("source_view")?,
                source_revision: row.try_get("source_revision")?,
            };
            validate_context_summary_cursor(&cursor)?;
            Ok(cursor)
        })
        .transpose()
    }

    /// Read summaries selected by one checked actor/summary namespace. Passing
    /// a session selects its private continuity; omitting it is reserved for a
    /// caller whose memory policy already admitted the shared namespace.
    pub async fn context_summary_window(
        &self,
        actor_namespace: &str,
        summary_namespace: &str,
        session_id: Option<&str>,
        source_namespace: Option<&str>,
        limit: usize,
    ) -> Result<ContextSummaryWindow> {
        self.readable()?;
        validate_context_summary_window_request(
            actor_namespace,
            summary_namespace,
            session_id,
            source_namespace,
            limit,
        )?;
        let schema_version = self.schema_version().await?;
        ensure!(
            schema_version >= 5,
            "context summaries require an upgraded memory view"
        );
        let _guard = self.shared.write.lock().await;
        let captured_revision = revision(&self.pool).await?;
        let mut transaction = self.pool.begin().await?;
        let total_rows: i64 = tokio::time::timeout(QUERY_TIMEOUT, async {
            match (session_id, source_namespace) {
                (Some(session_id), Some(source_namespace)) => sqlx::query_scalar(
                    "SELECT COUNT(*) FROM context_summary_cursors c JOIN context_summaries s ON s.summary_id = c.summary_id AND s.actor_namespace = c.actor_namespace AND s.session_id = c.session_id AND s.source_namespace = c.source_namespace WHERE s.actor_namespace = ? AND s.summary_namespace = ? AND s.session_id = ? AND s.source_namespace = ?",
                )
                .bind(actor_namespace.as_bytes())
                .bind(summary_namespace.as_bytes())
                .bind(session_id.as_bytes())
                .bind(source_namespace.as_bytes())
                .fetch_one(&mut *transaction)
                .await,
                (Some(session_id), None) => sqlx::query_scalar(
                    "SELECT COUNT(*) FROM context_summary_cursors c JOIN context_summaries s ON s.summary_id = c.summary_id AND s.actor_namespace = c.actor_namespace AND s.session_id = c.session_id AND s.source_namespace = c.source_namespace WHERE s.actor_namespace = ? AND s.summary_namespace = ? AND s.session_id = ?",
                )
                .bind(actor_namespace.as_bytes())
                .bind(summary_namespace.as_bytes())
                .bind(session_id.as_bytes())
                .fetch_one(&mut *transaction)
                .await,
                (None, Some(source_namespace)) => sqlx::query_scalar(
                    "SELECT COUNT(*) FROM context_summary_cursors c JOIN context_summaries s ON s.summary_id = c.summary_id AND s.actor_namespace = c.actor_namespace AND s.session_id = c.session_id AND s.source_namespace = c.source_namespace WHERE s.actor_namespace = ? AND s.summary_namespace = ? AND s.source_namespace = ?",
                )
                .bind(actor_namespace.as_bytes())
                .bind(summary_namespace.as_bytes())
                .bind(source_namespace.as_bytes())
                .fetch_one(&mut *transaction)
                .await,
                (None, None) => sqlx::query_scalar(
                    "SELECT COUNT(*) FROM context_summary_cursors c JOIN context_summaries s ON s.summary_id = c.summary_id AND s.actor_namespace = c.actor_namespace AND s.session_id = c.session_id AND s.source_namespace = c.source_namespace WHERE s.actor_namespace = ? AND s.summary_namespace = ?",
                )
                .bind(actor_namespace.as_bytes())
                .bind(summary_namespace.as_bytes())
                .fetch_one(&mut *transaction)
                .await,
            }
        })
        .await
        .context("context summary count deadline exceeded")??;
        let mut records = tokio::time::timeout(QUERY_TIMEOUT, async {
            let select = if schema_version >= 6 {
                "SELECT s.summary_id, s.actor_namespace, s.session_id, s.source_namespace, s.summary_namespace, s.source_view, s.source_revision, s.after_sequence, s.through_sequence, s.turn_id, s.operation_id, s.producer_actor_id, s.invocation_id, s.record_format, s.summary FROM context_summary_cursors c JOIN context_summaries s ON s.summary_id = c.summary_id AND s.actor_namespace = c.actor_namespace AND s.session_id = c.session_id AND s.source_namespace = c.source_namespace"
            } else {
                "SELECT s.summary_id, s.actor_namespace, s.session_id, s.source_namespace, s.summary_namespace, s.source_view, s.source_revision, s.after_sequence, s.through_sequence, s.turn_id, NULL AS operation_id, NULL AS producer_actor_id, s.invocation_id, s.record_format, s.summary FROM context_summary_cursors c JOIN context_summaries s ON s.summary_id = c.summary_id AND s.actor_namespace = c.actor_namespace AND s.session_id = c.session_id AND s.source_namespace = c.source_namespace"
            };
            let query_text = match (session_id, source_namespace) {
                (Some(_), Some(_)) => format!(
                    "{select} WHERE s.actor_namespace = ? AND s.summary_namespace = ? AND s.session_id = ? AND s.source_namespace = ? ORDER BY s.through_sequence DESC, s.summary_id DESC LIMIT ?"
                ),
                (Some(_), None) => format!(
                    "{select} WHERE s.actor_namespace = ? AND s.summary_namespace = ? AND s.session_id = ? ORDER BY s.through_sequence DESC, s.summary_id DESC LIMIT ?"
                ),
                (None, Some(_)) => format!(
                    "{select} WHERE s.actor_namespace = ? AND s.summary_namespace = ? AND s.source_namespace = ? ORDER BY s.through_sequence DESC, s.summary_id DESC LIMIT ?"
                ),
                (None, None) => format!(
                    "{select} WHERE s.actor_namespace = ? AND s.summary_namespace = ? ORDER BY s.through_sequence DESC, s.summary_id DESC LIMIT ?"
                ),
            };
            let query = match (session_id, source_namespace) {
                (Some(session_id), Some(source_namespace)) => {
                    sqlx::query(sqlx::AssertSqlSafe(query_text))
                    .bind(actor_namespace.as_bytes())
                    .bind(summary_namespace.as_bytes())
                    .bind(session_id.as_bytes())
                    .bind(source_namespace.as_bytes())
                    .bind(i64::try_from(limit).context("context summary limit exceeds integer range")?)
                },
                (Some(session_id), None) => sqlx::query(sqlx::AssertSqlSafe(query_text))
                    .bind(actor_namespace.as_bytes())
                    .bind(summary_namespace.as_bytes())
                    .bind(session_id.as_bytes())
                    .bind(i64::try_from(limit).context("context summary limit exceeds integer range")?),
                (None, Some(source_namespace)) => sqlx::query(sqlx::AssertSqlSafe(query_text))
                    .bind(actor_namespace.as_bytes())
                    .bind(summary_namespace.as_bytes())
                    .bind(source_namespace.as_bytes())
                    .bind(i64::try_from(limit).context("context summary limit exceeds integer range")?),
                (None, None) => sqlx::query(sqlx::AssertSqlSafe(query_text))
                    .bind(actor_namespace.as_bytes())
                    .bind(summary_namespace.as_bytes())
                    .bind(i64::try_from(limit).context("context summary limit exceeds integer range")?),
            };
            let mut source = query.fetch(&mut *transaction);
            let mut records = Vec::new();
            let mut budget = SessionSourceBudget::new(limit);
            while let Some(row) = source.try_next().await? {
                let record = decode_context_summary_item(&row)?;
                ensure!(
                    record.record.actor_namespace == actor_namespace
                        && record.record.summary_namespace == summary_namespace
                        && session_id
                            .is_none_or(|session| record.record.session_id == session)
                        && source_namespace
                            .is_none_or(|source| record.record.source_namespace == source),
                    "context summary projection escaped its requested boundary"
                );
                let row_bytes = serde_json::to_vec(&record)?.len();
                ensure!(
                    !records.is_empty() || row_bytes <= MAX_SESSION_SOURCE_BYTES,
                    "stored context summary exceeds the response byte bound"
                );
                if !budget.try_include(row_bytes)? {
                    break;
                }
                records.push(record);
            }
            records.reverse();
            Ok::<_, anyhow::Error>(records)
        })
        .await
        .context("context summary window deadline exceeded")??;
        transaction.commit().await?;
        records.shrink_to_fit();
        Ok(ContextSummaryWindow {
            actor_namespace: actor_namespace.into(),
            summary_namespace: summary_namespace.into(),
            session_id: session_id.map(str::to_owned),
            source_namespace: source_namespace.map(str::to_owned),
            view: self.branch.clone(),
            revision: captured_revision,
            records,
            total_rows: u64::try_from(total_rows).context("context summary count is negative")?,
        })
    }
    /// Read durable rows with their stable sequence for a caller that already
    /// owns namespace selection. This deliberately preserves every stored role.
    pub async fn notes(&self, namespace: &str, limit: usize) -> Result<Vec<StoredNote>> {
        identifier("namespace", namespace, 1024)?;
        let version = self.schema_version().await?;
        let limit = i64::try_from(limit).context("notes limit exceeds integer range")?;
        let query = if version >= 3 {
            "SELECT sequence, role, content_format, content FROM (SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        } else {
            "SELECT sequence, role, content FROM (SELECT sequence, role, content FROM messages WHERE namespace = ? ORDER BY sequence DESC LIMIT ?) AS recent ORDER BY sequence"
        };
        let rows = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query(query)
                .bind(namespace.as_bytes())
                .bind(limit)
                .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("memory read deadline exceeded")??;
        rows.into_iter()
            .map(|row| {
                let sequence: i64 = row.try_get("sequence")?;
                let role = String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)?;
                let content: String = row.try_get("content")?;
                let format: String = if version >= 3 {
                    row.try_get("content_format")?
                } else {
                    TEXT_FORMAT.into()
                };
                let message = decode_message(role, &format, &content).with_context(|| {
                    format!("invalid stored note sequence {sequence} on {}", self.branch)
                })?;
                let content = message
                    .plain_text()
                    .context("stored note contains structured content")?
                    .to_owned();
                Ok(StoredNote {
                    sequence,
                    role: message.role,
                    content,
                })
            })
            .collect()
    }
    /// Remove exactly one current row from an already-authorized notes namespace.
    /// Historical Dolt revisions remain intact.
    pub async fn forget_note(&self, namespace: &str, sequence: i64) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        ensure!(
            namespace.ends_with("/notes"),
            "selected deletion requires a notes namespace"
        );
        self.mutate(
            "forget note",
            Mutation::ForgetNote {
                namespace: namespace.into(),
                sequence,
            },
        )
        .await
    }
    pub async fn put(&self, key: &str, value: &Value) -> Result<()> {
        self.put_many(&[(key.into(), value.clone())]).await
    }
    pub async fn put_many(&self, values: &[(String, Value)]) -> Result<()> {
        let encoded = encode_state(values)?;
        if encoded.is_empty() {
            return Ok(());
        }
        self.mutate("state", Mutation::State(encoded)).await
    }

    pub async fn put_reasoning_summaries(&self, records: &[ReasoningSummaryRecord]) -> Result<()> {
        let encoded = encode_reasoning_summaries(records)?;
        self.mutate(
            "private reasoning summaries",
            Mutation::PrivateReasoningSummaries(encoded),
        )
        .await
    }

    /// Append messages to one namespace and update state in the same durable
    /// receipt-bearing transaction. This is the narrow turn-checkpoint seam;
    /// callers do not receive general SQL or cross-namespace authority.
    pub async fn checkpoint(
        &self,
        namespace: &str,
        messages: &[Message],
        values: &[(String, Value)],
    ) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        ensure!(
            self.schema_version().await? >= 3,
            "typed messages require an upgraded memory view"
        );
        let mut encoded_messages = Vec::with_capacity(messages.len());
        for message in messages {
            identifier("role", &message.role, 128)?;
            encoded_messages.push((message.role.clone(), encode_typed_message(message)?));
        }
        let encoded_state = encode_state(values)?;
        ensure!(
            !encoded_messages.is_empty() || !encoded_state.is_empty(),
            "memory checkpoint must contain a message or state value"
        );
        self.mutate(
            "checkpoint",
            Mutation::Checkpoint {
                namespace: namespace.into(),
                session_id: None,
                messages: encoded_messages,
                values: encoded_state,
                public_turn: None,
                mode: None,
            },
        )
        .await
    }

    /// Atomically append session-attributed history and update state for one
    /// turn checkpoint while retaining the global message sequence allocator.
    pub async fn checkpoint_session(
        &self,
        namespace: &str,
        session_id: &str,
        messages: &[Message],
        values: &[(String, Value)],
    ) -> Result<()> {
        self.checkpoint_session_inner(namespace, session_id, messages, values, None, None)
            .await
    }

    /// Apply one public-turn admission or settlement inside the same receipt
    /// and SQL transaction as its raw transcript and runtime state checkpoint.
    pub async fn checkpoint_session_turn(
        &self,
        namespace: &str,
        session_id: &str,
        messages: &[Message],
        values: &[(String, Value)],
        public_turn: &SessionTurnCheckpoint,
    ) -> Result<()> {
        let suffix = format!("/transcript/{session_id}");
        let scope = namespace
            .strip_suffix(&suffix)
            .context("public turn transcript namespace does not match its session")?;
        self.ensure_project_scope(scope)?;
        self.checkpoint_session_inner(
            namespace,
            session_id,
            messages,
            values,
            Some(public_turn),
            None,
        )
        .await
    }

    pub async fn checkpoint_session_mode(
        &self,
        namespace: &str,
        session_id: &str,
        values: &[(String, Value)],
        mode: &SessionModeCheckpoint,
    ) -> Result<()> {
        let suffix = format!("/transcript/{session_id}");
        let scope = namespace
            .strip_suffix(&suffix)
            .context("mode checkpoint transcript namespace does not match its session")?;
        self.ensure_project_scope(scope)?;
        self.checkpoint_session_inner(namespace, session_id, &[], values, None, Some(mode))
            .await
    }

    async fn checkpoint_session_inner(
        &self,
        namespace: &str,
        session_id: &str,
        messages: &[Message],
        values: &[(String, Value)],
        public_turn: Option<&SessionTurnCheckpoint>,
        mode: Option<&SessionModeCheckpoint>,
    ) -> Result<()> {
        validate_session_checkpoint(namespace, session_id, messages, values)?;
        if let Some(public_turn) = public_turn {
            validate_session_turn_checkpoint(session_id, messages, public_turn)?;
        }
        if let Some(mode) = mode {
            validate_session_mode_checkpoint(namespace, session_id, messages, values, mode)?;
        }
        ensure!(
            public_turn.is_none() || mode.is_none(),
            "session checkpoint cannot change mode and public turn together"
        );
        ensure!(
            self.schema_version().await?
                >= if public_turn.is_some() || mode.is_some() {
                    7
                } else {
                    5
                },
            if public_turn.is_some() || mode.is_some() {
                "session catalog checkpoints require an upgraded memory view"
            } else {
                "session-attributed checkpoints require an upgraded memory view"
            }
        );
        let mut encoded_messages = Vec::with_capacity(messages.len());
        for message in messages {
            identifier("role", &message.role, 128)?;
            encoded_messages.push((message.role.clone(), encode_typed_message(message)?));
        }
        let encoded_state = encode_state(values)?;
        let public_turn = public_turn
            .map(|checkpoint| encode_session_turn_checkpoint(checkpoint, messages).map(Box::new))
            .transpose()?;
        ensure!(
            !encoded_messages.is_empty() || !encoded_state.is_empty(),
            "memory checkpoint must contain a message or state value"
        );
        self.mutate(
            "session checkpoint",
            Mutation::Checkpoint {
                namespace: namespace.into(),
                session_id: Some(session_id.into()),
                messages: encoded_messages,
                values: encoded_state,
                public_turn,
                mode: mode.cloned(),
            },
        )
        .await
    }

    pub async fn get(&self, key: &str) -> Result<Option<Value>> {
        self.readable()?;
        identifier("state key", key, 1024)?;
        let value: Option<String> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(key.as_bytes())
                .fetch_optional(self.pool.as_ref()),
        )
        .await
        .context("memory read deadline exceeded")??;
        value
            .map(|value| serde_json::from_str(&value).context("stored state contains invalid JSON"))
            .transpose()
    }
    pub async fn clear(&self, namespace: &str) -> Result<()> {
        identifier("namespace", namespace, 1024)?;
        self.mutate("clear conversation", Mutation::Clear(namespace.into()))
            .await
    }
    async fn mutate(&self, label: &str, mutation: Mutation) -> Result<()> {
        self.writable()?;
        let guard = self.shared.write.clone().lock_owned().await;
        self.resolve_uncertain().await?;
        if let Some(receipt) = &self.logical_receipt {
            ensure!(
                self.schema_version().await? == migrations::CURRENT_VERSION,
                "logical mutation receipts require upgraded writable memory"
            );
            if operation_receipt_matches(&self.pool, receipt).await? {
                return Ok(());
            }
        }
        let store = self.clone();
        let label = label.to_owned();
        tokio::spawn(async move {
            let _guard = guard;
            let logical = store.logical_receipt.clone();
            let operation = logical.as_ref().map_or_else(
                || Uuid::new_v4().to_string(),
                |receipt| receipt.physical_id.clone(),
            );
            let (mut connection, id) = owned_connection(&store.pool).await?;
            *store.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: store.pool.clone(),
                connection: id,
                receipt: Receipt::Operation(operation.clone()),
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                apply(
                    &mut connection,
                    &operation,
                    &label,
                    mutation,
                    logical.as_ref(),
                ),
            )
            .await;
            drop(connection);
            if matches!(result, Ok(Ok(()))) {
                *store.shared.uncertain.lock().expect("uncertain lock") = None;
                return Ok(());
            }
            if store.resolve_uncertain().await? == Some(true) {
                return Ok(());
            }
            result.context("memory write deadline exceeded")??;
            bail!("memory mutation did not produce its durable receipt")
        })
        .await
        .context("memory write worker failed")?
    }
    async fn resolve_uncertain(&self) -> Result<Option<bool>> {
        let pending = self
            .shared
            .uncertain
            .lock()
            .expect("uncertain lock")
            .clone();
        if let Some(pending) = pending {
            await_session_end(&pending.pool, pending.connection, QUERY_TIMEOUT)
                .await
                .context("memory outcome is uncertain; original SQL session has not finished")?;
            let committed = match pending.receipt {
                Receipt::Operation(operation) => {
                    operation_exists(&pending.pool, &operation).await?
                }
                Receipt::CandidateCreation { branch, base } => {
                    let names = CandidateNames::from_open(&branch)?;
                    let heads = candidate_heads(&pending.pool, &names).await?;
                    ensure!(
                        !heads.contains_key(&names.promoting)
                            && !heads.contains_key(&names.abandoned),
                        "candidate creation gained a status ref before settling"
                    );
                    if let Some(head) = heads.get(&branch) {
                        ensure!(head == &base, "candidate creation changed its base head");
                        true
                    } else {
                        false
                    }
                }
                Receipt::Promotion { base, target } => {
                    let observed = revision(&pending.pool).await?;
                    ensure!(
                        observed == target || observed == base,
                        "cannot reconcile promotion: live history diverged from both base and target"
                    );
                    observed == target
                }
                Receipt::CandidateTransition {
                    source,
                    status,
                    expected,
                } => {
                    let names = CandidateNames::from_status(&status)?;
                    let heads = candidate_heads(&pending.pool, &names).await?;
                    for head in heads.values() {
                        ensure!(
                            head == &expected,
                            "candidate transition retained divergent refs"
                        );
                    }
                    if heads.contains_key(&status) {
                        true
                    } else {
                        ensure!(
                            heads.contains_key(&source),
                            "candidate transition lost both source and status refs"
                        );
                        false
                    }
                }
                Receipt::CandidateDeletion { branch, expected } => {
                    let names = CandidateNames::from_status_or_open(&branch)?;
                    let heads = candidate_heads(&pending.pool, &names).await?;
                    if let Some(head) = heads.get(&branch) {
                        ensure!(
                            head == &expected,
                            "candidate deletion changed the ref unexpectedly"
                        );
                        false
                    } else {
                        true
                    }
                }
                Receipt::CandidateExclusion { branch, expected } => {
                    let names = CandidateNames::from_status_or_open(&branch)?;
                    let heads = candidate_heads(&pending.pool, &names).await?;
                    ensure!(
                        heads.get(&branch).is_some_and(|head| head == &expected),
                        "candidate session exclusion changed the resolved ref"
                    );
                    true
                }
            };
            *self.shared.uncertain.lock().expect("uncertain lock") = None;
            return Ok(Some(committed));
        }
        Ok(None)
    }
    /// Resolve one pending durable operation, returning `None` when there was
    /// none, `Some(true)` when it committed, and `Some(false)` otherwise.
    pub async fn reconcile(&self) -> Result<Option<bool>> {
        self.readable()?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await
    }
    pub async fn begin_candidate(&self, label: &str) -> Result<Candidate> {
        self.begin_candidate_with_id(label, Uuid::new_v4()).await
    }

    pub(crate) async fn begin_candidate_with_id(&self, label: &str, id: Uuid) -> Result<Candidate> {
        self.writable()?;
        identifier("candidate label", label, 128)?;
        let guard = self.shared.write.clone().lock_owned().await;
        self.resolve_uncertain().await?;
        let branch = self.candidate_branch_for_id(id);
        let names = CandidateNames::from_open(&branch)?;
        let heads = candidate_heads(&self.pool, &names).await?;
        ensure!(
            heads.is_empty(),
            "candidate creation identity already has a durable ref; inspect its exact outcome"
        );
        let base = self.revision().await?;
        let worker = self.clone();
        let created_branch = branch.clone();
        let created_base = base.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let (mut connection, connection_id) = owned_connection(&worker.pool).await?;
            *worker.shared.uncertain.lock().expect("uncertain lock") = Some(Pending {
                pool: worker.pool.clone(),
                connection: connection_id,
                receipt: Receipt::CandidateCreation {
                    branch: created_branch.clone(),
                    base: created_base.clone(),
                },
            });
            let result = tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query("CALL DOLT_BRANCH(?, ?)")
                    .bind(&created_branch)
                    .bind(&created_base)
                    .fetch_all(&mut connection),
            )
            .await;
            drop(connection);
            if worker.resolve_uncertain().await? == Some(true) {
                return Ok::<_, anyhow::Error>(());
            }
            result.context("candidate creation deadline exceeded")??;
            bail!("candidate creation did not retain its exact ref")
        })
        .await
        .context("candidate creation worker failed")??;
        self.candidate_from_branch(branch, base).await
    }

    /// Read only the caller-derived branch identity. This never invokes branch
    /// creation, even when the original creation reply was lost.
    pub(crate) async fn candidate_for_id(&self, id: Uuid) -> Result<CandidateLookup> {
        self.readable()?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let branch = self.candidate_branch_for_id(id);
        let names = CandidateNames::from_open(&branch)?;
        let heads = candidate_heads(&self.pool, &names).await?;
        if heads.contains_key(&names.promoting) || heads.contains_key(&names.abandoned) {
            return Ok(CandidateLookup::Resolved);
        }
        if !heads.contains_key(&branch) {
            return Ok(CandidateLookup::Missing);
        }
        // For an unresolved open ref, main remains an append-only history.
        // Its exact common ancestor is the original candidate creation base,
        // including after both heads advance. A resolved ref is rejected above.
        let base = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar::<_, String>("SELECT DOLT_MERGE_BASE(?, ?)")
                .bind(&branch)
                .bind("main")
                .fetch_one(self.pool.as_ref()),
        )
        .await
        .context("candidate creation-base lookup deadline exceeded")??;
        Ok(CandidateLookup::Open(Box::new(
            self.candidate_from_branch(branch, base).await?,
        )))
    }

    /// Page canonical candidate identities from the owner's actual ref table.
    /// A creation UUID cannot be reconstructed from its store-bound branch
    /// hash, so recovery after process exit uses the exact retained branch.
    pub(crate) async fn candidate_inventory(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<CandidateInventoryPage> {
        self.readable()?;
        ensure!(
            self.branch == "main",
            "candidate inventory requires the live view"
        );
        ensure!(
            (1..=16).contains(&limit),
            "candidate inventory limit must be 1 through 16"
        );
        // The cursor is an opaque raw suffix, not an asserted candidate ID.
        // A foreign malformed prefix match can occupy a SQL page boundary;
        // filtering only the returned rows must not strand later valid refs.
        let after = after.unwrap_or("");
        ensure!(
            after.len() <= 1024 && !after.contains('\0'),
            "invalid candidate inventory cursor"
        );
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let suffixes: Vec<String> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar(
                "SELECT suffix FROM (\
                    SELECT SUBSTRING(name, ?) AS suffix FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ? \
                    UNION SELECT SUBSTRING(name, ?) AS suffix FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ? \
                    UNION SELECT SUBSTRING(name, ?) AS suffix FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ?\
                ) AS candidate_refs WHERE BINARY suffix > BINARY ? ORDER BY BINARY suffix LIMIT ?",
            )
            .bind((CANDIDATE_PREFIX.len() + 1) as i64)
            .bind(CANDIDATE_PREFIX.len() as i64)
            .bind(CANDIDATE_PREFIX)
            .bind((PROMOTING_PREFIX.len() + 1) as i64)
            .bind(PROMOTING_PREFIX.len() as i64)
            .bind(PROMOTING_PREFIX)
            .bind((ABANDONED_PREFIX.len() + 1) as i64)
            .bind(ABANDONED_PREFIX.len() as i64)
            .bind(ABANDONED_PREFIX)
            .bind(after)
            .bind((limit + 1) as i64)
            .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("candidate inventory deadline exceeded")??;
        ensure!(
            suffixes
                .iter()
                .all(|suffix| suffix.len() <= 1024 && !suffix.contains('\0')),
            "candidate inventory contains an unsupported branch name"
        );
        let more = suffixes.len() > limit;
        let next = more.then(|| suffixes[limit - 1].clone());
        let mut candidates = Vec::with_capacity(limit);
        for suffix in suffixes.into_iter().take(limit) {
            if Uuid::parse_str(&suffix).is_ok_and(|id| id.simple().to_string() == suffix) {
                candidates.push(
                    self.candidate_ref_status_locked(&format!("{CANDIDATE_PREFIX}{suffix}"))
                        .await?,
                );
            }
        }
        Ok(CandidateInventoryPage { candidates, next })
    }

    pub(crate) async fn candidate_ref_status(&self, branch: &str) -> Result<CandidateRefStatus> {
        self.readable()?;
        ensure!(
            self.branch == "main",
            "candidate status requires the live view"
        );
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        self.candidate_ref_status_locked(branch).await
    }

    async fn candidate_ref_status_locked(&self, branch: &str) -> Result<CandidateRefStatus> {
        let names = CandidateNames::from_open(branch)?;
        let heads = candidate_heads(&self.pool, &names).await?;
        let head = heads
            .get(&names.open)
            .or_else(|| heads.get(&names.promoting))
            .or_else(|| heads.get(&names.abandoned))
            .cloned();
        let state = if heads.is_empty() {
            CandidateRefState::Missing
        } else if heads.values().any(|value| Some(value) != head.as_ref()) {
            CandidateRefState::TransitionUncertain
        } else if heads.contains_key(&names.abandoned) {
            CandidateRefState::Resolved
        } else if heads.contains_key(&names.promoting) {
            // A promoting marker can precede the merge. In particular, a
            // target equal to its base has no unique outcome after cleanup.
            CandidateRefState::TransitionUncertain
        } else {
            let base: String = tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
                    .bind(&names.open)
                    .bind("main")
                    .fetch_one(self.pool.as_ref()),
            )
            .await
            .context("candidate inventory base lookup deadline exceeded")??;
            let state = if self.revision().await? == base {
                CandidateRefState::OpenUnchanged
            } else {
                CandidateRefState::OpenConflict
            };
            return Ok(CandidateRefStatus {
                branch: names.open,
                head,
                base: Some(base),
                state,
            });
        };
        Ok(CandidateRefStatus {
            branch: names.open,
            head,
            base: None,
            state,
        })
    }

    /// Explicit selected-ref disposal, never called from attachment Drop or
    /// uncertain transition recovery. The service must additionally exclude
    /// other live attachments before calling this local operation.
    pub(crate) async fn abandon_candidate_ref(
        &self,
        branch: &str,
        expected_base: &str,
        expected_head: &str,
    ) -> Result<()> {
        self.writable()?;
        if self.branch != "main" {
            return Err(CandidateRefRejected(CandidateRefRefusal::Invalid).into());
        }
        let names = CandidateNames::from_open(branch)
            .map_err(|_| CandidateRefRejected(CandidateRefRefusal::Invalid))?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain()
            .await
            .context(CandidateFailureStage::OutcomeReconciliation)?;
        let heads = candidate_heads(&self.pool, &names)
            .await
            .context(CandidateFailureStage::RefInspection)?;
        if heads.len() != 1
            || !heads
                .get(&names.open)
                .is_some_and(|head| head == expected_head)
        {
            return Err(
                anyhow::Error::new(CandidateRefRejected(CandidateRefRefusal::Changed))
                    .context(CandidateFailureStage::RefInspection),
            );
        }
        let base: String = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
                .bind(&names.open)
                .bind("main")
                .fetch_one(self.pool.as_ref()),
        )
        .await
        .context("candidate abandonment base lookup deadline exceeded")
        .context(CandidateFailureStage::RefInspection)?
        .context(CandidateFailureStage::RefInspection)?;
        if base != expected_base {
            return Err(CandidateRefRejected(CandidateRefRefusal::Changed).into());
        }
        let pool = self
            .shared
            .server
            .pool(&names.open)
            .await
            .context(CandidateFailureStage::WorkingSetInspection)?;
        if Arc::strong_count(&pool) != 1 {
            return Err(CandidateRefRejected(CandidateRefRefusal::Active).into());
        }
        tokio::time::timeout(QUERY_TIMEOUT, migrations::validate_current(&pool))
            .await
            .map_err(|_| CandidateRefRejected(CandidateRefRefusal::SchemaUnverified))
            .context(CandidateFailureStage::SchemaValidation)?
            .map_err(|_| CandidateRefRejected(CandidateRefRefusal::SchemaUnverified))
            .context(CandidateFailureStage::SchemaValidation)?;
        tokio::time::timeout(QUERY_TIMEOUT, pool.close())
            .await
            .map_err(|_| CandidateRefRejected(CandidateRefRefusal::SchemaUnverified))
            .context(CandidateFailureStage::PoolRetirement)?;
        drop(pool);
        let heads = candidate_heads(&self.pool, &names)
            .await
            .context(CandidateFailureStage::RefInspection)?;
        if heads.len() != 1
            || !heads
                .get(&names.open)
                .is_some_and(|head| head == expected_head)
        {
            return Err(
                anyhow::Error::new(CandidateRefRejected(CandidateRefRefusal::Changed))
                    .context(CandidateFailureStage::RefInspection),
            );
        }
        abandon_candidate(self, &names).await
    }

    /// Read-only transition proof for a branch and revisions captured before
    /// dispatch. Reclaimed refs without a unique result stay indeterminate;
    /// merely losing a connection never authorizes candidate deletion.
    pub(crate) async fn candidate_transition_observation(
        &self,
        branch: &str,
        base: &str,
        target: &str,
    ) -> Result<CandidateTransitionObservation> {
        self.readable()?;
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let names = CandidateNames::from_open(branch)?;
        let heads = candidate_heads(&self.pool, &names).await?;
        let matching = |name: &str| heads.get(name).is_some_and(|head| head == target);
        if heads.values().any(|head| head != target) {
            return Ok(CandidateTransitionObservation::Indeterminate);
        }
        if matching(&names.abandoned) {
            return Ok(CandidateTransitionObservation::Abandoned);
        }
        let current = self.revision().await?;
        let target_is_ancestor = if current == target {
            Some(true)
        } else {
            match tokio::time::timeout(
                QUERY_TIMEOUT,
                sqlx::query_scalar::<_, String>("SELECT DOLT_MERGE_BASE(?, ?)")
                    .bind(target)
                    .bind(&current)
                    .fetch_one(self.pool.as_ref()),
            )
            .await
            {
                Ok(Ok(ancestor)) => Some(ancestor == target),
                Ok(Err(_)) | Err(_) => None,
            }
        };
        if target != base && target_is_ancestor == Some(true) {
            return Ok(CandidateTransitionObservation::Promoted);
        }
        if matching(&names.promoting) {
            return Ok(CandidateTransitionObservation::Indeterminate);
        }
        if matching(&names.open) {
            return Ok(if current == base {
                CandidateTransitionObservation::OpenUnchanged
            } else {
                CandidateTransitionObservation::OpenConflict
            });
        }
        // Once the service has proven the original worker completed or the
        // previous owner was reaped, a missing exact ref with a candidate-only
        // commit outside main's ancestry is an abandoned result. An unchanged
        // candidate (target == base) has no distinguishable history here.
        Ok(if target != base && target_is_ancestor == Some(false) {
            CandidateTransitionObservation::AbandonedReclaimed
        } else {
            CandidateTransitionObservation::Indeterminate
        })
    }

    async fn candidate_from_branch(&self, branch: String, base: String) -> Result<Candidate> {
        let pool = self.shared.server.pool(&branch).await?;
        let view = Self {
            shared: self.shared.clone(),
            pool,
            branch,
            logical_receipt: None,
        };
        Ok(Candidate {
            live: self.clone(),
            view,
            base,
            promoted: Arc::new(StdMutex::new(None)),
        })
    }

    pub(crate) fn candidate_branch_for_id(&self, id: Uuid) -> String {
        let mut hash = Sha256::new();
        for field in [
            b"kuru-memory-candidate-creation-v1".as_slice(),
            self.shared.server.instance().as_bytes(),
            id.as_bytes(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        format!(
            "{CANDIDATE_PREFIX}{}",
            Uuid::new_v5(&Uuid::NAMESPACE_OID, &hash.finalize()).simple()
        )
    }

    async fn recover_candidates(&self) -> Result<()> {
        let _guard = self.shared.write.lock().await;
        self.resolve_uncertain().await?;
        let rows: Vec<(String, String)> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_as(
                "SELECT name, hash FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ? OR LEFT(BINARY name, ?) = BINARY ? ORDER BY BINARY name LIMIT ?",
            )
            .bind(PROMOTING_PREFIX.len() as i64)
            .bind(PROMOTING_PREFIX)
            .bind(ABANDONED_PREFIX.len() as i64)
            .bind(ABANDONED_PREFIX)
            .bind(CANDIDATE_RECOVERY_BATCH)
            .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("candidate recovery inventory deadline exceeded")??;
        #[cfg(test)]
        if let Some(pause) = &self.shared.candidate_recovery_pause {
            pause.reached.add_permits(1);
            pause
                .resume
                .acquire()
                .await
                .context("candidate recovery fixture release channel closed")?
                .forget();
        }
        for (status, target) in rows {
            let names = CandidateNames::from_status(&status)?;
            let heads = candidate_heads(&self.pool, &names).await?;
            ensure!(
                heads.get(&status).is_some_and(|head| head == &target),
                "candidate recovery inventory changed during inspection"
            );
            if status == names.promoting {
                ensure!(
                    !heads.contains_key(&names.abandoned),
                    "candidate has both promoting and abandoned status refs"
                );
                if validate_candidate_pair(&heads, &names.open, &names.promoting, &target).is_err()
                {
                    continue;
                }
                let mut clean = true;
                for branch in [&names.open, &names.promoting] {
                    if heads.contains_key(branch)
                        && !candidate_branch_is_clean(self, branch).await?
                    {
                        clean = false;
                        break;
                    }
                }
                if !clean {
                    continue;
                }
                let current = self.revision().await?;
                let merge_base: String = tokio::time::timeout(
                    QUERY_TIMEOUT,
                    sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
                        .bind(&target)
                        .bind(&current)
                        .fetch_one(self.pool.as_ref()),
                )
                .await
                .context("candidate ancestry inspection deadline exceeded")??;
                if merge_base != target {
                    continue;
                }
                if let Err(error) = cleanup_promoted_candidate(self, &names, &target).await {
                    preserve_resolved_cleanup(self, &names, &names.promoting, &target, error)
                        .await?;
                }
            } else {
                let mut eligible = true;
                for branch in [&names.open, &names.promoting, &names.abandoned] {
                    if let Some(head) = heads.get(branch) {
                        if head != &target {
                            eligible = false;
                            break;
                        }
                        if !candidate_branch_is_clean(self, branch).await? {
                            eligible = false;
                            break;
                        }
                    }
                }
                if !eligible {
                    continue;
                }
                if let Err(error) = cleanup_abandoned_candidate(self, &names, &target, &heads).await
                {
                    preserve_resolved_cleanup(self, &names, &names.abandoned, &target, error)
                        .await?;
                }
            }
        }
        Ok(())
    }
    pub async fn revision(&self) -> Result<String> {
        self.readable()?;
        revision(&self.pool).await
    }
    pub async fn revisions(&self, limit: usize) -> Result<Vec<Revision>> {
        self.readable()?;
        let limit = i64::try_from(limit).context("revision limit exceeds integer range")?;
        let rows = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query(
                "SELECT commit_hash, message FROM dolt_log ORDER BY commit_order DESC, commit_hash ASC LIMIT ?",
            )
                .bind(limit)
                .fetch_all(self.pool.as_ref()),
        )
        .await
        .context("revision read deadline exceeded")??;
        rows.into_iter()
            .map(|row| {
                Ok(Revision {
                    hash: row.try_get("commit_hash")?,
                    message: row.try_get("message")?,
                })
            })
            .collect()
    }
    pub async fn status(&self) -> Result<MemoryStatus> {
        self.readable()?;
        Ok(MemoryStatus {
            engine: "dolt",
            engine_version: provision::DOLT_VERSION,
            project: self.shared.project_scope.clone(),
            directory: self.shared.directory.clone(),
            branch: self.branch.clone(),
            revision: self.revision().await?,
            read_only: self.shared.read_only,
        })
    }

    pub(crate) fn try_acquire_dream_lease(&self) -> Option<OwnedMutexGuard<()>> {
        self.shared.dream.clone().try_lock_owned().ok()
    }

    pub(crate) async fn acquire_dream_lease(&self) -> OwnedMutexGuard<()> {
        self.shared.dream.clone().lock_owned().await
    }
    /// Explicitly shut down this shared server handle and every view that
    /// clones it. Dropping a view only releases that view.
    pub async fn close(self) -> Result<()> {
        let _guard = self.shared.write.lock().await;
        self.shared.server.close().await
    }

    pub(crate) async fn fixture_commit_malformed_state(&self, key: &str) -> Result<()> {
        self.writable()?;
        identifier("state key", key, 1024)?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(key.as_bytes())
            .bind("{malformed")
            .execute(self.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'malformed fixture state', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(self.pool.as_ref())
            .await?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn fixture_insert_public_session(
        &self,
        catalog: &SessionCatalogRecord,
        turns: &[PublicTurnRecord],
    ) -> Result<()> {
        self.writable()?;
        validate_session_catalog(catalog)?;
        for turn in turns {
            validate_public_turn(turn)?;
        }
        let _guard = self.shared.write.lock().await;
        let mut transaction = self.pool.begin().await?;
        for record in turns {
            sqlx::query("INSERT INTO session_public_turns (node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&record.node_id)
                .bind(record.origin_session_id.as_bytes())
                .bind(record.turn_id.as_bytes())
                .bind(match record.kind {
                    PublicTurnKind::Primary => "primary",
                    PublicTurnKind::Continuation => "continuation",
                    PublicTurnKind::LegacyContinuation => "legacy",
                })
                .bind(record.continuation_of_node_id.as_deref())
                .bind(record.predecessor_node_id.as_deref())
                .bind(match record.settlement {
                    PublicTurnSettlement::Pending => "pending",
                    PublicTurnSettlement::Completed => "completed",
                    PublicTurnSettlement::Interrupted => "interrupted",
                })
                .bind(record.user_entry.as_ref().map(serde_json::to_string).transpose()?)
                .bind(record.speaker_id.as_deref().map(str::as_bytes))
                .bind((!record.terminal_entries.is_empty()).then(|| serde_json::to_string(&record.terminal_entries)).transpose()?)
                .bind(&record.record_format)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query("INSERT INTO session_catalog (session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(catalog.session_id.as_bytes())
            .bind(serde_json::to_value(catalog.mode)?.as_str().context("session mode is not a string")?)
            .bind(&catalog.label)
            .bind(i64::try_from(catalog.created_order)?)
            .bind(i64::try_from(catalog.updated_order)?)
            .bind(i64::try_from(catalog.lifecycle_generation)?)
            .bind(session_lifecycle_sql(catalog.lifecycle_state))
            .bind(catalog.head_node_id.as_deref())
            .bind(catalog.pending_node_id.as_deref())
            .bind(catalog.legacy_prefix.as_ref().map(serde_json::to_string).transpose()?)
            .bind(catalog.fork_provenance.as_ref().map(serde_json::to_string).transpose()?)
            .bind(&catalog.record_format)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'public session fixture', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

async fn validate_session_fork_source(
    connection: &mut MySqlConnection,
    source_session_id: &str,
    expected_source_generation: u64,
    source_node_id: &str,
) -> Result<ValidatedSessionFork> {
    let mut transaction = connection.begin().await?;
    let source = sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ?")
        .bind(source_session_id.as_bytes())
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| decode_session_catalog_row(&row))
        .transpose()?
        .ok_or(SessionLifecycleRejected(SessionLifecycleRefusal::Missing))?;
    if source.lifecycle_generation != expected_source_generation {
        return Err(SessionLifecycleRejected(SessionLifecycleRefusal::GenerationChanged).into());
    }
    if source.lifecycle_state == SessionLifecycleState::Removed {
        return Err(SessionLifecycleRejected(SessionLifecycleRefusal::SourceRemoved).into());
    }

    let selected = load_public_turn(&mut transaction, source_node_id)
        .await?
        .ok_or(SessionLifecycleRejected(
            SessionLifecycleRefusal::SourceNodeMissing,
        ))?;
    if selected.settlement == PublicTurnSettlement::Pending {
        return Err(SessionLifecycleRejected(SessionLifecycleRefusal::SourceNodePending).into());
    }

    let mut current = source.head_node_id.clone();
    let mut reachable = false;
    let mut traversed = 0_u64;
    while let Some(node_id) = current {
        let node = load_public_turn(&mut transaction, &node_id)
            .await?
            .context("session fork predecessor turn does not exist")?;
        ensure!(
            node.node_id == node_id && node.settlement != PublicTurnSettlement::Pending,
            "session fork predecessor chain is invalid"
        );
        traversed = traversed
            .checked_add(1)
            .context("session fork predecessor count overflowed")?;
        if node_id == source_node_id {
            ensure!(
                node == selected,
                "session fork selected node changed during traversal"
            );
            reachable = true;
            break;
        }
        current = node.predecessor_node_id;
    }
    if !reachable {
        return Err(
            SessionLifecycleRejected(SessionLifecycleRefusal::SourceNodeUnreachable).into(),
        );
    }
    transaction.commit().await?;
    Ok(ValidatedSessionFork { source, selected })
}

async fn run_migration_worker(
    server: Server,
    pool: Arc<MySqlPool>,
    #[cfg(test)] hooks: Option<Arc<migrations::MigrationRunnerHooks>>,
) -> Result<(File, Result<()>)> {
    let (result, waiting) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        #[cfg(test)]
        let upgrade = match hooks {
            Some(hooks) => migrations::upgrade_with_hooks(&server, &pool, &hooks).await,
            None => migrations::upgrade(&server, &pool).await,
        };
        #[cfg(not(test))]
        let upgrade = migrations::upgrade(&server, &pool).await;
        let cleanup = close_migration_worker(server, pool).await;
        let outcome = match (upgrade, cleanup) {
            (upgrade, Ok(lock)) => Ok((lock, upgrade)),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => {
                Err(error.context(format!("memory migration cleanup also failed: {cleanup:#}")))
            }
        };
        let _ = result.send(outcome);
    });
    waiting
        .await
        .context("memory migration worker stopped before cleanup")?
}

async fn run_candidate_recovery_worker(store: &MemoryStore) -> Result<()> {
    let worker_store = store.clone();
    let (result, waiting) = tokio::sync::oneshot::channel();
    let (accepted, delivery) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        match worker_store.recover_candidates().await {
            Ok(()) => {
                if result.send(Ok(())).is_err() || delivery.await.is_err() {
                    let _ = close_candidate_recovery_worker(worker_store).await;
                }
            }
            Err(error) => {
                let cleanup = close_candidate_recovery_worker(worker_store).await;
                let outcome = match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(error.context(format!(
                        "candidate recovery cleanup also failed: {cleanup:#}"
                    ))),
                };
                let _ = result.send(outcome);
            }
        }
    });
    waiting
        .await
        .context("candidate recovery worker stopped before cleanup")??;
    // This acknowledgement and guard release contain no cancellation point.
    // A cancelled opener instead closes `delivery`, so the worker settles the
    // server before its installed startup guard can be released.
    accepted
        .send(())
        .map_err(|_| anyhow::anyhow!("candidate recovery worker stopped before handoff"))?;
    let lock: File = store.shared.server.take_reap_guard();
    drop(lock);
    Ok(())
}

async fn close_candidate_recovery_worker(store: MemoryStore) -> Result<()> {
    let stopped = store.shared.server.close_installed_guard().await;
    drop(store);
    let lock = stopped?;
    drop(lock);
    Ok(())
}

async fn close_migration_worker(server: Server, _pool: Arc<MySqlPool>) -> Result<File> {
    // The caller installed the startup guard before it began any operation.
    // If cancellation happens while pools drain, Owner::drop transfers it to
    // the independent reaper.
    // `Server::close_installed_guard` drains every registered branch pool under
    // its existing bounded close/reap discipline. Do not await an unbounded
    // individual pool close before that ownership boundary.
    let stopped = server.close_installed_guard().await;
    drop(server);
    // Even a bounded-close error is not permission to hand writer authority to
    // another opener: Owner transfers the installed guard to its independent
    // supervisor observer until Dolt has actually reaped.
    stopped
}

#[derive(Debug)]
enum Mutation {
    Append {
        namespace: String,
        session_id: Option<String>,
        role: String,
        content: String,
        format: Option<&'static str>,
    },
    State(Vec<(String, String)>),
    PrivateReasoningSummaries(Vec<(String, String)>),
    Checkpoint {
        namespace: String,
        session_id: Option<String>,
        messages: Vec<(String, String)>,
        values: Vec<(String, String)>,
        public_turn: Option<Box<EncodedSessionTurnCheckpoint>>,
        mode: Option<SessionModeCheckpoint>,
    },
    Clear(String),
    ForgetNote {
        namespace: String,
        sequence: i64,
    },
    ContextSummary {
        record: Box<ContextSummaryRecord>,
        summary_id: String,
        record_format: &'static str,
        private_reasoning: Vec<(String, String)>,
    },
}

#[derive(Debug)]
enum EncodedSessionTurnCheckpoint {
    Admit {
        expected_generation: u64,
        turn_id: String,
        label: Option<String>,
        expected_transcript_rows: Option<u64>,
        user_entry: String,
    },
    MarkRetryableInterruption {
        expected_generation: u64,
        turn_id: String,
        speaker_id: String,
        terminal_entries: String,
    },
    Resume {
        expected_generation: u64,
        turn_id: String,
        legacy: Option<LegacySessionTurnResume>,
    },
    Settle {
        expected_generation: u64,
        turn_id: String,
        settlement: PublicTurnSettlement,
        speaker_id: String,
        terminal_entries: String,
    },
}

#[derive(Clone, Debug)]
enum SessionLifecycleMutation {
    Create {
        session_id: String,
        mode: Mode,
        label: String,
    },
    Rename {
        session_id: String,
        expected_generation: u64,
        label: String,
    },
    Remove {
        session_id: String,
        expected_generation: u64,
    },
    Restore {
        session_id: String,
        expected_generation: u64,
    },
    Fork {
        source_session_id: String,
        expected_source_generation: u64,
        source_node_id: String,
        child_session_id: String,
        label: String,
        validated: Option<Box<ValidatedSessionFork>>,
    },
}

#[derive(Clone, Debug)]
struct ValidatedSessionFork {
    source: SessionCatalogRecord,
    selected: PublicTurnRecord,
}

impl SessionLifecycleMutation {
    fn session_id(&self) -> &str {
        match self {
            Self::Create { session_id, .. }
            | Self::Rename { session_id, .. }
            | Self::Remove { session_id, .. }
            | Self::Restore { session_id, .. } => session_id,
            Self::Fork {
                child_session_id, ..
            } => child_session_id,
        }
    }
}

async fn apply(
    connection: &mut MySqlConnection,
    operation: &str,
    label: &str,
    mutation: Mutation,
    logical: Option<&LogicalReceipt>,
) -> Result<()> {
    let mut transaction = connection.begin().await?;
    match mutation {
        Mutation::Append {
            namespace,
            session_id,
            role,
            content,
            format,
        } => {
            if let Some(session_id) = session_id {
                let format = format.context("session message format is missing")?;
                sqlx::query("INSERT INTO messages (namespace, session_id, role, content_format, content) VALUES (?, ?, ?, ?, ?)")
                    .bind(namespace.as_bytes())
                    .bind(session_id.as_bytes())
                    .bind(role.as_bytes())
                    .bind(format)
                    .bind(content)
                    .execute(&mut *transaction)
                    .await?;
            } else if let Some(format) = format {
                sqlx::query("INSERT INTO messages (namespace, role, content_format, content) VALUES (?, ?, ?, ?)")
                    .bind(namespace.as_bytes())
                    .bind(role.as_bytes())
                    .bind(format)
                    .bind(content)
                    .execute(&mut *transaction)
                    .await?;
            } else {
                // This path also works for a retained pre-v3 candidate branch.
                sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
                    .bind(namespace.as_bytes())
                    .bind(role.as_bytes())
                    .bind(content)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        Mutation::State(values) => {
            for (key, value) in values {
                sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)").bind(key.as_bytes()).bind(value).execute(&mut *transaction).await?;
            }
        }
        Mutation::PrivateReasoningSummaries(values) => {
            for (key, value) in values {
                let existing: Option<String> =
                    sqlx::query_scalar("SELECT value FROM state WHERE `key` = ? FOR UPDATE")
                        .bind(key.as_bytes())
                        .fetch_optional(&mut *transaction)
                        .await?;
                if let Some(existing) = existing {
                    if existing != value {
                        return Err(ReasoningSummaryConflict.into());
                    }
                } else {
                    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                        .bind(key.as_bytes())
                        .bind(value)
                        .execute(&mut *transaction)
                        .await?;
                }
            }
        }
        Mutation::Checkpoint {
            namespace,
            session_id,
            messages,
            values,
            public_turn,
            mode,
        } => {
            if let Some(mode) = mode {
                apply_session_mode_checkpoint(
                    &mut transaction,
                    &namespace,
                    session_id
                        .as_deref()
                        .context("mode checkpoint lacks its session identity")?,
                    &values,
                    &mode,
                )
                .await?;
            }
            if let Some(public_turn) = public_turn {
                apply_session_turn_checkpoint(
                    &mut transaction,
                    &namespace,
                    session_id
                        .as_deref()
                        .context("public turn checkpoint lacks its session identity")?,
                    &values,
                    &public_turn,
                )
                .await?;
            }
            for (role, content) in messages {
                if let Some(session_id) = &session_id {
                    sqlx::query("INSERT INTO messages (namespace, session_id, role, content_format, content) VALUES (?, ?, ?, ?, ?)")
                        .bind(namespace.as_bytes())
                        .bind(session_id.as_bytes())
                        .bind(role.as_bytes())
                        .bind(TYPED_FORMAT)
                        .bind(content)
                        .execute(&mut *transaction)
                        .await?;
                } else {
                    sqlx::query("INSERT INTO messages (namespace, role, content_format, content) VALUES (?, ?, ?, ?)")
                        .bind(namespace.as_bytes())
                        .bind(role.as_bytes())
                        .bind(TYPED_FORMAT)
                        .bind(content)
                        .execute(&mut *transaction)
                        .await?;
                }
            }
            for (key, value) in values {
                sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?) ON DUPLICATE KEY UPDATE value = VALUES(value)").bind(key.as_bytes()).bind(value).execute(&mut *transaction).await?;
            }
        }
        Mutation::Clear(namespace) => {
            sqlx::query("DELETE FROM messages WHERE namespace = ?")
                .bind(namespace.as_bytes())
                .execute(&mut *transaction)
                .await?;
        }
        Mutation::ForgetNote {
            namespace,
            sequence,
        } => {
            let deleted = sqlx::query("DELETE FROM messages WHERE namespace = ? AND sequence = ?")
                .bind(namespace.as_bytes())
                .bind(sequence)
                .execute(&mut *transaction)
                .await?;
            ensure!(
                deleted.rows_affected() == 1,
                "note sequence {sequence} is not present in this notes namespace"
            );
        }
        Mutation::ContextSummary {
            record,
            summary_id,
            record_format,
            private_reasoning,
        } => {
            let current_revision: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
                .fetch_one(&mut *transaction)
                .await?;
            if current_revision != record.source_revision {
                ensure!(
                    context_summary_source_unchanged(&mut transaction, &record, &current_revision)
                        .await?,
                    ContextSummaryStale
                );
            }
            let current_cursor: Option<i64> = sqlx::query_scalar(
                "SELECT through_sequence FROM context_summary_cursors WHERE actor_namespace = ? AND session_id = ? AND source_namespace = ? FOR UPDATE",
            )
            .bind(record.actor_namespace.as_bytes())
            .bind(record.session_id.as_bytes())
            .bind(record.source_namespace.as_bytes())
            .fetch_optional(&mut *transaction)
            .await?;
            ensure!(
                current_cursor.unwrap_or(0) == record.after_sequence,
                ContextSummaryStale
            );
            ensure!(
                context_summary_range_is_bounded(&mut transaction, &record).await?,
                ContextSummaryStale
            );
            for (key, value) in private_reasoning {
                let existing: Option<String> =
                    sqlx::query_scalar("SELECT value FROM state WHERE `key` = ? FOR UPDATE")
                        .bind(key.as_bytes())
                        .fetch_optional(&mut *transaction)
                        .await?;
                if let Some(existing) = existing {
                    if existing != value {
                        return Err(ReasoningSummaryConflict.into());
                    }
                } else {
                    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                        .bind(key.as_bytes())
                        .bind(value)
                        .execute(&mut *transaction)
                        .await?;
                }
            }
            sqlx::query("INSERT INTO context_summaries (summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, operation_id, producer_actor_id, invocation_id, record_format, summary) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&summary_id)
                .bind(record.actor_namespace.as_bytes())
                .bind(record.session_id.as_bytes())
                .bind(record.source_namespace.as_bytes())
                .bind(record.summary_namespace.as_bytes())
                .bind(&record.source_view)
                .bind(&record.source_revision)
                .bind(record.after_sequence)
                .bind(record.through_sequence)
                .bind(record.turn_id.as_ref().map(|value| value.as_bytes()))
                .bind(record.operation_id.as_ref().map(|value| value.as_bytes()))
                .bind(record.producer_actor_id.as_ref().map(|value| value.as_bytes()))
                .bind(record.invocation_id.as_bytes())
                .bind(record_format)
                .bind(&record.summary)
                .execute(&mut *transaction)
                .await?;
            if current_cursor.is_some() {
                let updated = sqlx::query("UPDATE context_summary_cursors SET through_sequence = ?, summary_id = ?, source_view = ?, source_revision = ? WHERE actor_namespace = ? AND session_id = ? AND source_namespace = ? AND through_sequence = ?")
                    .bind(record.through_sequence)
                    .bind(&summary_id)
                    .bind(&record.source_view)
                    .bind(&record.source_revision)
                    .bind(record.actor_namespace.as_bytes())
                    .bind(record.session_id.as_bytes())
                    .bind(record.source_namespace.as_bytes())
                    .bind(record.after_sequence)
                    .execute(&mut *transaction)
                    .await?;
                ensure!(updated.rows_affected() == 1, ContextSummaryStale);
            } else {
                sqlx::query("INSERT INTO context_summary_cursors (actor_namespace, session_id, source_namespace, through_sequence, summary_id, source_view, source_revision) VALUES (?, ?, ?, ?, ?, ?, ?)")
                    .bind(record.actor_namespace.as_bytes())
                    .bind(record.session_id.as_bytes())
                    .bind(record.source_namespace.as_bytes())
                    .bind(record.through_sequence)
                    .bind(&summary_id)
                    .bind(&record.source_view)
                    .bind(&record.source_revision)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
    }
    // Historical candidate views keep their committed schema and sole-receipt
    // behavior. Current writable views retain every indexed receipt so a later
    // sibling mutation cannot erase evidence of an accepted operation.
    let version: i32 = sqlx::query_scalar("SELECT version FROM kuru_schema WHERE id = 1")
        .fetch_one(&mut *transaction)
        .await?;
    if version <= 4 {
        sqlx::query("DELETE FROM operations")
            .execute(&mut *transaction)
            .await?;
    } else {
        ensure!(
            version == migrations::CURRENT_VERSION,
            "unsupported writable memory schema version {version}"
        );
    }
    if let Some(receipt) = logical {
        sqlx::query("INSERT INTO operations (id, label, receipt_format, method, request_digest) VALUES (?, ?, 1, ?, ?)")
            .bind(operation)
            .bind(label)
            .bind(&receipt.method)
            .bind(&receipt.digest)
            .execute(&mut *transaction)
            .await?;
    } else {
        sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
            .bind(operation)
            .bind(label)
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(format!("{label} [{operation}]"))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn apply_session_lifecycle(
    connection: &mut MySqlConnection,
    operation: &str,
    label: &str,
    mutation: SessionLifecycleMutation,
    logical: Option<&LogicalReceipt>,
) -> Result<SessionLifecycleOutcome> {
    let mut transaction = connection.begin().await?;
    let session_id = mutation.session_id().to_owned();
    let existing = sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ? FOR UPDATE")
        .bind(session_id.as_bytes())
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| decode_session_catalog_row(&row))
        .transpose()?;
    let next_order: i64 = sqlx::query_scalar("SELECT CAST(GREATEST(COALESCE(MAX(created_order), 0), COALESCE(MAX(updated_order), 0)) + 1 AS SIGNED) FROM session_catalog")
        .fetch_one(&mut *transaction)
        .await?;
    ensure!(next_order > 0, "session catalog order overflowed");
    let next_order = u64::try_from(next_order)?;

    let record = match mutation {
        SessionLifecycleMutation::Create {
            session_id,
            mode,
            label,
        } => {
            if existing.is_some() {
                return Err(
                    SessionLifecycleRejected(SessionLifecycleRefusal::AlreadyExists).into(),
                );
            }
            let record = SessionCatalogRecord {
                session_id,
                mode,
                label,
                created_order: next_order,
                updated_order: next_order,
                lifecycle_generation: 0,
                lifecycle_state: SessionLifecycleState::Active,
                head_node_id: None,
                pending_node_id: None,
                legacy_prefix: None,
                fork_provenance: None,
                record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
            };
            validate_session_catalog(&record)?;
            let mode = serde_json::to_value(record.mode)?;
            let mode = mode.as_str().context("session mode is not a string")?;
            sqlx::query("INSERT INTO session_catalog (session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?)")
                .bind(record.session_id.as_bytes())
                .bind(mode)
                .bind(&record.label)
                .bind(i64::try_from(record.created_order)?)
                .bind(i64::try_from(record.updated_order)?)
                .bind(i64::try_from(record.lifecycle_generation)?)
                .bind(session_lifecycle_sql(record.lifecycle_state))
                .bind(record.head_node_id.as_deref())
                .bind(record.pending_node_id.as_deref())
                .bind(&record.record_format)
                .execute(&mut *transaction)
                .await?;
            record
        }
        SessionLifecycleMutation::Rename {
            expected_generation,
            label,
            ..
        } => {
            let mut record = checked_session_lifecycle_record(existing, expected_generation)?;
            record.label = label;
            record.updated_order = next_order;
            record.lifecycle_generation = record
                .lifecycle_generation
                .checked_add(1)
                .context("session lifecycle generation overflowed")?;
            validate_session_catalog(&record)?;
            update_session_lifecycle_row(&mut transaction, &record, expected_generation).await?;
            record
        }
        SessionLifecycleMutation::Remove {
            expected_generation,
            ..
        } => {
            let mut record = checked_session_lifecycle_record(existing, expected_generation)?;
            if record.lifecycle_state == SessionLifecycleState::Removed {
                return Err(
                    SessionLifecycleRejected(SessionLifecycleRefusal::AlreadyRemoved).into(),
                );
            }
            record.lifecycle_state = SessionLifecycleState::Removed;
            record.updated_order = next_order;
            record.lifecycle_generation = record
                .lifecycle_generation
                .checked_add(1)
                .context("session lifecycle generation overflowed")?;
            validate_session_catalog(&record)?;
            update_session_lifecycle_row(&mut transaction, &record, expected_generation).await?;
            record
        }
        SessionLifecycleMutation::Restore {
            expected_generation,
            ..
        } => {
            let mut record = checked_session_lifecycle_record(existing, expected_generation)?;
            if record.lifecycle_state == SessionLifecycleState::Active {
                return Err(
                    SessionLifecycleRejected(SessionLifecycleRefusal::AlreadyActive).into(),
                );
            }
            record.lifecycle_state = SessionLifecycleState::Active;
            record.updated_order = next_order;
            record.lifecycle_generation = record
                .lifecycle_generation
                .checked_add(1)
                .context("session lifecycle generation overflowed")?;
            validate_session_catalog(&record)?;
            update_session_lifecycle_row(&mut transaction, &record, expected_generation).await?;
            record
        }
        SessionLifecycleMutation::Fork {
            source_session_id,
            expected_source_generation,
            source_node_id,
            child_session_id,
            label,
            validated,
        } => {
            if existing.is_some() {
                return Err(
                    SessionLifecycleRejected(SessionLifecycleRefusal::AlreadyExists).into(),
                );
            }
            let validated = validated.context("session fork source was not validated")?;
            ensure!(
                validated.source.session_id == source_session_id
                    && validated.source.lifecycle_generation == expected_source_generation
                    && validated.selected.node_id == source_node_id,
                "session fork validation coordinates changed"
            );
            let current_source = sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ? FOR UPDATE")
                .bind(source_session_id.as_bytes())
                .fetch_optional(&mut *transaction)
                .await?
                .map(|row| decode_session_catalog_row(&row))
                .transpose()?
                .ok_or(SessionLifecycleRejected(SessionLifecycleRefusal::Missing))?;
            if current_source.lifecycle_generation != expected_source_generation {
                return Err(
                    SessionLifecycleRejected(SessionLifecycleRefusal::GenerationChanged).into(),
                );
            }
            if current_source.lifecycle_state == SessionLifecycleState::Removed {
                return Err(
                    SessionLifecycleRejected(SessionLifecycleRefusal::SourceRemoved).into(),
                );
            }
            ensure!(
                current_source == validated.source,
                SessionLifecycleRejected(SessionLifecycleRefusal::GenerationChanged)
            );
            let selected = load_public_turn(&mut transaction, &source_node_id)
                .await?
                .ok_or(SessionLifecycleRejected(
                    SessionLifecycleRefusal::SourceNodeMissing,
                ))?;
            ensure!(
                selected == validated.selected
                    && selected.settlement != PublicTurnSettlement::Pending,
                SessionLifecycleRejected(SessionLifecycleRefusal::SourceNodeUnreachable)
            );

            let fork_provenance = SessionForkProvenance {
                source_session_id: source_session_id.clone(),
                source_node_id: source_node_id.clone(),
                source_turn_id: selected.turn_id.clone(),
                source_label: current_source.label.clone(),
                shares_current_project_memory: true,
                record_format: FORK_PROVENANCE_RECORD_FORMAT.into(),
            };
            let record = SessionCatalogRecord {
                session_id: child_session_id,
                mode: current_source.mode,
                label,
                created_order: next_order,
                updated_order: next_order,
                lifecycle_generation: 0,
                lifecycle_state: SessionLifecycleState::Active,
                head_node_id: Some(source_node_id),
                pending_node_id: None,
                legacy_prefix: current_source.legacy_prefix.clone(),
                fork_provenance: Some(fork_provenance),
                record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
            };
            validate_session_catalog(&record)?;
            let mode = serde_json::to_value(record.mode)?;
            let mode = mode.as_str().context("session mode is not a string")?;
            sqlx::query("INSERT INTO session_catalog (session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format) VALUES (?, ?, ?, ?, ?, 0, 'active', ?, NULL, ?, ?, ?)")
                .bind(record.session_id.as_bytes())
                .bind(mode)
                .bind(&record.label)
                .bind(i64::try_from(record.created_order)?)
                .bind(i64::try_from(record.updated_order)?)
                .bind(record.head_node_id.as_deref())
                .bind(record.legacy_prefix.as_ref().map(serde_json::to_string).transpose()?)
                .bind(record.fork_provenance.as_ref().map(serde_json::to_string).transpose()?)
                .bind(&record.record_format)
                .execute(&mut *transaction)
                .await?;
            record
        }
    };

    let outcome = session_lifecycle_outcome(&record)?;
    let result_ref = encode_session_lifecycle_result_ref(&outcome)?;
    let version: i32 = sqlx::query_scalar("SELECT version FROM kuru_schema WHERE id = 1")
        .fetch_one(&mut *transaction)
        .await?;
    ensure!(
        version == migrations::CURRENT_VERSION,
        "unsupported writable memory schema version {version}"
    );
    if let Some(receipt) = logical {
        sqlx::query("INSERT INTO operations (id, label, receipt_format, method, request_digest, result_ref) VALUES (?, ?, 1, ?, ?, ?)")
            .bind(operation)
            .bind(label)
            .bind(&receipt.method)
            .bind(&receipt.digest)
            .bind(&result_ref)
            .execute(&mut *transaction)
            .await?;
    } else {
        sqlx::query("INSERT INTO operations (id, label, result_ref) VALUES (?, ?, ?)")
            .bind(operation)
            .bind(label)
            .bind(&result_ref)
            .execute(&mut *transaction)
            .await?;
    }
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(format!("{label} [{operation}]"))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(outcome)
}

async fn apply_session_mode_checkpoint(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    namespace: &str,
    session_id: &str,
    values: &[(String, String)],
    checkpoint: &SessionModeCheckpoint,
) -> Result<()> {
    let row = sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ? FOR UPDATE")
        .bind(session_id.as_bytes())
        .fetch_optional(&mut **transaction)
        .await?
        .context("mode checkpoint session is absent from the catalog")?;
    let mut catalog = decode_session_catalog_row(&row)?;
    ensure!(
        catalog.lifecycle_state == SessionLifecycleState::Active,
        "mode checkpoint session is removed"
    );
    ensure!(
        catalog.lifecycle_generation == checkpoint.expected_generation,
        "mode checkpoint lifecycle generation changed"
    );
    ensure!(
        catalog.mode == checkpoint.expected_mode,
        "mode checkpoint current mode changed"
    );
    ensure!(
        catalog.mode != checkpoint.mode,
        "mode checkpoint must change the mode"
    );
    let state_key = session_mode_state_key(namespace, session_id)?;
    let state = values
        .iter()
        .find(|(key, _)| key == &state_key)
        .context("mode checkpoint lacks the matching runtime session state")?;
    let state: Value = serde_json::from_str(&state.1)?;
    ensure!(
        state.get("id").and_then(Value::as_str) == Some(session_id),
        "mode checkpoint runtime session identity differs"
    );
    ensure!(
        state.get("mode") == Some(&serde_json::to_value(checkpoint.mode)?),
        "mode checkpoint runtime state has a different mode"
    );
    ensure!(
        state.get("lifecycle_generation").and_then(Value::as_u64)
            == Some(checkpoint.expected_generation),
        "mode checkpoint runtime state has a different lifecycle generation"
    );
    let next_order: i64 = sqlx::query_scalar("SELECT CAST(GREATEST(COALESCE(MAX(created_order), 0), COALESCE(MAX(updated_order), 0)) + 1 AS SIGNED) FROM session_catalog")
        .fetch_one(&mut **transaction)
        .await?;
    ensure!(next_order > 0, "session catalog order overflowed");
    catalog.mode = checkpoint.mode;
    catalog.updated_order = u64::try_from(next_order)?;
    validate_session_catalog(&catalog)?;
    let mode = serde_json::to_value(checkpoint.mode)?;
    let mode = mode.as_str().context("session mode is not a string")?;
    let old_mode = serde_json::to_value(checkpoint.expected_mode)?;
    let old_mode = old_mode
        .as_str()
        .context("previous session mode is not a string")?;
    let updated = sqlx::query("UPDATE session_catalog SET mode = ?, updated_order = ? WHERE session_id = ? AND lifecycle_generation = ? AND lifecycle_state = 'active' AND mode = ?")
        .bind(mode)
        .bind(next_order)
        .bind(session_id.as_bytes())
        .bind(i64::try_from(checkpoint.expected_generation)?)
        .bind(old_mode)
        .execute(&mut **transaction)
        .await?;
    ensure!(
        updated.rows_affected() == 1,
        "mode checkpoint catalog changed before publication"
    );
    Ok(())
}

fn session_mode_state_key(namespace: &str, session_id: &str) -> Result<String> {
    let suffix = format!("/transcript/{session_id}");
    let scope = namespace
        .strip_suffix(&suffix)
        .context("mode checkpoint transcript namespace does not match its session")?;
    ensure!(!scope.is_empty(), "mode checkpoint project scope is empty");
    Ok(format!("{scope}/session/{session_id}"))
}

async fn apply_session_turn_checkpoint(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    namespace: &str,
    session_id: &str,
    values: &[(String, String)],
    checkpoint: &EncodedSessionTurnCheckpoint,
) -> Result<()> {
    let row = sqlx::query("SELECT session_id, mode, label, created_order, updated_order, lifecycle_generation, lifecycle_state, head_node_id, pending_node_id, legacy_prefix, fork_provenance, record_format FROM session_catalog WHERE session_id = ? FOR UPDATE")
        .bind(session_id.as_bytes())
        .fetch_optional(&mut **transaction)
        .await?;
    let catalog = row
        .map(|row| decode_session_catalog_row(&row))
        .transpose()?
        .ok_or(SessionTurnRejected(SessionTurnRefusal::SessionMissing))?;
    if catalog.lifecycle_state != SessionLifecycleState::Active {
        return Err(SessionTurnRejected(SessionTurnRefusal::SessionRemoved).into());
    }
    let (expected_generation, turn_id) = match checkpoint {
        EncodedSessionTurnCheckpoint::Admit {
            expected_generation,
            turn_id,
            ..
        }
        | EncodedSessionTurnCheckpoint::MarkRetryableInterruption {
            expected_generation,
            turn_id,
            ..
        }
        | EncodedSessionTurnCheckpoint::Resume {
            expected_generation,
            turn_id,
            ..
        }
        | EncodedSessionTurnCheckpoint::Settle {
            expected_generation,
            turn_id,
            ..
        } => (*expected_generation, turn_id),
    };
    if catalog.lifecycle_generation != expected_generation {
        return Err(SessionTurnRejected(SessionTurnRefusal::GenerationChanged).into());
    }
    let node_id = public_turn_node_id(session_id, turn_id)?;
    let next_order: i64 = sqlx::query_scalar("SELECT CAST(GREATEST(COALESCE(MAX(created_order), 0), COALESCE(MAX(updated_order), 0)) + 1 AS SIGNED) FROM session_catalog")
        .fetch_one(&mut **transaction)
        .await?;
    ensure!(next_order > 0, "session catalog order overflowed");

    match checkpoint {
        EncodedSessionTurnCheckpoint::Admit {
            label,
            expected_transcript_rows,
            user_entry,
            ..
        } => {
            if let Some(expected_rows) = expected_transcript_rows {
                let actual_rows: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE namespace = ?")
                        .bind(namespace.as_bytes())
                        .fetch_one(&mut **transaction)
                        .await?;
                if u64::try_from(actual_rows)? != *expected_rows {
                    return Err(SessionTurnRejected(SessionTurnRefusal::TranscriptChanged).into());
                }
            }
            let mut predecessor_node_id = catalog.head_node_id.clone();
            let mut superseded_pending = None;
            if let Some(pending_node_id) = catalog.pending_node_id.as_deref() {
                let pending = load_public_turn(transaction, pending_node_id)
                    .await?
                    .ok_or(SessionTurnRejected(SessionTurnRefusal::PendingTurnMissing))?;
                ensure!(
                    pending.settlement == PublicTurnSettlement::Pending
                        && pending.origin_session_id == session_id,
                    "session catalog pending turn coordinates are invalid"
                );
                match pending.kind {
                    PublicTurnKind::Primary if !pending.terminal_entries.is_empty() => {
                        let updated = sqlx::query("UPDATE session_public_turns SET settlement = 'interrupted' WHERE node_id = ? AND settlement = 'pending'")
                            .bind(&pending.node_id)
                            .execute(&mut **transaction)
                            .await?;
                        ensure!(
                            updated.rows_affected() == 1,
                            SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
                        );
                        predecessor_node_id = Some(pending.node_id.clone());
                        superseded_pending = Some(pending.node_id);
                    }
                    PublicTurnKind::Continuation | PublicTurnKind::LegacyContinuation
                        if pending.terminal_entries.is_empty() =>
                    {
                        superseded_pending = Some(pending.node_id);
                    }
                    _ => {
                        return Err(
                            SessionTurnRejected(SessionTurnRefusal::PendingTurnExists).into()
                        );
                    }
                }
            }
            if label
                .as_ref()
                .is_some_and(|label| !catalog.label.is_empty() && label != &catalog.label)
            {
                return Err(SessionTurnRejected(SessionTurnRefusal::LabelChanged).into());
            }
            if load_public_turn(transaction, &node_id).await?.is_some() {
                return Err(SessionTurnRejected(SessionTurnRefusal::TurnAlreadyExists).into());
            }
            let record = PublicTurnRecord {
                node_id: node_id.clone(),
                origin_session_id: session_id.into(),
                turn_id: turn_id.clone(),
                kind: PublicTurnKind::Primary,
                continuation_of_node_id: None,
                predecessor_node_id: predecessor_node_id.clone(),
                settlement: PublicTurnSettlement::Pending,
                user_entry: Some(
                    serde_json::from_str(user_entry)
                        .context("encoded public user entry is malformed")?,
                ),
                speaker_id: None,
                terminal_entries: vec![],
                record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
            };
            validate_public_turn(&record)?;
            sqlx::query("INSERT INTO session_public_turns (node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format) VALUES (?, ?, ?, 'primary', NULL, ?, 'pending', ?, NULL, NULL, ?)")
                .bind(&record.node_id)
                .bind(record.origin_session_id.as_bytes())
                .bind(record.turn_id.as_bytes())
                .bind(record.predecessor_node_id.as_deref())
                .bind(user_entry)
                .bind(&record.record_format)
                .execute(&mut **transaction)
                .await?;
            let updated = if let Some(superseded_pending) = superseded_pending {
                sqlx::query("UPDATE session_catalog SET label = COALESCE(?, label), head_node_id = ?, pending_node_id = ?, updated_order = ? WHERE session_id = ? AND lifecycle_generation = ? AND lifecycle_state = 'active' AND pending_node_id = ?")
                    .bind(label.as_deref())
                    .bind(predecessor_node_id.as_deref())
                    .bind(&node_id)
                    .bind(next_order)
                    .bind(session_id.as_bytes())
                    .bind(i64::try_from(expected_generation)?)
                    .bind(&superseded_pending)
                    .execute(&mut **transaction)
                    .await?
            } else {
                sqlx::query("UPDATE session_catalog SET label = COALESCE(?, label), pending_node_id = ?, updated_order = ? WHERE session_id = ? AND lifecycle_generation = ? AND lifecycle_state = 'active' AND pending_node_id IS NULL")
                    .bind(label.as_deref())
                    .bind(&node_id)
                    .bind(next_order)
                    .bind(session_id.as_bytes())
                    .bind(i64::try_from(expected_generation)?)
                    .execute(&mut **transaction)
                    .await?
            };
            ensure!(
                updated.rows_affected() == 1,
                SessionTurnRejected(SessionTurnRefusal::GenerationChanged)
            );
        }
        EncodedSessionTurnCheckpoint::MarkRetryableInterruption {
            speaker_id,
            terminal_entries,
            ..
        } => {
            let pending = catalog
                .pending_node_id
                .as_deref()
                .ok_or(SessionTurnRejected(SessionTurnRefusal::PendingTurnMissing))?;
            let mut record = load_public_turn(transaction, pending)
                .await?
                .ok_or(SessionTurnRejected(SessionTurnRefusal::PendingTurnMissing))?;
            if record.settlement != PublicTurnSettlement::Pending {
                return Err(SessionTurnRejected(SessionTurnRefusal::TurnAlreadySettled).into());
            }
            if !record.terminal_entries.is_empty() || record.speaker_id.is_some() {
                return Err(SessionTurnRejected(
                    SessionTurnRefusal::RetryableInterruptionAlreadyMarked,
                )
                .into());
            }
            ensure!(
                record.origin_session_id == session_id
                    && record.turn_id == *turn_id
                    && record.predecessor_node_id == catalog.head_node_id,
                "pending public turn no longer matches its catalog boundary"
            );
            ensure!(
                record.kind == PublicTurnKind::Primary && record.node_id == node_id,
                SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
            );
            record.speaker_id = Some(speaker_id.clone());
            record.terminal_entries = serde_json::from_str(terminal_entries)
                .context("encoded retryable interruption entries are malformed")?;
            validate_public_turn(&record)?;
            let updated = sqlx::query("UPDATE session_public_turns SET speaker_id = ?, terminal_entries = ? WHERE node_id = ? AND settlement = 'pending' AND speaker_id IS NULL AND terminal_entries IS NULL")
                .bind(speaker_id.as_bytes())
                .bind(terminal_entries)
                .bind(&record.node_id)
                .execute(&mut **transaction)
                .await?;
            ensure!(
                updated.rows_affected() == 1,
                SessionTurnRejected(SessionTurnRefusal::RetryableInterruptionAlreadyMarked)
            );
            let updated = sqlx::query("UPDATE session_catalog SET updated_order = ? WHERE session_id = ? AND lifecycle_generation = ? AND lifecycle_state = 'active' AND pending_node_id = ?")
                .bind(next_order)
                .bind(session_id.as_bytes())
                .bind(i64::try_from(expected_generation)?)
                .bind(&record.node_id)
                .execute(&mut **transaction)
                .await?;
            ensure!(
                updated.rows_affected() == 1,
                SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
            );
        }
        EncodedSessionTurnCheckpoint::Resume { legacy, .. } => {
            let primary = load_public_turn(transaction, &node_id).await?;
            let continuation = match primary {
                Some(primary) => {
                    ensure!(
                        primary.kind == PublicTurnKind::Primary
                            && primary.origin_session_id == session_id
                            && primary.turn_id == *turn_id,
                        "resumed public turn does not match its primary identity"
                    );
                    match primary.settlement {
                        PublicTurnSettlement::Pending => {
                            ensure!(
                                catalog.pending_node_id.as_deref()
                                    == Some(primary.node_id.as_str()),
                                SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
                            );
                            None
                        }
                        PublicTurnSettlement::Interrupted => Some((
                            public_turn_continuation_node_id(session_id, turn_id)?,
                            PublicTurnKind::Continuation,
                            Some(primary.node_id),
                        )),
                        PublicTurnSettlement::Completed => {
                            return Err(SessionTurnRejected(
                                SessionTurnRefusal::TurnAlreadySettled,
                            )
                            .into());
                        }
                    }
                }
                None => {
                    let legacy = legacy
                        .as_ref()
                        .ok_or(SessionTurnRejected(SessionTurnRefusal::PendingTurnMissing))?;
                    validate_legacy_resume_binding(
                        transaction,
                        namespace,
                        session_id,
                        values,
                        catalog.legacy_prefix.as_ref(),
                        legacy,
                    )
                    .await?;
                    Some((
                        public_turn_legacy_continuation_node_id(session_id, turn_id)?,
                        PublicTurnKind::LegacyContinuation,
                        None,
                    ))
                }
            };
            let active_node_id = if let Some((continuation_id, kind, original)) = continuation {
                if catalog
                    .pending_node_id
                    .as_deref()
                    .is_some_and(|pending| pending != continuation_id)
                {
                    return Err(SessionTurnRejected(SessionTurnRefusal::PendingTurnExists).into());
                }
                let existing = load_public_turn(transaction, &continuation_id).await?;
                if let Some(mut continuation) = existing {
                    ensure!(
                        continuation.kind == kind
                            && continuation.settlement == PublicTurnSettlement::Pending
                            && continuation.continuation_of_node_id == original
                            && continuation.terminal_entries.is_empty(),
                        SessionTurnRejected(SessionTurnRefusal::TurnAlreadySettled)
                    );
                    continuation.predecessor_node_id = catalog.head_node_id.clone();
                    validate_public_turn(&continuation)?;
                    let updated = sqlx::query("UPDATE session_public_turns SET predecessor_node_id = ? WHERE node_id = ? AND settlement = 'pending'")
                        .bind(continuation.predecessor_node_id.as_deref())
                        .bind(&continuation_id)
                        .execute(&mut **transaction)
                        .await?;
                    ensure!(
                        updated.rows_affected() == 1,
                        SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
                    );
                } else {
                    let continuation = PublicTurnRecord {
                        node_id: continuation_id.clone(),
                        origin_session_id: session_id.into(),
                        turn_id: turn_id.clone(),
                        kind,
                        continuation_of_node_id: original,
                        predecessor_node_id: catalog.head_node_id.clone(),
                        settlement: PublicTurnSettlement::Pending,
                        user_entry: None,
                        speaker_id: None,
                        terminal_entries: vec![],
                        record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
                    };
                    validate_public_turn(&continuation)?;
                    sqlx::query("INSERT INTO session_public_turns (node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format) VALUES (?, ?, ?, ?, ?, ?, 'pending', NULL, NULL, NULL, ?)")
                        .bind(&continuation.node_id)
                        .bind(continuation.origin_session_id.as_bytes())
                        .bind(continuation.turn_id.as_bytes())
                        .bind(match kind {
                            PublicTurnKind::Continuation => "continuation",
                            PublicTurnKind::LegacyContinuation => "legacy",
                            PublicTurnKind::Primary => unreachable!("primary cannot be admitted as a continuation"),
                        })
                        .bind(continuation.continuation_of_node_id.as_deref())
                        .bind(continuation.predecessor_node_id.as_deref())
                        .bind(&continuation.record_format)
                        .execute(&mut **transaction)
                        .await?;
                }
                continuation_id
            } else {
                node_id.clone()
            };
            let updated = sqlx::query("UPDATE session_catalog SET pending_node_id = ?, updated_order = ? WHERE session_id = ? AND lifecycle_generation = ? AND lifecycle_state = 'active' AND (pending_node_id IS NULL OR pending_node_id = ?)")
                .bind(&active_node_id)
                .bind(next_order)
                .bind(session_id.as_bytes())
                .bind(i64::try_from(expected_generation)?)
                .bind(&active_node_id)
                .execute(&mut **transaction)
                .await?;
            ensure!(
                updated.rows_affected() == 1,
                SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
            );
        }
        EncodedSessionTurnCheckpoint::Settle {
            settlement,
            speaker_id,
            terminal_entries,
            ..
        } => {
            let pending = catalog
                .pending_node_id
                .as_deref()
                .ok_or(SessionTurnRejected(SessionTurnRefusal::PendingTurnMissing))?;
            let mut record = load_public_turn(transaction, pending)
                .await?
                .ok_or(SessionTurnRejected(SessionTurnRefusal::PendingTurnMissing))?;
            if record.settlement != PublicTurnSettlement::Pending {
                return Err(SessionTurnRejected(SessionTurnRefusal::TurnAlreadySettled).into());
            }
            ensure!(
                record.origin_session_id == session_id
                    && record.turn_id == *turn_id
                    && record.predecessor_node_id == catalog.head_node_id,
                "pending public turn no longer matches its catalog boundary"
            );
            ensure!(
                record.node_id == node_id
                    || record.node_id == public_turn_continuation_node_id(session_id, turn_id)?
                    || record.node_id
                        == public_turn_legacy_continuation_node_id(session_id, turn_id)?,
                SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
            );
            let mut appended_entries: Vec<Message> = serde_json::from_str(terminal_entries)
                .context("encoded public terminal entries are malformed")?;
            record.settlement = *settlement;
            record.speaker_id = Some(speaker_id.clone());
            record.terminal_entries.append(&mut appended_entries);
            validate_public_turn(&record)?;
            let combined_terminal_entries = serde_json::to_string(&record.terminal_entries)?;
            let updated = sqlx::query("UPDATE session_public_turns SET settlement = ?, speaker_id = ?, terminal_entries = ? WHERE node_id = ? AND settlement = 'pending'")
                .bind(match settlement {
                    PublicTurnSettlement::Completed => "completed",
                    PublicTurnSettlement::Interrupted => "interrupted",
                    PublicTurnSettlement::Pending => {
                        bail!("public turn settlement cannot remain pending")
                    }
                })
                .bind(speaker_id.as_bytes())
                .bind(&combined_terminal_entries)
                .bind(&record.node_id)
                .execute(&mut **transaction)
                .await?;
            ensure!(
                updated.rows_affected() == 1,
                SessionTurnRejected(SessionTurnRefusal::TurnAlreadySettled)
            );
            let updated = sqlx::query("UPDATE session_catalog SET head_node_id = ?, pending_node_id = NULL, updated_order = ? WHERE session_id = ? AND lifecycle_generation = ? AND lifecycle_state = 'active' AND pending_node_id = ?")
                .bind(&record.node_id)
                .bind(next_order)
                .bind(session_id.as_bytes())
                .bind(i64::try_from(expected_generation)?)
                .bind(&record.node_id)
                .execute(&mut **transaction)
                .await?;
            ensure!(
                updated.rows_affected() == 1,
                SessionTurnRejected(SessionTurnRefusal::PendingTurnChanged)
            );
        }
    }
    Ok(())
}

async fn validate_legacy_resume_binding(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    namespace: &str,
    session_id: &str,
    values: &[(String, String)],
    catalog_prefix: Option<&LegacyTranscriptPrefix>,
    proof: &LegacySessionTurnResume,
) -> Result<()> {
    ensure!(
        catalog_prefix == Some(&proof.legacy_prefix) && proof.legacy_prefix.namespace == namespace,
        SessionTurnRejected(SessionTurnRefusal::LegacyPrefixChanged)
    );
    let transcript_suffix = format!("/transcript/{session_id}");
    let scope = namespace
        .strip_suffix(&transcript_suffix)
        .ok_or(SessionTurnRejected(
            SessionTurnRefusal::LegacyJournalChanged,
        ))?;
    let journal_prefix = format!("{scope}/session/{session_id}/turn/");
    let correlation =
        proof
            .journal_key
            .strip_prefix(&journal_prefix)
            .ok_or(SessionTurnRejected(
                SessionTurnRefusal::LegacyJournalChanged,
            ))?;
    ensure!(
        correlation.len() == 64
            && correlation
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        SessionTurnRejected(SessionTurnRefusal::LegacyJournalChanged)
    );
    ensure!(
        matches!(values, [(key, _)] if key == &proof.journal_key),
        SessionTurnRejected(SessionTurnRefusal::LegacyJournalChanged)
    );
    let expected = serde_json::to_string(&proof.expected_journal)?;
    let retained: Option<String> =
        sqlx::query_scalar("SELECT value FROM state WHERE BINARY `key` = BINARY ? FOR UPDATE")
            .bind(proof.journal_key.as_bytes())
            .fetch_optional(&mut **transaction)
            .await?;
    ensure!(
        retained.as_deref() == Some(expected.as_str()),
        SessionTurnRejected(SessionTurnRefusal::LegacyJournalChanged)
    );
    Ok(())
}

fn checked_session_lifecycle_record(
    existing: Option<SessionCatalogRecord>,
    expected_generation: u64,
) -> Result<SessionCatalogRecord> {
    let record = existing.ok_or(SessionLifecycleRejected(SessionLifecycleRefusal::Missing))?;
    if record.lifecycle_generation != expected_generation {
        return Err(SessionLifecycleRejected(SessionLifecycleRefusal::GenerationChanged).into());
    }
    Ok(record)
}

async fn update_session_lifecycle_row(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    record: &SessionCatalogRecord,
    expected_generation: u64,
) -> Result<()> {
    let updated = sqlx::query("UPDATE session_catalog SET label = ?, updated_order = ?, lifecycle_generation = ?, lifecycle_state = ? WHERE session_id = ? AND lifecycle_generation = ?")
        .bind(&record.label)
        .bind(i64::try_from(record.updated_order)?)
        .bind(i64::try_from(record.lifecycle_generation)?)
        .bind(session_lifecycle_sql(record.lifecycle_state))
        .bind(record.session_id.as_bytes())
        .bind(i64::try_from(expected_generation)?)
        .execute(&mut **transaction)
        .await?;
    ensure!(
        updated.rows_affected() == 1,
        SessionLifecycleRejected(SessionLifecycleRefusal::GenerationChanged)
    );
    Ok(())
}

fn session_lifecycle_outcome(record: &SessionCatalogRecord) -> Result<SessionLifecycleOutcome> {
    validate_session_catalog(record)?;
    let outcome = SessionLifecycleOutcome {
        session_id: record.session_id.clone(),
        lifecycle_generation: record.lifecycle_generation,
        lifecycle_state: record.lifecycle_state,
        updated_order: record.updated_order,
        record_format: SESSION_LIFECYCLE_OUTCOME_FORMAT.into(),
    };
    validate_session_lifecycle_outcome(&outcome)?;
    Ok(outcome)
}

fn encode_session_lifecycle_result_ref(outcome: &SessionLifecycleOutcome) -> Result<String> {
    validate_session_lifecycle_outcome(outcome)?;
    let result = format!(
        "session-lifecycle:v1:{}:{}:{}",
        outcome.lifecycle_generation,
        outcome.updated_order,
        session_lifecycle_sql(outcome.lifecycle_state)
    );
    ensure!(
        result.len() <= 256 && result.is_ascii(),
        "session lifecycle receipt result exceeds its durable bound"
    );
    Ok(result)
}

fn decode_session_lifecycle_result_ref(
    session_id: &str,
    result_ref: &str,
) -> Result<SessionLifecycleOutcome> {
    let mut fields = result_ref.split(':');
    ensure!(
        fields.next() == Some("session-lifecycle") && fields.next() == Some("v1"),
        "session lifecycle receipt result format is unsupported"
    );
    let lifecycle_generation = fields
        .next()
        .context("session lifecycle receipt omitted its generation")?
        .parse::<u64>()
        .context("session lifecycle receipt generation is invalid")?;
    let updated_order = fields
        .next()
        .context("session lifecycle receipt omitted its update order")?
        .parse::<u64>()
        .context("session lifecycle receipt update order is invalid")?;
    let lifecycle_state = match fields.next() {
        Some("active") => SessionLifecycleState::Active,
        Some("removed") => SessionLifecycleState::Removed,
        _ => bail!("session lifecycle receipt state is invalid"),
    };
    ensure!(
        fields.next().is_none(),
        "session lifecycle receipt result has trailing fields"
    );
    let outcome = SessionLifecycleOutcome {
        session_id: session_id.into(),
        lifecycle_generation,
        lifecycle_state,
        updated_order,
        record_format: SESSION_LIFECYCLE_OUTCOME_FORMAT.into(),
    };
    validate_session_lifecycle_outcome(&outcome)?;
    Ok(outcome)
}

async fn load_session_lifecycle_outcome(
    pool: &MySqlPool,
    operation: &str,
    session_id: &str,
) -> Result<SessionLifecycleOutcome> {
    let result_ref: Option<Option<String>> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT result_ref FROM operations WHERE id = ?")
            .bind(operation)
            .fetch_optional(pool),
    )
    .await
    .context("session lifecycle receipt result deadline exceeded")??;
    let result_ref = result_ref
        .flatten()
        .context("session lifecycle receipt has no typed result")?;
    decode_session_lifecycle_result_ref(session_id, &result_ref)
}

pub(crate) fn validate_session_lifecycle_outcome(outcome: &SessionLifecycleOutcome) -> Result<()> {
    ensure!(
        outcome.record_format == SESSION_LIFECYCLE_OUTCOME_FORMAT,
        "session lifecycle outcome format is unsupported"
    );
    session_identity(
        "session lifecycle outcome identity",
        &outcome.session_id,
        128,
    )?;
    ensure!(
        outcome.lifecycle_generation <= i64::MAX as u64,
        "session lifecycle outcome generation exceeds the store range"
    );
    ensure!(
        outcome.updated_order > 0 && outcome.updated_order <= i64::MAX as u64,
        "session lifecycle outcome order is invalid"
    );
    Ok(())
}

pub(crate) fn validate_session_lifecycle_input(
    session_id: &str,
    expected_generation: Option<u64>,
    label: Option<&str>,
) -> Result<()> {
    session_identity("session lifecycle identity", session_id, 128)?;
    if let Some(generation) = expected_generation {
        ensure!(
            generation <= i64::MAX as u64,
            "session lifecycle generation exceeds the store range"
        );
    }
    if let Some(label) = label {
        bounded_session_label("session label", label)?;
    }
    Ok(())
}

pub(crate) fn validate_session_fork_input(
    source_session_id: &str,
    expected_source_generation: u64,
    source_node_id: &str,
    child_session_id: &str,
    label: &str,
) -> Result<()> {
    validate_session_lifecycle_input(source_session_id, Some(expected_source_generation), None)?;
    validate_public_node_id("fork source node", source_node_id)?;
    validate_session_lifecycle_input(child_session_id, None, Some(label))?;
    ensure!(
        source_session_id != child_session_id,
        "session fork source and child must differ"
    );
    Ok(())
}

fn encode_state(values: &[(String, Value)]) -> Result<Vec<(String, String)>> {
    let mut keys = BTreeSet::new();
    let mut encoded = Vec::with_capacity(values.len());
    for (key, value) in values {
        identifier("state key", key, 1024)?;
        ensure!(keys.insert(key), "duplicate state key in atomic update");
        encoded.push((key.clone(), serde_json::to_string(value)?));
    }
    Ok(encoded)
}

pub fn public_turn_node_id(origin_session_id: &str, turn_id: &str) -> Result<String> {
    session_identity("origin session identity", origin_session_id, 128)?;
    session_identity("public turn identity", turn_id, 128)?;
    let mut digest = Sha256::new();
    hash_context_field(&mut digest, b"kuru.public-turn.node.v1");
    hash_context_field(&mut digest, origin_session_id.as_bytes());
    hash_context_field(&mut digest, turn_id.as_bytes());
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn public_turn_continuation_node_id(origin_session_id: &str, turn_id: &str) -> Result<String> {
    session_identity("origin session identity", origin_session_id, 128)?;
    session_identity("public turn identity", turn_id, 128)?;
    let mut digest = Sha256::new();
    hash_context_field(&mut digest, b"kuru.public-turn.continuation-node.v1");
    hash_context_field(&mut digest, origin_session_id.as_bytes());
    hash_context_field(&mut digest, turn_id.as_bytes());
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn public_turn_legacy_continuation_node_id(
    origin_session_id: &str,
    turn_id: &str,
) -> Result<String> {
    session_identity("origin session identity", origin_session_id, 128)?;
    session_identity("public turn identity", turn_id, 128)?;
    let mut digest = Sha256::new();
    hash_context_field(&mut digest, b"kuru.public-turn.legacy-continuation-node.v1");
    hash_context_field(&mut digest, origin_session_id.as_bytes());
    hash_context_field(&mut digest, turn_id.as_bytes());
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn validate_session_catalog(record: &SessionCatalogRecord) -> Result<()> {
    ensure!(
        record.record_format == SESSION_CATALOG_RECORD_FORMAT,
        "session catalog format is unsupported"
    );
    session_identity("session identity", &record.session_id, 128)?;
    bounded_session_label("session label", &record.label)?;
    ensure!(
        record.created_order > 0
            && record.created_order <= i64::MAX as u64
            && record.updated_order >= record.created_order
            && record.updated_order <= i64::MAX as u64,
        "session catalog ordering is invalid"
    );
    ensure!(
        record.lifecycle_generation <= i64::MAX as u64,
        "session lifecycle generation exceeds the store range"
    );
    for (field, node) in [
        ("session head node", record.head_node_id.as_deref()),
        ("session pending node", record.pending_node_id.as_deref()),
    ] {
        if let Some(node) = node {
            validate_public_node_id(field, node)?;
        }
    }
    ensure!(
        record.head_node_id != record.pending_node_id || record.head_node_id.is_none(),
        "session pending node must remain separate from its settled head"
    );
    if let Some(prefix) = &record.legacy_prefix {
        validate_legacy_prefix(prefix)?;
    }
    if let Some(fork) = &record.fork_provenance {
        validate_fork_provenance(fork)?;
        ensure!(
            record.head_node_id.is_some(),
            "a forked session requires its inherited settled head"
        );
    }
    Ok(())
}

pub(crate) fn validate_session_catalog_cursor(cursor: &SessionCatalogCursor) -> Result<()> {
    ensure!(
        cursor.updated_order > 0 && cursor.updated_order <= i64::MAX as u64,
        "session catalog cursor order is invalid"
    );
    session_identity("session catalog cursor identity", &cursor.session_id, 128)
}

pub(crate) fn validate_session_catalog_page(page: &SessionCatalogPage) -> Result<()> {
    session_identity("session catalog page view", &page.view, 64)?;
    validate_revision_identity("session catalog page revision", &page.revision)?;
    ensure!(
        page.records.len() <= MAX_SESSION_SOURCE_ROWS,
        "session catalog page exceeds the row bound"
    );
    for record in &page.records {
        validate_session_catalog(record)?;
        if let Some(state) = page.lifecycle_state {
            ensure!(
                record.lifecycle_state == state,
                "session catalog page contains a different lifecycle state"
            );
        }
    }
    if let Some(next) = &page.next {
        validate_session_catalog_cursor(next)?;
        let last = page
            .records
            .last()
            .context("session catalog continuation lacks a retained row")?;
        ensure!(
            next.updated_order == last.updated_order && next.session_id == last.session_id,
            "session catalog continuation does not match the retained last row"
        );
    }
    ensure!(
        serde_json::to_vec(page)?.len() <= MAX_SESSION_SOURCE_BYTES,
        "session catalog page exceeds the 32 MiB managed response bound"
    );
    Ok(())
}

pub(crate) fn validate_public_turn(record: &PublicTurnRecord) -> Result<()> {
    ensure!(
        record.record_format == PUBLIC_TURN_RECORD_FORMAT,
        "public turn format is unsupported"
    );
    session_identity("turn origin session", &record.origin_session_id, 128)?;
    session_identity("public turn identity", &record.turn_id, 128)?;
    validate_public_node_id("public turn node", &record.node_id)?;
    let expected_node_id = match record.kind {
        PublicTurnKind::Primary => {
            ensure!(
                record.continuation_of_node_id.is_none() && record.user_entry.is_some(),
                "primary public turn requires one user entry and no continuation reference"
            );
            public_turn_node_id(&record.origin_session_id, &record.turn_id)?
        }
        PublicTurnKind::Continuation => {
            let original = record
                .continuation_of_node_id
                .as_deref()
                .context("public continuation lacks its original node reference")?;
            validate_public_node_id("public continuation original node", original)?;
            ensure!(
                original == public_turn_node_id(&record.origin_session_id, &record.turn_id)?,
                "public continuation does not reference its exact same-session primary turn"
            );
            ensure!(
                record.user_entry.is_none(),
                "public continuation cannot contain another user entry"
            );
            public_turn_continuation_node_id(&record.origin_session_id, &record.turn_id)?
        }
        PublicTurnKind::LegacyContinuation => {
            ensure!(
                record.continuation_of_node_id.is_none() && record.user_entry.is_none(),
                "legacy public continuation cannot fabricate a primary reference or user entry"
            );
            public_turn_legacy_continuation_node_id(&record.origin_session_id, &record.turn_id)?
        }
    };
    ensure!(
        record.node_id == expected_node_id,
        "public turn node does not match its kind, origin and turn identity"
    );
    if let Some(predecessor) = &record.predecessor_node_id {
        validate_public_node_id("public turn predecessor", predecessor)?;
        ensure!(
            predecessor != &record.node_id,
            "public turn cannot name itself as predecessor"
        );
    }
    if let Some(user_entry) = &record.user_entry {
        validate_public_message(user_entry)?;
        ensure!(
            user_entry.role == "user",
            "public turn user entry must retain the user role"
        );
    }
    for entry in &record.terminal_entries {
        validate_public_message(entry)?;
        ensure!(
            entry.role != "user",
            "public terminal entry cannot claim the user role"
        );
    }
    match record.settlement {
        PublicTurnSettlement::Pending => {
            ensure!(
                record.speaker_id.is_some() == !record.terminal_entries.is_empty(),
                "pending public turn must contain both or neither retryable interruption speaker and entries"
            );
            if let Some(speaker) = record.speaker_id.as_deref() {
                session_identity("pending interruption speaker", speaker, 1024)?;
            }
        }
        PublicTurnSettlement::Completed | PublicTurnSettlement::Interrupted => {
            let speaker = record
                .speaker_id
                .as_deref()
                .context("settled public turn lacks stable speaker identity")?;
            session_identity("public turn speaker", speaker, 1024)?;
            ensure!(
                !record.terminal_entries.is_empty()
                    || (matches!(
                        record.kind,
                        PublicTurnKind::Continuation | PublicTurnKind::LegacyContinuation
                    ) && record.settlement == PublicTurnSettlement::Interrupted),
                "settled primary/completed public turn lacks terminal entries"
            );
        }
    }
    ensure!(
        serde_json::to_vec(record)?.len() <= MAX_SESSION_SOURCE_BYTES,
        "public turn exceeds the 32 MiB managed row bound"
    );
    Ok(())
}

pub(crate) fn validate_public_transcript_cursor(cursor: &PublicTranscriptCursor) -> Result<()> {
    session_identity("public transcript cursor session", &cursor.session_id, 128)?;
    validate_revision_identity("public transcript cursor revision", &cursor.revision)?;
    if let Some(head) = &cursor.head_node_id {
        validate_public_node_id("public transcript cursor head", head)?;
    }
    match &cursor.next {
        PublicTranscriptPosition::Turn { node_id } => {
            validate_public_node_id("public transcript cursor node", node_id)?
        }
        PublicTranscriptPosition::Legacy { sequence } => ensure!(
            *sequence > 0,
            "public transcript legacy cursor must be positive"
        ),
    }
    Ok(())
}

pub(crate) fn validate_public_transcript_page(page: &PublicTranscriptPage) -> Result<()> {
    session_identity("public transcript page session", &page.session_id, 128)?;
    session_identity("public transcript page view", &page.view, 64)?;
    validate_revision_identity("public transcript page revision", &page.revision)?;
    if let Some(head) = &page.head_node_id {
        validate_public_node_id("public transcript page head", head)?;
    }
    if let Some(pending) = &page.pending {
        validate_public_turn(pending)?;
        ensure!(
            pending.settlement == PublicTurnSettlement::Pending,
            "public transcript pending row is already settled"
        );
    }
    ensure!(
        page.records.len() <= MAX_SESSION_SOURCE_ROWS,
        "public transcript page exceeds the row bound"
    );
    for entry in &page.records {
        match entry {
            PublicTranscriptEntry::Turn { record } => {
                validate_public_turn(record)?;
                ensure!(
                    record.settlement != PublicTurnSettlement::Pending,
                    "public transcript settled page contains a pending turn"
                );
            }
            PublicTranscriptEntry::Legacy { sequence, message } => {
                ensure!(*sequence > 0, "legacy transcript sequence must be positive");
                validate_public_message(message)?;
            }
        }
    }
    if let Some(next) = &page.next {
        validate_public_transcript_cursor(next)?;
        ensure!(
            next.session_id == page.session_id
                && next.revision == page.revision
                && next.head_node_id == page.head_node_id,
            "public transcript continuation changed its captured coordinates"
        );
    }
    ensure!(
        serde_json::to_vec(page)?.len() <= MAX_SESSION_SOURCE_BYTES,
        "public transcript page exceeds the 32 MiB managed response bound"
    );
    Ok(())
}

fn validate_legacy_prefix(prefix: &LegacyTranscriptPrefix) -> Result<()> {
    ensure!(
        prefix.record_format == LEGACY_PREFIX_RECORD_FORMAT,
        "legacy transcript prefix format is unsupported"
    );
    session_identity("legacy transcript namespace", &prefix.namespace, 1024)?;
    session_identity(
        "legacy transcript source session",
        &prefix.source_session_id,
        128,
    )?;
    validate_revision_identity("legacy transcript revision", &prefix.source_revision)?;
    ensure!(
        prefix.row_count > 0
            && prefix.row_count <= i64::MAX as u64
            && prefix.first_sequence <= prefix.through_sequence,
        "legacy transcript prefix range is invalid"
    );
    Ok(())
}

fn validate_fork_provenance(fork: &SessionForkProvenance) -> Result<()> {
    ensure!(
        fork.record_format == FORK_PROVENANCE_RECORD_FORMAT,
        "session fork provenance format is unsupported"
    );
    session_identity("fork source session", &fork.source_session_id, 128)?;
    validate_public_node_id("fork source node", &fork.source_node_id)?;
    session_identity("fork source turn", &fork.source_turn_id, 128)?;
    bounded_session_label("fork source label", &fork.source_label)?;
    ensure!(
        fork.shares_current_project_memory,
        "session fork must disclose shared current project memory"
    );
    Ok(())
}

fn session_lifecycle_sql(state: SessionLifecycleState) -> &'static str {
    match state {
        SessionLifecycleState::Active => "active",
        SessionLifecycleState::Removed => "removed",
    }
}

fn decode_session_catalog_row(row: &sqlx::mysql::MySqlRow) -> Result<SessionCatalogRecord> {
    let mode: String = row.try_get("mode")?;
    let lifecycle_state: String = row.try_get("lifecycle_state")?;
    let created_order: i64 = row.try_get("created_order")?;
    let updated_order: i64 = row.try_get("updated_order")?;
    let lifecycle_generation: i64 = row.try_get("lifecycle_generation")?;
    let record = SessionCatalogRecord {
        session_id: String::from_utf8(row.try_get::<Vec<u8>, _>("session_id")?)
            .context("stored session identity is not UTF-8")?,
        mode: serde_json::from_value(Value::String(mode))
            .context("stored session mode is unsupported")?,
        label: row.try_get("label")?,
        created_order: u64::try_from(created_order)
            .context("stored session creation order is negative")?,
        updated_order: u64::try_from(updated_order)
            .context("stored session update order is negative")?,
        lifecycle_generation: u64::try_from(lifecycle_generation)
            .context("stored session lifecycle generation is negative")?,
        lifecycle_state: match lifecycle_state.as_str() {
            "active" => SessionLifecycleState::Active,
            "removed" => SessionLifecycleState::Removed,
            _ => bail!("stored session lifecycle state is unsupported"),
        },
        head_node_id: row.try_get("head_node_id")?,
        pending_node_id: row.try_get("pending_node_id")?,
        legacy_prefix: row
            .try_get::<Option<String>, _>("legacy_prefix")?
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .context("stored legacy transcript prefix is malformed")?,
        fork_provenance: row
            .try_get::<Option<String>, _>("fork_provenance")?
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .context("stored session fork provenance is malformed")?,
        record_format: row.try_get("record_format")?,
    };
    validate_session_catalog(&record)?;
    Ok(record)
}

async fn load_public_turn(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    node_id: &str,
) -> Result<Option<PublicTurnRecord>> {
    validate_public_node_id("public transcript node", node_id)?;
    let row = sqlx::query("SELECT node_id, origin_session_id, turn_id, record_kind, continuation_of_node_id, predecessor_node_id, settlement, user_entry, speaker_id, terminal_entries, record_format FROM session_public_turns WHERE node_id = ?")
        .bind(node_id)
        .fetch_optional(&mut **transaction)
        .await?;
    row.as_ref().map(decode_public_turn_row).transpose()
}

fn decode_public_turn_row(row: &sqlx::mysql::MySqlRow) -> Result<PublicTurnRecord> {
    let settlement: String = row.try_get("settlement")?;
    let record_kind: String = row.try_get("record_kind")?;
    let user_entry: Option<String> = row.try_get("user_entry")?;
    let terminal_entries: Option<String> = row.try_get("terminal_entries")?;
    let record = PublicTurnRecord {
        node_id: row.try_get("node_id")?,
        origin_session_id: String::from_utf8(row.try_get::<Vec<u8>, _>("origin_session_id")?)
            .context("stored public turn origin session is not UTF-8")?,
        turn_id: String::from_utf8(row.try_get::<Vec<u8>, _>("turn_id")?)
            .context("stored public turn identity is not UTF-8")?,
        kind: match record_kind.as_str() {
            "primary" => PublicTurnKind::Primary,
            "continuation" => PublicTurnKind::Continuation,
            "legacy" => PublicTurnKind::LegacyContinuation,
            _ => bail!("stored public turn kind is unsupported"),
        },
        continuation_of_node_id: row.try_get("continuation_of_node_id")?,
        predecessor_node_id: row.try_get("predecessor_node_id")?,
        settlement: match settlement.as_str() {
            "pending" => PublicTurnSettlement::Pending,
            "completed" => PublicTurnSettlement::Completed,
            "interrupted" => PublicTurnSettlement::Interrupted,
            _ => bail!("stored public turn settlement is unsupported"),
        },
        user_entry: user_entry
            .map(|entry| serde_json::from_str(&entry))
            .transpose()
            .context("stored public user entry is malformed")?,
        speaker_id: row
            .try_get::<Option<Vec<u8>>, _>("speaker_id")?
            .map(String::from_utf8)
            .transpose()
            .context("stored public turn speaker is not UTF-8")?,
        terminal_entries: terminal_entries
            .map(|entries| serde_json::from_str(&entries))
            .transpose()
            .context("stored public terminal entries are malformed")?
            .unwrap_or_default(),
        record_format: row.try_get("record_format")?,
    };
    validate_public_turn(&record)?;
    Ok(record)
}

fn validate_public_message(message: &Message) -> Result<()> {
    identifier("public message role", &message.role, 128)?;
    ensure!(
        message
            .blocks
            .iter()
            .all(|block| !matches!(block, ContentBlock::ReasoningSummary { .. })),
        "private reasoning summaries cannot enter the public transcript"
    );
    encode_typed_message(message).map(drop)
}

fn public_transcript_entry_position(entry: &PublicTranscriptEntry) -> PublicTranscriptPosition {
    match entry {
        PublicTranscriptEntry::Turn { record } => PublicTranscriptPosition::Turn {
            node_id: record.node_id.clone(),
        },
        PublicTranscriptEntry::Legacy { sequence, .. } => PublicTranscriptPosition::Legacy {
            sequence: *sequence,
        },
    }
}

fn bounded_session_label(label: &str, value: &str) -> Result<()> {
    ensure!(
        value.len() <= MAX_SESSION_LABEL_BYTES,
        "{label} exceeds {MAX_SESSION_LABEL_BYTES} bytes"
    );
    ensure!(!value.contains('\0'), "{label} must not contain NUL");
    Ok(())
}

pub(crate) fn session_identity(label: &str, value: &str, maximum: usize) -> Result<()> {
    identifier(label, value, maximum)?;
    ensure!(
        !value.chars().any(char::is_control),
        "{label} must not contain control characters"
    );
    Ok(())
}

fn validate_public_node_id(label: &str, value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} is malformed"
    );
    Ok(())
}

pub(crate) fn validate_revision_identity(label: &str, value: &str) -> Result<()> {
    session_identity(label, value, 64)
}

fn encode_reasoning_summaries(records: &[ReasoningSummaryRecord]) -> Result<Vec<(String, String)>> {
    validate_reasoning_summaries(records)?;
    Ok(records
        .iter()
        .map(|record| {
            Ok((
                reasoning_summary_key(record)?,
                serde_json::to_string(record)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?
        .into_iter()
        .collect())
}

/// Validate one complete private-summary mutation before an attachment sends
/// it. The owner invokes the same validation again because the RPC boundary is
/// untrusted; callers use it to reject definite no-effect inputs without
/// turning a local serialization failure into an uncertain remote write.
pub(crate) fn validate_reasoning_summaries(records: &[ReasoningSummaryRecord]) -> Result<()> {
    ensure!(
        (1..=1024).contains(&records.len()),
        "private reasoning summary batch must contain 1–1024 records"
    );
    let mut string_bytes = 0usize;
    let mut identities = BTreeMap::new();
    for record in records {
        ensure!(
            record.turn_id.is_some() ^ record.operation_id.is_some(),
            "private reasoning summary requires exactly one turn or operation identity"
        );
        for (field, value, limit) in [
            ("reasoning summary session", &record.session_id, 1024),
            ("reasoning summary actor", &record.actor_id, 1024),
            ("reasoning summary invocation", &record.invocation_id, 1024),
            (
                "reasoning summary text",
                &record.text,
                MAX_TYPED_MESSAGE_BYTES,
            ),
        ] {
            identifier(field, value, limit)?;
            string_bytes = string_bytes
                .checked_add(value.len())
                .context("private reasoning summary aggregate byte count overflow")?;
        }
        for (field, value) in [
            ("reasoning summary turn", record.turn_id.as_deref()),
            (
                "reasoning summary operation",
                record.operation_id.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                identifier(field, value, 1024)?;
                string_bytes = string_bytes
                    .checked_add(value.len())
                    .context("private reasoning summary aggregate byte count overflow")?;
            }
        }
        if let Some(item_id) = &record.item_id {
            identifier("reasoning summary item", item_id, 1024)?;
            string_bytes = string_bytes
                .checked_add(item_id.len())
                .context("private reasoning summary aggregate byte count overflow")?;
        }
        ensure!(
            string_bytes <= MAX_REASONING_SUMMARY_BATCH_STRING_BYTES,
            "private reasoning summary batch strings exceed 16 MiB"
        );
        let key = reasoning_summary_key(record)?;
        let value = serde_json::to_string(record)?;
        if let Some(existing) = identities.insert(key, value.clone()) {
            ensure!(existing == value, ReasoningSummaryConflict);
        }
    }
    Ok(())
}

pub(crate) fn reasoning_summary_key(record: &ReasoningSummaryRecord) -> Result<String> {
    let (prefix, encoded) = match (&record.turn_id, &record.operation_id) {
        (Some(turn_id), None) => (
            PRIVATE_REASONING_SUMMARY_PREFIX,
            serde_json::to_vec(&(
                &record.session_id,
                turn_id,
                &record.actor_id,
                &record.invocation_id,
                &record.item_id,
                record.output_index,
                record.summary_index,
            ))?,
        ),
        (None, Some(operation_id)) => (
            PRIVATE_REASONING_SUMMARY_OPERATION_PREFIX,
            serde_json::to_vec(&(
                "kuru.private-reasoning-summary.operation.v2",
                &record.session_id,
                operation_id,
                &record.actor_id,
                &record.invocation_id,
                &record.item_id,
                record.output_index,
                record.summary_index,
            ))?,
        ),
        _ => bail!("private reasoning summary requires exactly one turn or operation identity"),
    };
    let digest = Sha256::digest(encoded);
    Ok(format!(
        "{prefix}{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[derive(Serialize)]
struct TypedMessageRef<'a> {
    blocks: &'a [ContentBlock],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedMessage {
    blocks: Vec<ContentBlock>,
}

pub(crate) fn validate_session_message(
    namespace: &str,
    session_id: &str,
    message: &Message,
) -> Result<()> {
    identifier("namespace", namespace, 1024)?;
    identifier("session identity", session_id, 128)?;
    identifier("role", &message.role, 128)?;
    encode_typed_message(message)?;
    Ok(())
}

pub(crate) fn validate_session_source_request(
    actor_namespace: &str,
    session_id: &str,
    source_namespace: &str,
    after_exclusive: i64,
    limit: usize,
) -> Result<()> {
    identifier("actor namespace", actor_namespace, 1024)?;
    identifier("session identity", session_id, 128)?;
    identifier("source namespace", source_namespace, 1024)?;
    ensure!(after_exclusive >= 0, "source cursor cannot be negative");
    ensure!(
        (1..=MAX_SESSION_SOURCE_ROWS).contains(&limit),
        "source snapshot limit must be between 1 and {MAX_SESSION_SOURCE_ROWS}"
    );
    Ok(())
}

pub(crate) fn validate_session_history_request(
    namespace: &str,
    session_id: &str,
    limit: usize,
) -> Result<()> {
    identifier("namespace", namespace, 1024)?;
    identifier("session identity", session_id, 128)?;
    ensure!(
        limit <= MAX_SESSION_SOURCE_ROWS,
        "session history limit cannot exceed {MAX_SESSION_SOURCE_ROWS}"
    );
    Ok(())
}

pub(crate) fn validate_session_history_after_request(
    namespace: &str,
    session_id: &str,
    after_exclusive: i64,
    limit: usize,
) -> Result<()> {
    validate_session_history_request(namespace, session_id, limit)?;
    ensure!(
        after_exclusive >= 0,
        "session history cursor cannot be negative"
    );
    Ok(())
}

pub(crate) fn validate_session_checkpoint(
    namespace: &str,
    session_id: &str,
    messages: &[Message],
    values: &[(String, Value)],
) -> Result<()> {
    identifier("namespace", namespace, 1024)?;
    identifier("session identity", session_id, 128)?;
    for message in messages {
        identifier("role", &message.role, 128)?;
        encode_typed_message(message)?;
    }
    encode_state(values)?;
    ensure!(
        !messages.is_empty() || !values.is_empty(),
        "memory checkpoint must contain a message or state value"
    );
    Ok(())
}

pub(crate) fn validate_session_mode_checkpoint(
    namespace: &str,
    session_id: &str,
    messages: &[Message],
    values: &[(String, Value)],
    checkpoint: &SessionModeCheckpoint,
) -> Result<()> {
    session_identity("mode checkpoint session", session_id, 128)?;
    ensure!(
        messages.is_empty(),
        "mode checkpoint cannot append a transcript entry"
    );
    ensure!(
        checkpoint.expected_generation <= i64::MAX as u64,
        "mode checkpoint generation exceeds the store range"
    );
    ensure!(
        checkpoint.expected_mode != checkpoint.mode,
        "mode checkpoint must change the mode"
    );
    let state_key = session_mode_state_key(namespace, session_id)?;
    let state = values
        .iter()
        .find(|(key, _)| key == &state_key)
        .context("mode checkpoint lacks the matching runtime session state")?;
    ensure!(
        state.1.get("id").and_then(Value::as_str) == Some(session_id),
        "mode checkpoint runtime session identity differs"
    );
    ensure!(
        state.1.get("mode") == Some(&serde_json::to_value(checkpoint.mode)?),
        "mode checkpoint runtime state has a different mode"
    );
    ensure!(
        state.1.get("lifecycle_generation").and_then(Value::as_u64)
            == Some(checkpoint.expected_generation),
        "mode checkpoint runtime state has a different lifecycle generation"
    );
    Ok(())
}

pub(crate) fn validate_session_turn_checkpoint(
    session_id: &str,
    messages: &[Message],
    checkpoint: &SessionTurnCheckpoint,
) -> Result<()> {
    session_identity("public turn session", session_id, 128)?;
    let (expected_generation, turn_id) = match checkpoint {
        SessionTurnCheckpoint::Admit {
            expected_generation,
            turn_id,
            ..
        }
        | SessionTurnCheckpoint::MarkRetryableInterruption {
            expected_generation,
            turn_id,
            ..
        }
        | SessionTurnCheckpoint::Resume {
            expected_generation,
            turn_id,
            ..
        }
        | SessionTurnCheckpoint::Settle {
            expected_generation,
            turn_id,
            ..
        } => (*expected_generation, turn_id),
    };
    ensure!(
        expected_generation <= i64::MAX as u64,
        "public turn generation exceeds the store range"
    );
    session_identity("public turn identity", turn_id, 128)?;
    match checkpoint {
        SessionTurnCheckpoint::Admit {
            label,
            expected_transcript_rows,
            ..
        } => {
            if let Some(label) = label {
                bounded_session_label("session admission label", label)?;
            }
            ensure!(
                expected_transcript_rows.is_none_or(|rows| rows < i64::MAX as u64),
                "admitted transcript row exceeds the store range"
            );
            ensure!(
                messages.len() == 1 && messages[0].role == "user",
                "public turn admission requires exactly one user entry"
            );
            validate_public_message(&messages[0])?;
        }
        SessionTurnCheckpoint::MarkRetryableInterruption { speaker_id, .. } => {
            session_identity("public turn interruption speaker", speaker_id, 1024)?;
            ensure!(
                messages.len() == 1 && messages[0].role != "user",
                "retryable interruption requires exactly one terminal marker"
            );
            validate_public_message(&messages[0])?;
        }
        SessionTurnCheckpoint::Resume { legacy, .. } => {
            ensure!(
                messages.is_empty(),
                "public turn resume cannot append another transcript entry"
            );
            if let Some(legacy) = legacy {
                validate_legacy_prefix(&legacy.legacy_prefix)?;
                identifier("legacy turn journal key", &legacy.journal_key, 1024)?;
                ensure!(
                    serde_json::to_vec(&legacy.expected_journal)?.len() <= MAX_SESSION_SOURCE_BYTES,
                    "legacy turn journal proof exceeds the 32 MiB managed row bound"
                );
            }
        }
        SessionTurnCheckpoint::Settle {
            settlement,
            speaker_id,
            ..
        } => {
            ensure!(
                *settlement != PublicTurnSettlement::Pending,
                "public turn settlement cannot remain pending"
            );
            session_identity("public turn speaker", speaker_id, 1024)?;
            ensure!(
                !messages.is_empty() || *settlement == PublicTurnSettlement::Interrupted,
                "completed public turn settlement requires terminal entries"
            );
            for message in messages {
                validate_public_message(message)?;
                ensure!(
                    message.role != "user",
                    "public terminal entry cannot claim the user role"
                );
            }
            ensure!(
                serde_json::to_vec(messages)?.len() <= MAX_SESSION_SOURCE_BYTES,
                "public terminal entries exceed the 32 MiB managed row bound"
            );
        }
    }
    Ok(())
}

fn encode_session_turn_checkpoint(
    checkpoint: &SessionTurnCheckpoint,
    messages: &[Message],
) -> Result<EncodedSessionTurnCheckpoint> {
    match checkpoint {
        SessionTurnCheckpoint::Admit {
            expected_generation,
            turn_id,
            label,
            expected_transcript_rows,
        } => Ok(EncodedSessionTurnCheckpoint::Admit {
            expected_generation: *expected_generation,
            turn_id: turn_id.clone(),
            label: label.clone(),
            expected_transcript_rows: *expected_transcript_rows,
            user_entry: serde_json::to_string(&messages[0])?,
        }),
        SessionTurnCheckpoint::MarkRetryableInterruption {
            expected_generation,
            turn_id,
            speaker_id,
        } => Ok(EncodedSessionTurnCheckpoint::MarkRetryableInterruption {
            expected_generation: *expected_generation,
            turn_id: turn_id.clone(),
            speaker_id: speaker_id.clone(),
            terminal_entries: serde_json::to_string(messages)?,
        }),
        SessionTurnCheckpoint::Resume {
            expected_generation,
            turn_id,
            legacy,
        } => Ok(EncodedSessionTurnCheckpoint::Resume {
            expected_generation: *expected_generation,
            turn_id: turn_id.clone(),
            legacy: legacy.clone(),
        }),
        SessionTurnCheckpoint::Settle {
            expected_generation,
            turn_id,
            settlement,
            speaker_id,
        } => Ok(EncodedSessionTurnCheckpoint::Settle {
            expected_generation: *expected_generation,
            turn_id: turn_id.clone(),
            settlement: *settlement,
            speaker_id: speaker_id.clone(),
            terminal_entries: serde_json::to_string(messages)?,
        }),
    }
}

pub(crate) fn validate_context_summary(record: &ContextSummaryRecord) -> Result<()> {
    identifier("actor namespace", &record.actor_namespace, 1024)?;
    identifier("session identity", &record.session_id, 128)?;
    identifier("source namespace", &record.source_namespace, 1024)?;
    identifier("summary namespace", &record.summary_namespace, 1024)?;
    identifier("source view", &record.source_view, 64)?;
    identifier("source revision", &record.source_revision, 64)?;
    ensure!(
        record.turn_id.is_some() ^ record.operation_id.is_some(),
        "context summary requires exactly one turn or operation identity"
    );
    match (
        &record.turn_id,
        &record.operation_id,
        &record.producer_actor_id,
    ) {
        (Some(turn_id), None, None) => identifier("turn identity", turn_id, 128)?,
        (None, Some(operation_id), Some(producer_actor_id)) => {
            identifier("operation identity", operation_id, 128)?;
            identifier("producer actor identity", producer_actor_id, 1024)?;
        }
        (Some(_), None, Some(_)) => {
            bail!("turn-attributed context summary cannot claim a producer actor")
        }
        (None, Some(_), None) => {
            bail!("operation-attributed context summary requires a producer actor")
        }
        _ => bail!("context summary requires exactly one turn or operation identity"),
    }
    identifier("invocation identity", &record.invocation_id, 128)?;
    ensure!(
        record.after_sequence >= 0 && record.through_sequence > record.after_sequence,
        "context summary source range is invalid"
    );
    ensure!(!record.summary.is_empty(), "context summary is empty");
    ensure!(
        record.summary.len() <= MAX_TYPED_MESSAGE_BYTES,
        "context summary exceeds {MAX_TYPED_MESSAGE_BYTES} bytes"
    );
    Ok(())
}

pub(crate) fn validate_context_summary_checkpoint(
    checkpoint: &ContextSummaryCheckpoint,
) -> Result<()> {
    validate_context_summary(&checkpoint.record)?;
    if !checkpoint.private_reasoning.is_empty() {
        validate_reasoning_summaries(&checkpoint.private_reasoning)?;
        let record = &checkpoint.record;
        let operation_id = record
            .operation_id
            .as_deref()
            .context("private compact reasoning requires operation provenance")?;
        let producer_actor_id = record
            .producer_actor_id
            .as_deref()
            .context("private compact reasoning requires producer actor provenance")?;
        for private in &checkpoint.private_reasoning {
            ensure!(
                private.turn_id.is_none()
                    && private.operation_id.as_deref() == Some(operation_id)
                    && private.session_id == record.session_id
                    && private.actor_id == producer_actor_id
                    && private.invocation_id == record.invocation_id,
                "private compact reasoning provenance does not match its context checkpoint"
            );
        }
    }
    ensure!(
        serde_json::to_vec(checkpoint)?.len() <= MAX_CONTEXT_SUMMARY_CHECKPOINT_BYTES,
        "context summary checkpoint exceeds 64 MiB"
    );
    Ok(())
}

pub(crate) fn validate_context_summary_cursor(cursor: &ContextSummaryCursor) -> Result<()> {
    identifier("actor namespace", &cursor.actor_namespace, 1024)?;
    identifier("session identity", &cursor.session_id, 128)?;
    identifier("source namespace", &cursor.source_namespace, 1024)?;
    identifier("source view", &cursor.source_view, 64)?;
    identifier("source revision", &cursor.source_revision, 64)?;
    validate_context_summary_id(&cursor.summary_id)?;
    ensure!(
        cursor.through_sequence > 0,
        "context summary cursor is invalid"
    );
    Ok(())
}

pub(crate) fn validate_context_summary_window_request(
    actor_namespace: &str,
    summary_namespace: &str,
    session_id: Option<&str>,
    source_namespace: Option<&str>,
    limit: usize,
) -> Result<()> {
    identifier("actor namespace", actor_namespace, 1024)?;
    identifier("summary namespace", summary_namespace, 1024)?;
    if let Some(session_id) = session_id {
        identifier("session identity", session_id, 128)?;
    }
    if let Some(source_namespace) = source_namespace {
        identifier("source namespace", source_namespace, 1024)?;
    }
    ensure!(
        limit <= MAX_SESSION_SOURCE_ROWS,
        "context summary limit cannot exceed {MAX_SESSION_SOURCE_ROWS}"
    );
    Ok(())
}

fn validate_context_summary_id(summary_id: &str) -> Result<()> {
    ensure!(
        summary_id.len() == 64
            && summary_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "context summary identity is malformed"
    );
    Ok(())
}

fn decode_context_summary_item(row: &sqlx::mysql::MySqlRow) -> Result<ContextSummaryItem> {
    let utf8 = |column| -> Result<String> {
        String::from_utf8(row.try_get(column)?)
            .with_context(|| format!("context summary {column} is not UTF-8"))
    };
    let summary_id: String = row.try_get("summary_id")?;
    validate_context_summary_id(&summary_id)?;
    let optional_utf8 = |column| -> Result<Option<String>> {
        row.try_get::<Option<Vec<u8>>, _>(column)?
            .map(String::from_utf8)
            .transpose()
            .with_context(|| format!("context summary {column} is not UTF-8"))
    };
    let format: String = row.try_get("record_format")?;
    let record = ContextSummaryRecord {
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
    validate_context_summary(&record)?;
    ensure!(
        format == context_summary_format(&record)?,
        "unsupported context summary record format"
    );
    Ok(ContextSummaryItem { summary_id, record })
}

fn decode_session_source_row(
    row: &sqlx::mysql::MySqlRow,
    after_exclusive: i64,
) -> Result<SequencedMessage> {
    let sequence: i64 = row.try_get("sequence")?;
    ensure!(
        sequence > after_exclusive,
        "session source sequence is out of range"
    );
    let role = String::from_utf8(row.try_get::<Vec<u8>, _>("role")?)?;
    let format: String = row.try_get("content_format")?;
    let content: String = row.try_get("content")?;
    Ok(SequencedMessage {
        sequence,
        message: decode_message(role, &format, &content)
            .with_context(|| format!("invalid session source message sequence {sequence}"))?,
    })
}

fn serialized_session_source_row_bytes(row: &SequencedMessage) -> Result<usize> {
    Ok(serde_json::to_vec(row)?.len())
}

struct SessionSourceBudget {
    max_rows: usize,
    rows: usize,
    serialized_bytes: usize,
}

impl SessionSourceBudget {
    fn new(max_rows: usize) -> Self {
        Self {
            max_rows,
            rows: 0,
            serialized_bytes: 0,
        }
    }

    fn try_include(&mut self, row_bytes: usize) -> Result<bool> {
        if self.rows >= self.max_rows {
            return Ok(false);
        }
        let serialized_bytes = self
            .serialized_bytes
            .checked_add(row_bytes)
            .context("session source snapshot byte count overflowed")?;
        if serialized_bytes > MAX_SESSION_SOURCE_BYTES {
            return Ok(false);
        }
        self.rows += 1;
        self.serialized_bytes = serialized_bytes;
        Ok(true)
    }
}

async fn context_summary_range_is_bounded(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    record: &ContextSummaryRecord,
) -> Result<bool> {
    let query_limit = i64::try_from(MAX_SESSION_SOURCE_ROWS + 1)
        .context("session source row bound exceeds integer range")?;
    let mut source = sqlx::query("SELECT sequence, role, content_format, content FROM messages WHERE namespace = ? AND session_id = ? AND sequence > ? AND sequence <= ? ORDER BY sequence LIMIT ?")
        .bind(record.source_namespace.as_bytes())
        .bind(record.session_id.as_bytes())
        .bind(record.after_sequence)
        .bind(record.through_sequence)
        .bind(query_limit)
        .fetch(&mut **transaction);
    let mut budget = SessionSourceBudget::new(MAX_SESSION_SOURCE_ROWS);
    let mut observed_through = None;
    while let Some(row) = source.try_next().await? {
        let row = decode_session_source_row(&row, record.after_sequence)?;
        let row_bytes = serialized_session_source_row_bytes(&row)?;
        if !budget.try_include(row_bytes)? {
            return Ok(false);
        }
        observed_through = Some(row.sequence);
    }
    Ok(observed_through == Some(record.through_sequence))
}

// A project-wide Dolt commit can advance because another actor wrote its own
// history. The captured commit remains the source provenance, but publication
// depends on the selected actor/session rows and prior summary still matching.
// All comparisons run before the checkpoint mutation in its short transaction.
async fn context_summary_source_unchanged(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    record: &ContextSummaryRecord,
    current_revision: &str,
) -> Result<bool> {
    // AS OF does not accept a prepared placeholder in Dolt. Permit only its
    // canonical lowercase base32 commit hash before placing it in SQL text.
    if record.source_revision.len() != 32
        || !record
            .source_revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
    {
        return Ok(false);
    }
    let merge_base: Option<String> = sqlx::query_scalar("SELECT DOLT_MERGE_BASE(?, ?)")
        .bind(&record.source_revision)
        .bind(current_revision)
        .fetch_one(&mut **transaction)
        .await?;
    if merge_base.as_deref() != Some(record.source_revision.as_str()) {
        return Ok(false);
    }
    let historical = format!("AS OF '{}'", record.source_revision);
    let (old_rows, new_rows) = (
        context_summary_source_rows(transaction, record, &historical).await?,
        context_summary_source_rows(transaction, record, "").await?,
    );
    if old_rows != new_rows
        || old_rows.last().map(|row| row.sequence) != Some(record.through_sequence)
    {
        return Ok(false);
    }
    let old_prior = context_summary_prior_state(transaction, record, &historical).await?;
    let new_prior = context_summary_prior_state(transaction, record, "").await?;
    Ok(old_prior == new_prior
        && old_prior
            .0
            .as_ref()
            .map_or(record.after_sequence == 0, |cursor| {
                cursor.through_sequence == record.after_sequence
            }))
}

async fn context_summary_source_rows(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    record: &ContextSummaryRecord,
    as_of: &str,
) -> Result<Vec<SequencedMessage>> {
    let query = format!(
        "SELECT sequence, role, content_format, content FROM messages {as_of} WHERE namespace = ? AND session_id = ? AND sequence > ? AND sequence <= ? ORDER BY sequence LIMIT ?"
    );
    let mut source = sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(record.source_namespace.as_bytes())
        .bind(record.session_id.as_bytes())
        .bind(record.after_sequence)
        .bind(record.through_sequence)
        .bind(i64::try_from(MAX_SESSION_SOURCE_ROWS + 1)?)
        .fetch(&mut **transaction);
    let mut rows = Vec::new();
    let mut budget = SessionSourceBudget::new(MAX_SESSION_SOURCE_ROWS);
    while let Some(row) = source.try_next().await? {
        let row = decode_session_source_row(&row, record.after_sequence)?;
        let row_bytes = serialized_session_source_row_bytes(&row)?;
        if !budget.try_include(row_bytes)? {
            return Ok(Vec::new());
        }
        rows.push(row);
    }
    Ok(rows)
}

async fn context_summary_prior_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::MySql>,
    record: &ContextSummaryRecord,
    as_of: &str,
) -> Result<(Option<ContextSummaryCursor>, Option<ContextSummaryItem>)> {
    let cursor_query = format!(
        "SELECT through_sequence, summary_id, source_view, source_revision FROM context_summary_cursors {as_of} WHERE actor_namespace = ? AND session_id = ? AND source_namespace = ?"
    );
    let cursor = sqlx::query(sqlx::AssertSqlSafe(cursor_query))
        .bind(record.actor_namespace.as_bytes())
        .bind(record.session_id.as_bytes())
        .bind(record.source_namespace.as_bytes())
        .fetch_optional(&mut **transaction)
        .await?
        .map(|row| {
            let cursor = ContextSummaryCursor {
                actor_namespace: record.actor_namespace.clone(),
                session_id: record.session_id.clone(),
                source_namespace: record.source_namespace.clone(),
                through_sequence: row.try_get("through_sequence")?,
                summary_id: row.try_get("summary_id")?,
                source_view: row.try_get("source_view")?,
                source_revision: row.try_get("source_revision")?,
            };
            validate_context_summary_cursor(&cursor)?;
            Ok::<_, anyhow::Error>(cursor)
        })
        .transpose()?;
    let Some(cursor) = cursor else {
        return Ok((None, None));
    };
    let summary_query = format!(
        "SELECT summary_id, actor_namespace, session_id, source_namespace, summary_namespace, source_view, source_revision, after_sequence, through_sequence, turn_id, operation_id, producer_actor_id, invocation_id, record_format, summary FROM context_summaries {as_of} WHERE summary_id = ?"
    );
    let summary = sqlx::query(sqlx::AssertSqlSafe(summary_query))
        .bind(&cursor.summary_id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(|row| decode_context_summary_item(&row))
        .transpose()?;
    ensure!(
        summary.as_ref().is_some_and(|item| {
            item.summary_id == cursor.summary_id
                && item.record.actor_namespace == record.actor_namespace
                && item.record.session_id == record.session_id
                && item.record.source_namespace == record.source_namespace
                && item.record.summary_namespace == record.summary_namespace
                && item.record.source_view == cursor.source_view
                && item.record.source_revision == cursor.source_revision
                && item.record.through_sequence == cursor.through_sequence
        }),
        ContextSummaryStale
    );
    Ok((Some(cursor), summary))
}

/// Derive the canonical durable identity for one validated context summary.
pub fn context_summary_id(record: &ContextSummaryRecord) -> Result<String> {
    validate_context_summary(record)?;
    let mut digest = Sha256::new();
    let mut values = vec![
        &record.actor_namespace,
        &record.session_id,
        &record.source_namespace,
        &record.summary_namespace,
        &record.source_view,
        &record.source_revision,
    ];
    match (
        &record.turn_id,
        &record.operation_id,
        &record.producer_actor_id,
    ) {
        (Some(turn_id), None, None) => {
            hash_context_field(&mut digest, b"kuru.context-summary.v1");
            values.extend([turn_id, &record.invocation_id]);
        }
        (None, Some(operation_id), Some(producer_actor_id)) => {
            hash_context_field(&mut digest, b"kuru.context-summary.operation.v2");
            values.extend([operation_id, producer_actor_id, &record.invocation_id]);
        }
        _ => bail!("context summary provenance is invalid"),
    }
    for value in values {
        hash_context_field(&mut digest, value.as_bytes());
    }
    hash_context_field(&mut digest, &record.after_sequence.to_be_bytes());
    hash_context_field(&mut digest, &record.through_sequence.to_be_bytes());
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn context_summary_format(record: &ContextSummaryRecord) -> Result<&'static str> {
    match (
        &record.turn_id,
        &record.operation_id,
        &record.producer_actor_id,
    ) {
        (Some(_), None, None) => Ok(CONTEXT_SUMMARY_FORMAT),
        (None, Some(_), Some(_)) => Ok(CONTEXT_SUMMARY_OPERATION_FORMAT),
        _ => bail!("context summary provenance is invalid"),
    }
}

fn hash_context_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn encode_typed_message(message: &Message) -> Result<String> {
    let encoded = serde_json::to_string(&TypedMessageRef {
        blocks: &message.blocks,
    })?;
    ensure!(
        encoded.len() <= MAX_TYPED_MESSAGE_BYTES,
        "typed message exceeds the memory row limit"
    );
    Ok(encoded)
}

fn decode_message(role: String, format: &str, content: &str) -> Result<Message> {
    match format {
        TEXT_FORMAT => Ok(Message::text(role, content)),
        TYPED_FORMAT => {
            ensure!(
                content.len() <= MAX_TYPED_MESSAGE_BYTES,
                "typed message exceeds the memory row limit"
            );
            let payload: TypedMessage = serde_json::from_str(content).map_err(|_| {
                anyhow::anyhow!("stored typed message has an invalid block payload")
            })?;
            Ok(Message {
                role,
                blocks: payload.blocks,
            })
        }
        _ => bail!("stored message has an unsupported content format"),
    }
}

async fn owned_connection(pool: &MySqlPool) -> Result<(MySqlConnection, u64)> {
    let mut connection = pool.acquire().await?.detach();
    let id = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut connection),
    )
    .await
    .context("memory connection identity deadline exceeded")??;
    Ok((connection, id))
}

async fn await_session_end(pool: &MySqlPool, id: u64, duration: Duration) -> Result<()> {
    tokio::time::timeout(duration, async {
        loop {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.processlist WHERE ID = ?",
            )
            .bind(id)
            .fetch_one(pool)
            .await?;
            if active == 0 {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("memory SQL session teardown deadline exceeded")?
}

async fn await_branch_sessions_end(
    store: &MemoryStore,
    branch: &str,
    duration: Duration,
) -> Result<()> {
    let database = format!("kuru/{branch}");
    tokio::time::timeout(duration, async {
        loop {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.processlist WHERE BINARY DB = BINARY ?",
            )
            .bind(&database)
            .fetch_one(store.pool.as_ref())
            .await?;
            if active == 0 {
                return Ok::<_, anyhow::Error>(());
            }
            #[cfg(test)]
            store.shared.server.notify_candidate_wait().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .context("candidate source session retirement deadline exceeded")?
}

async fn operation_exists(pool: &MySqlPool, operation: &str) -> Result<bool> {
    let result: Option<String> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT id FROM operations WHERE id = ?")
            .bind(operation)
            .fetch_optional(pool),
    )
    .await
    .context("memory reconciliation deadline exceeded")??;
    Ok(result.is_some())
}

async fn operation_receipt_matches(pool: &MySqlPool, expected: &LogicalReceipt) -> Result<bool> {
    let row: Option<(i8, Option<String>, Option<String>)> = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_as(
            "SELECT receipt_format, method, request_digest FROM operations WHERE id = ? LIMIT 2",
        )
        .bind(&expected.physical_id)
        .fetch_optional(pool),
    )
    .await
    .context("logical mutation receipt lookup deadline exceeded")??;
    match row {
        None => Ok(false),
        Some((1, Some(method), Some(digest)))
            if method == expected.method && digest == expected.digest =>
        {
            Ok(true)
        }
        Some(_) => Err(LogicalReceiptConflict.into()),
    }
}
async fn revision(pool: &MySqlPool) -> Result<String> {
    Ok(tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')").fetch_one(pool),
    )
    .await
    .context("revision deadline exceeded")??)
}
async fn initialize(pool: &MySqlPool) -> Result<()> {
    // Initialization is only called in a new, unpublished staging directory.
    for statement in [
        "CREATE TABLE kuru_schema (id INT PRIMARY KEY, version INT NOT NULL)",
        "INSERT INTO kuru_schema VALUES (1, 1)",
        "CREATE TABLE messages (sequence BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY, namespace VARBINARY(1024) NOT NULL, role VARBINARY(128) NOT NULL, content LONGTEXT CHARACTER SET utf8mb4 NOT NULL, INDEX messages_namespace_sequence (namespace, sequence))",
        "CREATE TABLE state (`key` VARBINARY(1024) PRIMARY KEY, value LONGTEXT CHARACTER SET utf8mb4 NOT NULL)",
        "CREATE TABLE operations (id VARCHAR(36) CHARACTER SET ascii COLLATE ascii_bin PRIMARY KEY, label VARCHAR(128) NOT NULL)",
    ] {
        sqlx::query(statement).execute(pool).await?;
    }
    sqlx::query("CALL DOLT_COMMIT('-Am', 'Initialize Kuru memory schema 1', '--author', ?)")
        .bind(AUTHOR)
        .fetch_all(pool)
        .await?;
    migrations::validate_supported(pool).await?;
    Ok(())
}
async fn validate_schema_v1(pool: &MySqlPool) -> Result<()> {
    for query in [
        "SELECT sequence, namespace, role, content FROM messages LIMIT 0",
        "SELECT `key`, value FROM state LIMIT 0",
        "SELECT id, label FROM operations LIMIT 0",
    ] {
        tokio::time::timeout(QUERY_TIMEOUT, sqlx::query(query).fetch_all(pool))
            .await
            .context("schema v1 validation deadline exceeded")??;
    }
    Ok(())
}
async fn import(pool: &MySqlPool, legacy: &LegacyImport) -> Result<()> {
    // The old file contains every project; this may be a new, empty scope.
    // Its snapshot receipt is still retained in the activation record.
    if legacy.messages.is_empty() && legacy.state.is_empty() {
        return Ok(());
    }
    let mut transaction = pool.begin().await?;
    for message in &legacy.messages {
        sqlx::query(
            "INSERT INTO messages (sequence, namespace, role, content) VALUES (?, ?, ?, ?)",
        )
        .bind(message.sequence)
        .bind(message.namespace.as_bytes())
        .bind(message.role.as_bytes())
        .bind(&message.content)
        .execute(&mut *transaction)
        .await?;
    }
    for (key, value) in &legacy.state {
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(key.as_bytes())
            .bind(value)
            .execute(&mut *transaction)
            .await?;
    }
    // Compare actual rows before making the import a durable revision.
    let rows =
        sqlx::query("SELECT sequence, namespace, role, content FROM messages ORDER BY sequence")
            .fetch_all(&mut *transaction)
            .await?;
    ensure!(
        rows.len() == legacy.messages.len(),
        "import message count differs"
    );
    for (row, original) in rows.iter().zip(&legacy.messages) {
        ensure!(
            row.try_get::<i64, _>("sequence")? == original.sequence
                && row.try_get::<Vec<u8>, _>("namespace")? == original.namespace.as_bytes()
                && row.try_get::<Vec<u8>, _>("role")? == original.role.as_bytes()
                && row.try_get::<String, _>("content")? == original.content,
            "imported message differs from preserved snapshot"
        );
    }
    let rows = sqlx::query("SELECT `key`, value FROM state ORDER BY `key`")
        .fetch_all(&mut *transaction)
        .await?;
    ensure!(
        rows.len() == legacy.state.len(),
        "import state count differs"
    );
    for (row, (key, value)) in rows.iter().zip(&legacy.state) {
        ensure!(
            row.try_get::<Vec<u8>, _>("key")? == key.as_bytes()
                && row.try_get::<String, _>("value")? == *value,
            "imported state differs from preserved snapshot"
        );
    }
    sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
        .bind(format!(
            "Import preserved SQLite {}",
            legacy.receipt.source_sha256
        ))
        .bind(AUTHOR)
        .fetch_all(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// Reuse a fully committed import only when its preserved source still matches.
/// Partial or superseded stores are stopped and moved aside, never overwritten.
async fn recover_staging(
    directory: &Path,
    scope: &str,
    legacy: Option<&LegacyImport>,
    options: &impl Fn(PathBuf, bool) -> ServerOptions,
    startup_lock: &mut Option<File>,
    progress: &mut ProgressReporter,
) -> Result<Option<StoppedStage>> {
    let parent = directory.parent().context("project store has no parent")?;
    let prefix = format!(
        "{}.staging-",
        directory
            .file_name()
            .context("project store has no name")?
            .to_string_lossy()
    );
    let mut stages = Vec::new();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        ensure!(
            Uuid::parse_str(suffix).is_ok() && entry.file_type()?.is_dir(),
            "unrecognized memory staging path: {}",
            entry.path().display()
        );
        stages.push(entry.path());
        ensure!(
            stages.len() <= 64,
            "too many interrupted memory imports; preserve and inspect {}",
            parent.display()
        );
    }
    stages.sort();
    let mut recovered = None;
    for stage in stages {
        let marker_exists = fs::symlink_metadata(stage.join("ready.json")).is_ok();
        let activation = marker_exists
            .then(|| read_activation(&stage, scope))
            .transpose()?;
        let identity_exists = fs::symlink_metadata(stage.join("identity.json")).is_ok();
        if identity_exists {
            // A completed stage may attach read-only to another owner, while
            // an incomplete bootstrap always owns one. In either case this
            // opener retains the startup lock before its first await.
            let inspection = activation.is_some();
            progress.report(MemoryOpenStage::OpeningDatabase);
            let server = Server::open_with_guard(
                options(stage.clone(), inspection),
                startup_lock.take().expect("startup lock"),
            )
            .await?;
            let pool = if inspection {
                Some(server.pool("main").await?)
            } else {
                None
            };
            let checked = async {
                if let (Some(activation), Some(pool)) = (&activation, &pool) {
                    migrations::validate_ready(&server, pool).await?;
                    ensure!(
                        revision(pool).await? == activation.initial_revision,
                        "interrupted import revision differs from its activation record"
                    );
                    let dirty: i64 = tokio::time::timeout(
                        QUERY_TIMEOUT,
                        sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
                            .fetch_one(pool.as_ref()),
                    )
                    .await
                    .context("interrupted memory stage clean-state deadline exceeded")??;
                    ensure!(
                        dirty == 0,
                        "interrupted import has uncommitted changes; preserve {} before recovery",
                        stage.display()
                    );
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let stopped = server.close_installed_guard().await;
            match (checked, stopped) {
                (Ok(()), Ok(lock)) => *startup_lock = Some(lock),
                (Err(error), Ok(_)) => return Err(error),
                (Ok(()), Err(error)) => return Err(error),
                (Err(error), Err(cleanup)) => {
                    return Err(error.context(format!(
                        "interrupted-stage validation cleanup also failed: {cleanup:#}"
                    )));
                }
            }
        } else {
            let mut recognized = true;
            for entry in fs::read_dir(&stage)? {
                let entry = entry?;
                recognized &= entry.file_name() == "lifecycle.lock" && entry.file_type()?.is_file();
            }
            ensure!(
                activation.is_none() && recognized,
                "unrecognized interrupted import without server identity: {}",
                stage.display()
            );
        }
        let lease_options = options(stage.clone(), false);
        let mut lease = Server::quiescence_at(
            &stage,
            lease_options.lifecycle_root.as_deref(),
            lease_options.timeout,
        )
        .await?;
        let matches_source =
            activation
                .as_ref()
                .is_some_and(|activation| match (&activation.migration, legacy) {
                    (None, None) => true,
                    (Some(receipt), Some(legacy)) => {
                        receipt.source_sha256 == legacy.receipt.source_sha256
                            && receipt.project_scope == legacy.receipt.project_scope
                            && receipt.messages == legacy.receipt.messages
                            && receipt.state == legacy.receipt.state
                    }
                    _ => false,
                });
        if matches_source && recovered.is_none() {
            recovered = Some(StoppedStage { _lease: lease });
        } else {
            let preserved = parent.join("interrupted");
            private_dir(&preserved)?;
            let destination =
                preserved.join(stage.file_name().context("staging path has no name")?);
            ensure!(
                fs::symlink_metadata(&destination).is_err(),
                "interrupted import preservation path already exists"
            );
            lease.move_to(&destination)?;
            eprintln!(
                "Preserved an interrupted memory import at {}",
                destination.display()
            );
        }
    }
    Ok(recovered)
}

async fn preserve_unready_stage(
    stage: &Path,
    parent: &Path,
    lifecycle_root: Option<&Path>,
    timeout: Duration,
) -> Result<()> {
    let mut lease = Server::quiescence_at(stage, lifecycle_root, timeout).await?;
    let interrupted = parent.join("interrupted");
    private_dir(&interrupted)?;
    let destination = interrupted.join(stage.file_name().context("staging path has no name")?);
    ensure!(
        fs::symlink_metadata(&destination).is_err(),
        "interrupted import preservation path already exists"
    );
    lease.move_to(&destination)?;
    Ok(())
}

pub(crate) fn identifier(label: &str, value: &str, maximum: usize) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} must not be empty");
    ensure!(value.len() <= maximum, "{label} exceeds {maximum} bytes");
    ensure!(!value.contains('\0'), "{label} must not contain NUL");
    Ok(())
}
pub(crate) fn project_directory(data: &Path, scope: &str) -> Result<PathBuf> {
    let hash = scope
        .strip_prefix("project/")
        .context("memory project scope must start with project/")?;
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "memory project scope must contain a lowercase SHA256 digest"
    );
    Ok(data.join("memory").join(hash))
}
fn read_activation(directory: &Path, scope: &str) -> Result<Activation> {
    let marker = directory.join("ready.json");
    let bytes = files::read_bytes(&marker, 16 * 1024).context(
        "project memory has no activation record; preserve the store and repair it before opening",
    )?;
    let activation: Activation = serde_json::from_slice(&bytes)?;
    ensure!(
        activation.format == 1 && activation.project_scope == scope,
        "project memory activation identity does not match"
    );
    Ok(activation)
}
pub(crate) fn private_dir(path: &Path) -> Result<()> {
    files::private_dir(path)
}
pub(crate) fn private_file(path: &Path) -> Result<File> {
    let parent = files::parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    match parent.create_new(files::name(path)?) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(parent.read_write(files::name(path)?)?)
        }
        Err(error) => Err(error.into()),
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    files::write(path, &serde_json::to_vec(value)?)
}
async fn acquire_lock(file: File, duration: Duration) -> Result<File> {
    let deadline = Instant::now() + duration;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(error) => {
                return Err(anyhow::anyhow!(error))
                    .context("memory initialization is locked by another process");
            }
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn test_cache() -> PathBuf {
    std::env::var_os("KURU_DOLT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("kuru-dolt-test-cache"))
}
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn test_supervisor() -> Result<PathBuf> {
    if let Some(snapshot) = crate::test_support::prepared_supervisor()? {
        return Ok(snapshot);
    }
    let executable = std::env::current_exe()?;
    let parent = executable
        .parent()
        .context("test executable has no parent")?;
    let directory = if parent.file_name().is_some_and(|name| name == "deps") {
        parent
            .parent()
            .context("test executable has no build directory")?
    } else {
        parent
    };
    let helper = directory.join(if cfg!(windows) {
        "kuru-memory.exe"
    } else {
        "kuru-memory"
    });
    ensure!(
        helper.is_file(),
        "Dolt supervisor fixture is missing at {}; run mise run //packages/kuru-memory:build",
        helper.display()
    );
    Ok(helper)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::rpc::ViewOperation;
    use crate::service::{self, ServiceCall, ServiceValue};
    use serde_json::json;
    use sha2::Digest;

    fn pending_public_turn() -> PublicTurnRecord {
        let origin_session_id = "session-a".to_owned();
        let turn_id = "turn-a".to_owned();
        PublicTurnRecord {
            node_id: public_turn_node_id(&origin_session_id, &turn_id).unwrap(),
            origin_session_id,
            turn_id,
            kind: PublicTurnKind::Primary,
            continuation_of_node_id: None,
            predecessor_node_id: None,
            settlement: PublicTurnSettlement::Pending,
            user_entry: Some(Message::text("user", "hello")),
            speaker_id: None,
            terminal_entries: vec![],
            record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
        }
    }

    #[test]
    fn session_catalog_and_public_turn_validators_preserve_lifecycle_boundaries() -> Result<()> {
        let pending = pending_public_turn();
        validate_public_turn(&pending)?;

        let mut completed = pending.clone();
        completed.settlement = PublicTurnSettlement::Completed;
        completed.speaker_id = Some("speaker-a".into());
        completed.terminal_entries = vec![Message::text("assistant", "answer")];
        validate_public_turn(&completed)?;

        let mut retryable_pending = completed.clone();
        retryable_pending.settlement = PublicTurnSettlement::Pending;
        validate_public_turn(&retryable_pending)?;
        let mut invalid_pending = retryable_pending;
        invalid_pending.terminal_entries.clear();
        assert!(validate_public_turn(&invalid_pending).is_err());
        let mut private = completed.clone();
        private.terminal_entries = vec![Message {
            role: "assistant".into(),
            blocks: vec![ContentBlock::ReasoningSummary {
                text: "private".into(),
            }],
        }];
        assert!(validate_public_turn(&private).is_err());

        let catalog = SessionCatalogRecord {
            session_id: "session-a".into(),
            mode: Mode::Ifs,
            label: String::new(),
            created_order: 1,
            updated_order: 1,
            lifecycle_generation: 0,
            lifecycle_state: SessionLifecycleState::Active,
            head_node_id: None,
            pending_node_id: Some(pending.node_id.clone()),
            legacy_prefix: None,
            fork_provenance: None,
            record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
        };
        validate_session_catalog(&catalog)?;
        let mut invalid_catalog = catalog.clone();
        invalid_catalog.head_node_id = invalid_catalog.pending_node_id.clone();
        assert!(validate_session_catalog(&invalid_catalog).is_err());
        let mut unsupported_catalog = catalog.clone();
        unsupported_catalog.record_format = "future-catalog".into();
        assert!(validate_session_catalog(&unsupported_catalog).is_err());
        let mut oversized_catalog = catalog.clone();
        oversized_catalog.label = "x".repeat(MAX_SESSION_LABEL_BYTES + 1);
        assert!(validate_session_catalog(&oversized_catalog).is_err());
        let mut unknown_catalog = serde_json::to_value(&catalog)?;
        unknown_catalog["unrecognized_authority"] = json!(true);
        assert!(serde_json::from_value::<SessionCatalogRecord>(unknown_catalog).is_err());

        let mut unsupported_turn = completed.clone();
        unsupported_turn.record_format = "future-turn".into();
        assert!(validate_public_turn(&unsupported_turn).is_err());
        let mut wrong_origin = completed.clone();
        wrong_origin.origin_session_id = "other-session".into();
        assert!(validate_public_turn(&wrong_origin).is_err());
        let mut missing_speaker = completed.clone();
        missing_speaker.speaker_id = None;
        assert!(validate_public_turn(&missing_speaker).is_err());
        let mut oversized_speaker = completed.clone();
        oversized_speaker.speaker_id = Some("s".repeat(1025));
        assert!(validate_public_turn(&oversized_speaker).is_err());
        let mut oversized_turn = completed.clone();
        oversized_turn.terminal_entries = vec![Message::text(
            "assistant",
            "x".repeat(MAX_SESSION_SOURCE_BYTES),
        )];
        assert!(validate_public_turn(&oversized_turn).is_err());
        let mut unknown_turn = serde_json::to_value(&completed)?;
        unknown_turn["unrecognized_entry"] = json!(true);
        assert!(serde_json::from_value::<PublicTurnRecord>(unknown_turn).is_err());
        let fork = SessionForkProvenance {
            source_session_id: "session-a".into(),
            source_node_id: completed.node_id.clone(),
            source_turn_id: completed.turn_id.clone(),
            source_label: "source".into(),
            shares_current_project_memory: true,
            record_format: FORK_PROVENANCE_RECORD_FORMAT.into(),
        };
        let mut forked_catalog = catalog.clone();
        forked_catalog.head_node_id = Some(completed.node_id.clone());
        forked_catalog.pending_node_id = None;
        forked_catalog.fork_provenance = Some(fork.clone());
        validate_session_catalog(&forked_catalog)?;
        forked_catalog
            .fork_provenance
            .as_mut()
            .unwrap()
            .record_format = "future-fork".into();
        assert!(validate_session_catalog(&forked_catalog).is_err());
        forked_catalog.fork_provenance = Some(SessionForkProvenance {
            shares_current_project_memory: false,
            ..fork
        });
        assert!(validate_session_catalog(&forked_catalog).is_err());

        let catalog_page = SessionCatalogPage {
            lifecycle_state: Some(SessionLifecycleState::Active),
            view: "main".into(),
            revision: "revision".into(),
            records: vec![catalog.clone()],
            total_rows: 1,
            next: Some(SessionCatalogCursor {
                updated_order: catalog.updated_order,
                session_id: catalog.session_id.clone(),
            }),
        };
        validate_session_catalog_page(&catalog_page)?;
        let mut wrong_catalog_cursor = catalog_page.clone();
        wrong_catalog_cursor.next.as_mut().unwrap().session_id = "other".into();
        assert!(validate_session_catalog_page(&wrong_catalog_cursor).is_err());
        let mut oversized_catalog_page = catalog_page.clone();
        oversized_catalog_page.next = None;
        oversized_catalog_page.records = vec![catalog.clone(); MAX_SESSION_SOURCE_ROWS + 1];
        assert!(validate_session_catalog_page(&oversized_catalog_page).is_err());

        let transcript_page = PublicTranscriptPage {
            session_id: "session-a".into(),
            view: "main".into(),
            revision: "revision".into(),
            head_node_id: Some(completed.node_id.clone()),
            pending: Some(pending),
            records: vec![
                PublicTranscriptEntry::Turn {
                    record: completed.clone(),
                },
                PublicTranscriptEntry::Legacy {
                    sequence: 1,
                    message: Message::text("assistant", "legacy unknown speaker"),
                },
            ],
            total_rows: 2,
            next: Some(PublicTranscriptCursor {
                session_id: "session-a".into(),
                revision: "revision".into(),
                head_node_id: Some(completed.node_id),
                next: PublicTranscriptPosition::Legacy { sequence: 1 },
            }),
        };
        validate_public_transcript_page(&transcript_page)?;
        let mut wrong_transcript_cursor = transcript_page;
        wrong_transcript_cursor.next.as_mut().unwrap().session_id = "other".into();
        assert!(validate_public_transcript_page(&wrong_transcript_cursor).is_err());
        let mut oversized_transcript_page = wrong_transcript_cursor;
        oversized_transcript_page.next = None;
        oversized_transcript_page.records = vec![
            PublicTranscriptEntry::Legacy {
                sequence: 1,
                message: Message::text("assistant", "legacy"),
            };
            MAX_SESSION_SOURCE_ROWS + 1
        ];
        assert!(validate_public_transcript_page(&oversized_transcript_page).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn malformed_public_rows_and_foreign_turn_coordinates_have_no_dolt_effect() -> Result<()>
    {
        let directory = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "a".repeat(64));
        let options =
            crate::test_support::open_options(directory.path().to_owned(), scope.clone())?;
        let store = MemoryStore::open(options).await?;
        let session = "bounded-turn";
        let namespace = format!("{scope}/transcript/{session}");
        let catalog = SessionCatalogRecord {
            session_id: session.into(),
            mode: Mode::Ifs,
            label: String::new(),
            created_order: 1,
            updated_order: 1,
            lifecycle_generation: 0,
            lifecycle_state: SessionLifecycleState::Active,
            head_node_id: None,
            pending_node_id: None,
            legacy_prefix: None,
            fork_provenance: None,
            record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
        };
        let before = store.revision().await?;
        let mut unsupported = catalog.clone();
        unsupported.record_format = "future-catalog".into();
        assert!(
            store
                .fixture_insert_public_session(&unsupported, &[])
                .await
                .is_err()
        );
        let mut oversized = catalog.clone();
        oversized.label = "x".repeat(MAX_SESSION_LABEL_BYTES + 1);
        assert!(
            store
                .fixture_insert_public_session(&oversized, &[])
                .await
                .is_err()
        );
        let mut unsupported_turn = pending_public_turn();
        unsupported_turn.record_format = "future-turn".into();
        assert!(
            store
                .fixture_insert_public_session(&catalog, &[unsupported_turn])
                .await
                .is_err()
        );
        assert_eq!(store.revision().await?, before);
        assert!(store.session_catalog_record(session).await?.is_none());

        store.create_session_catalog(session, Mode::Ifs, "").await?;
        let before = store.revision().await?;
        let message = [Message::text("user", "exact question")];
        let checkpoint = SessionTurnCheckpoint::Admit {
            expected_generation: 0,
            turn_id: "exact-turn".into(),
            label: None,
            expected_transcript_rows: Some(0),
        };
        for wrong_namespace in [
            format!("project/{}/transcript/{session}", "b".repeat(64)),
            format!("{scope}/transcript/other-session"),
        ] {
            assert!(
                store
                    .checkpoint_session_turn(&wrong_namespace, session, &message, &[], &checkpoint)
                    .await
                    .is_err()
            );
            assert_eq!(store.revision().await?, before);
            assert!(
                store
                    .public_transcript_page(session, None, 8)
                    .await?
                    .pending
                    .is_none()
            );
            assert_eq!(store.history_window(&namespace, 8).await?.total_rows, 0);
        }

        let receipt = store.with_logical_receipt(
            Uuid::new_v4(),
            "view.checkpoint_session",
            b"exact bound public turn admission",
        );
        receipt
            .checkpoint_session_turn(&namespace, session, &message, &[], &checkpoint)
            .await?;
        let accepted_revision = store.revision().await?;
        receipt
            .checkpoint_session_turn(&namespace, session, &message, &[], &checkpoint)
            .await?;
        assert_eq!(store.revision().await?, accepted_revision);
        assert_eq!(store.history_window(&namespace, 8).await?.total_rows, 1);
        assert_eq!(
            store
                .public_transcript_page(session, None, 8)
                .await?
                .pending
                .as_ref()
                .map(|turn| turn.turn_id.as_str()),
            Some("exact-turn")
        );
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn public_transcript_pages_prove_the_selected_predecessor_chain() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let mut first = pending_public_turn();
        first.settlement = PublicTurnSettlement::Completed;
        first.speaker_id = Some("speaker-a".into());
        first.terminal_entries = vec![Message::text("assistant", "first answer")];
        let second = PublicTurnRecord {
            node_id: public_turn_node_id("session-a", "turn-b")?,
            origin_session_id: "session-a".into(),
            turn_id: "turn-b".into(),
            kind: PublicTurnKind::Primary,
            continuation_of_node_id: None,
            predecessor_node_id: Some(first.node_id.clone()),
            settlement: PublicTurnSettlement::Interrupted,
            user_entry: Some(Message::text("user", "second question")),
            speaker_id: Some("speaker-b".into()),
            terminal_entries: vec![Message::text("assistant", "interrupted answer")],
            record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
        };
        validate_public_turn(&second)?;
        let pending = PublicTurnRecord {
            node_id: public_turn_node_id("session-a", "turn-c")?,
            origin_session_id: "session-a".into(),
            turn_id: "turn-c".into(),
            kind: PublicTurnKind::Primary,
            continuation_of_node_id: None,
            predecessor_node_id: Some(second.node_id.clone()),
            settlement: PublicTurnSettlement::Pending,
            user_entry: Some(Message::text("user", "pending question")),
            speaker_id: None,
            terminal_entries: vec![],
            record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
        };
        validate_public_turn(&pending)?;
        let mut sibling = first.clone();
        sibling.origin_session_id = "session-b".into();
        sibling.turn_id = "turn-sibling".into();
        sibling.node_id = public_turn_node_id(&sibling.origin_session_id, &sibling.turn_id)?;
        sibling.predecessor_node_id = None;
        validate_public_turn(&sibling)?;

        store
            .fixture_insert_public_session(
                &SessionCatalogRecord {
                    session_id: "session-a".into(),
                    mode: Mode::Ifs,
                    label: String::new(),
                    created_order: 1,
                    updated_order: 1,
                    lifecycle_generation: 0,
                    lifecycle_state: SessionLifecycleState::Active,
                    head_node_id: Some(second.node_id.clone()),
                    pending_node_id: Some(pending.node_id.clone()),
                    legacy_prefix: None,
                    fork_provenance: None,
                    record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
                },
                &[first.clone(), second.clone(), pending.clone()],
            )
            .await?;
        store
            .fixture_insert_public_session(
                &SessionCatalogRecord {
                    session_id: "session-b".into(),
                    mode: Mode::Ifs,
                    label: String::new(),
                    created_order: 2,
                    updated_order: 2,
                    lifecycle_generation: 0,
                    lifecycle_state: SessionLifecycleState::Active,
                    head_node_id: Some(sibling.node_id.clone()),
                    pending_node_id: None,
                    legacy_prefix: None,
                    fork_provenance: None,
                    record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
                },
                &[sibling.clone()],
            )
            .await?;

        let first_page = store.public_transcript_page("session-a", None, 1).await?;
        assert_eq!(first_page.total_rows, 2);
        assert_eq!(first_page.pending.as_ref(), Some(&pending));
        assert!(matches!(
            first_page.records.as_slice(),
            [PublicTranscriptEntry::Turn { record }] if record == &second
        ));
        let cursor = first_page
            .next
            .as_ref()
            .context("first page continuation")?;
        assert!(matches!(
            &cursor.next,
            PublicTranscriptPosition::Turn { node_id } if node_id == &first.node_id
        ));
        let last_page = store
            .public_transcript_page("session-a", Some(cursor), 1)
            .await?;
        assert!(matches!(
            last_page.records.as_slice(),
            [PublicTranscriptEntry::Turn { record }] if record == &first
        ));
        assert!(last_page.next.is_none());

        let mut forged = cursor.clone();
        forged.next = PublicTranscriptPosition::Turn {
            node_id: sibling.node_id.clone(),
        };
        let error = store
            .public_transcript_page("session-a", Some(&forged), 1)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not reachable"), "{error:#}");

        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn long_public_transcript_pages_cover_one_pinned_revision_exactly_once() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let session_id = "long-public-session";
        let mut turns = Vec::with_capacity(1025);
        let mut predecessor = None;
        for index in 0..1025 {
            let turn_id = format!("turn-{index:04}");
            let record = PublicTurnRecord {
                node_id: public_turn_node_id(session_id, &turn_id)?,
                origin_session_id: session_id.into(),
                turn_id,
                kind: PublicTurnKind::Primary,
                continuation_of_node_id: None,
                predecessor_node_id: predecessor,
                settlement: PublicTurnSettlement::Completed,
                user_entry: Some(Message::text("user", format!("question-{index:04}"))),
                speaker_id: Some("speaker-a".into()),
                terminal_entries: vec![Message::text("assistant", format!("answer-{index:04}"))],
                record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
            };
            predecessor = Some(record.node_id.clone());
            turns.push(record);
        }
        store
            .fixture_insert_public_session(
                &SessionCatalogRecord {
                    session_id: session_id.into(),
                    mode: Mode::Ifs,
                    label: "long public transcript".into(),
                    created_order: 1,
                    updated_order: 1,
                    lifecycle_generation: 0,
                    lifecycle_state: SessionLifecycleState::Active,
                    head_node_id: predecessor,
                    pending_node_id: None,
                    legacy_prefix: None,
                    fork_provenance: None,
                    record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
                },
                &turns,
            )
            .await?;

        let mut cursor = None;
        let mut first_cursor = None;
        let mut revision = None;
        let mut actual = Vec::new();
        loop {
            let page = store
                .public_transcript_page(session_id, cursor.as_ref(), 128)
                .await?;
            assert_eq!(page.total_rows, 1025);
            assert!(page.records.len() <= 128);
            assert_eq!(
                revision.get_or_insert_with(|| page.revision.clone()),
                &page.revision
            );
            for entry in &page.records {
                let PublicTranscriptEntry::Turn { record } = entry else {
                    panic!("long v7 transcript returned a legacy row")
                };
                actual.push(record.turn_id.clone());
            }
            if first_cursor.is_none() {
                first_cursor = page.next.clone();
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let expected = (0..1025)
            .rev()
            .map(|index| format!("turn-{index:04}"))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);

        let namespace = "project/transcript/long-public-session";
        store
            .checkpoint_session_turn(
                namespace,
                session_id,
                &[Message::text("user", "question after captured page")],
                &[("long-page-journal".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "turn-after-capture".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                session_id,
                &[Message::text("assistant", "answer after captured page")],
                &[("long-page-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "turn-after-capture".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "speaker-b".into(),
                },
            )
            .await?;
        let current = store.public_transcript_page(session_id, None, 128).await?;
        assert_eq!(current.total_rows, 1026);
        assert!(matches!(current.records.first(),
            Some(PublicTranscriptEntry::Turn { record })
                if record.turn_id == "turn-after-capture"));
        let error = store
            .public_transcript_page(
                session_id,
                Some(first_cursor.as_ref().context("long transcript cursor")?),
                128,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("revision changed"), "{error:#}");
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn terminal_interruption_is_a_fork_boundary_with_independent_suffixes() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let parent = "interrupted-fork-parent";
        let child = "interrupted-fork-child";
        let parent_namespace = "project/transcript/interrupted-fork-parent";
        let child_namespace = "project/transcript/interrupted-fork-child";
        store
            .create_session_catalog(parent, Mode::Ifs, "parent")
            .await?;
        store
            .checkpoint_session_turn(
                parent_namespace,
                parent,
                &[Message::text("user", "shared interrupted question")],
                &[("interrupted-journal".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "terminal-boundary".into(),
                    label: None,
                    expected_transcript_rows: Some(0),
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                parent_namespace,
                parent,
                &[Message::text("kuru-interruption", "terminal interruption")],
                &[("interrupted-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "terminal-boundary".into(),
                    settlement: PublicTurnSettlement::Interrupted,
                    speaker_id: "speaker-a".into(),
                },
            )
            .await?;
        let source = store.public_transcript_page(parent, None, 16).await?;
        let [PublicTranscriptEntry::Turn { record: boundary }] = source.records.as_slice() else {
            bail!("terminal interruption did not create a settled boundary")
        };
        assert_eq!(boundary.settlement, PublicTurnSettlement::Interrupted);
        assert_eq!(boundary.speaker_id.as_deref(), Some("speaker-a"));
        let boundary = boundary.clone();
        store
            .fork_session_catalog(parent, 0, &boundary.node_id, child, "child")
            .await?;

        for (session, namespace, turn, question, answer) in [
            (
                parent,
                parent_namespace,
                "parent-suffix",
                "parent question",
                "parent answer",
            ),
            (
                child,
                child_namespace,
                "child-suffix",
                "child question",
                "child answer",
            ),
        ] {
            let prior_rows = store.history_window(namespace, 0).await?.total_rows;
            store
                .checkpoint_session_turn(
                    namespace,
                    session,
                    &[Message::text("user", question)],
                    &[(format!("{turn}-journal"), json!({"state": "started"}))],
                    &SessionTurnCheckpoint::Admit {
                        expected_generation: 0,
                        turn_id: turn.into(),
                        label: None,
                        expected_transcript_rows: Some(prior_rows),
                    },
                )
                .await?;
            store
                .checkpoint_session_turn(
                    namespace,
                    session,
                    &[Message::text("assistant", answer)],
                    &[(format!("{turn}-journal"), json!({"state": "ended"}))],
                    &SessionTurnCheckpoint::Settle {
                        expected_generation: 0,
                        turn_id: turn.into(),
                        settlement: PublicTurnSettlement::Completed,
                        speaker_id: "speaker-b".into(),
                    },
                )
                .await?;
        }
        let parent_page = store.public_transcript_page(parent, None, 16).await?;
        let child_page = store.public_transcript_page(child, None, 16).await?;
        for (page, own, other) in [
            (&parent_page, "parent-suffix", "child-suffix"),
            (&child_page, "child-suffix", "parent-suffix"),
        ] {
            assert_eq!(page.total_rows, 2);
            assert!(matches!(page.records.as_slice(),
                [PublicTranscriptEntry::Turn { record: suffix }, PublicTranscriptEntry::Turn { record: shared }]
                    if suffix.turn_id == own
                        && suffix.predecessor_node_id.as_deref() == Some(boundary.node_id.as_str())
                        && shared == &boundary));
            assert!(!page.records.iter().any(|entry| matches!(entry,
                PublicTranscriptEntry::Turn { record } if record.turn_id == other)));
        }
        assert_eq!(
            store.history_window(parent_namespace, 16).await?.total_rows,
            4
        );
        assert_eq!(
            store.history_window(child_namespace, 16).await?.total_rows,
            2
        );
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn session_lifecycle_mutations_are_generation_checked_and_reversible() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let created = store
            .create_session_catalog("lifecycle-session", Mode::Jungian, "first label")
            .await?;
        assert_eq!(created.lifecycle_generation, 0);
        assert_eq!(created.lifecycle_state, SessionLifecycleState::Active);
        let created_record = store
            .session_catalog_page(None, None, None, 16)
            .await?
            .records
            .into_iter()
            .next()
            .context("created session was absent from its catalog")?;
        assert_eq!(created_record.created_order, created_record.updated_order);

        let duplicate = store
            .create_session_catalog("lifecycle-session", Mode::Jungian, "duplicate")
            .await
            .unwrap_err();
        assert_eq!(
            duplicate
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::AlreadyExists)
        );

        let renamed = store
            .rename_session_catalog("lifecycle-session", 0, "renamed")
            .await?;
        assert_eq!(renamed.lifecycle_generation, 1);
        let renamed_record = store
            .session_catalog_page(None, None, None, 16)
            .await?
            .records
            .into_iter()
            .next()
            .context("renamed session was absent from its catalog")?;
        assert_eq!(renamed_record.label, "renamed");
        assert!(renamed.updated_order > created.updated_order);
        let revision = store.revision().await?;
        let stale = store
            .remove_session_catalog("lifecycle-session", 0)
            .await
            .unwrap_err();
        assert_eq!(
            stale
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::GenerationChanged)
        );
        assert_eq!(store.revision().await?, revision);

        let removed = store.remove_session_catalog("lifecycle-session", 1).await?;
        assert_eq!(removed.lifecycle_generation, 2);
        assert_eq!(removed.lifecycle_state, SessionLifecycleState::Removed);
        let already_removed = store
            .remove_session_catalog("lifecycle-session", 2)
            .await
            .unwrap_err();
        assert_eq!(
            already_removed
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::AlreadyRemoved)
        );

        let restored = store
            .restore_session_catalog("lifecycle-session", 2)
            .await?;
        assert_eq!(restored.lifecycle_generation, 3);
        assert_eq!(restored.lifecycle_state, SessionLifecycleState::Active);
        assert!(restored.updated_order > removed.updated_order);
        let page = store
            .session_catalog_page(Some(SessionLifecycleState::Active), None, None, 16)
            .await?;
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].label, "renamed");
        assert_eq!(page.records[0].created_order, created_record.created_order);
        assert_eq!(page.records[0].updated_order, restored.updated_order);

        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn session_lifecycle_exact_retry_returns_its_original_receipt_result() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        store
            .create_session_catalog("receipt-session", Mode::Ifs, "original")
            .await?;
        let request_id = Uuid::new_v4();
        let rename =
            store.with_logical_receipt(request_id, "view.rename_session", b"exact rename request");
        let original = rename
            .rename_session_catalog("receipt-session", 0, "renamed")
            .await?;
        assert_eq!(original.lifecycle_generation, 1);
        assert_eq!(original.lifecycle_state, SessionLifecycleState::Active);

        let removed = store.remove_session_catalog("receipt-session", 1).await?;
        assert_eq!(removed.lifecycle_generation, 2);
        assert_eq!(removed.lifecycle_state, SessionLifecycleState::Removed);
        let revision = store.revision().await?;

        let replayed = rename
            .rename_session_catalog("receipt-session", 0, "renamed")
            .await?;
        assert_eq!(replayed, original);
        assert_eq!(store.revision().await?, revision);
        let page = store.session_catalog_page(None, None, None, 16).await?;
        assert_eq!(page.records.len(), 1);
        assert_eq!(
            page.records[0].lifecycle_state,
            SessionLifecycleState::Removed
        );
        assert_eq!(page.records[0].lifecycle_generation, 2);

        let changed = store.with_logical_receipt(
            request_id,
            "view.rename_session",
            b"different rename request",
        );
        let conflict = changed
            .rename_session_catalog("receipt-session", 0, "changed")
            .await
            .unwrap_err();
        assert!(conflict.downcast_ref::<LogicalReceiptConflict>().is_some());
        assert_eq!(store.revision().await?, revision);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn settled_forks_keep_the_exact_legacy_prefix_across_ancestor_changes() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let parent = "legacy-fork-parent";
        let child = "legacy-fork-child";
        let grandchild = "legacy-fork-grandchild";
        let namespace = "project/transcript/legacy-fork-parent";

        store
            .append_session_message(
                namespace,
                parent,
                &Message::text("assistant", "legacy prefix sentinel"),
            )
            .await?;
        let legacy = store
            .session_history_window_after(namespace, parent, 0, 16)
            .await?;
        let legacy_row = legacy.rows.first().context("legacy prefix row")?;
        let prefix = LegacyTranscriptPrefix {
            namespace: namespace.into(),
            source_session_id: parent.into(),
            source_revision: legacy.revision,
            first_sequence: legacy_row.sequence,
            through_sequence: legacy_row.sequence,
            row_count: 1,
            record_format: LEGACY_PREFIX_RECORD_FORMAT.into(),
        };
        store
            .fixture_insert_public_session(
                &SessionCatalogRecord {
                    session_id: parent.into(),
                    mode: Mode::Freudian,
                    label: "original parent".into(),
                    created_order: 1,
                    updated_order: 1,
                    lifecycle_generation: 0,
                    lifecycle_state: SessionLifecycleState::Active,
                    head_node_id: None,
                    pending_node_id: None,
                    legacy_prefix: Some(prefix.clone()),
                    fork_provenance: None,
                    record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
                },
                &[],
            )
            .await?;

        store
            .checkpoint_session_turn(
                namespace,
                parent,
                &[Message::text("user", "shared question")],
                &[("legacy-fork-journal-1".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "shared-turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                parent,
                &[Message::text("assistant", "shared answer")],
                &[("legacy-fork-journal-1".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "shared-turn".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "part-a".into(),
                },
            )
            .await?;
        let parent_prefix = store.public_transcript_page(parent, None, 16).await?;
        let selected = match parent_prefix.records.first() {
            Some(PublicTranscriptEntry::Turn { record }) => record.clone(),
            _ => bail!("settled parent turn is absent"),
        };

        store
            .fork_session_catalog(parent, 0, &selected.node_id, child, "child label")
            .await?;
        let child_before = store.public_transcript_page(child, None, 16).await?;
        assert_eq!(
            child_before.head_node_id.as_deref(),
            Some(&*selected.node_id)
        );
        assert_eq!(child_before.total_rows, 2);
        assert!(matches!(
            child_before.records.as_slice(),
            [
                PublicTranscriptEntry::Turn { record },
                PublicTranscriptEntry::Legacy { message, .. }
            ] if record == &selected
                && message.text_projection() == "legacy prefix sentinel"
        ));

        store
            .checkpoint_session_turn(
                namespace,
                parent,
                &[Message::text("user", "later parent question")],
                &[("legacy-fork-journal-2".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "later-parent-turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                parent,
                &[Message::text("assistant", "later parent answer")],
                &[("legacy-fork-journal-2".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "later-parent-turn".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "part-b".into(),
                },
            )
            .await?;
        store
            .fork_session_catalog(child, 0, &selected.node_id, grandchild, "grandchild label")
            .await?;

        store
            .rename_session_catalog(parent, 0, "renamed parent")
            .await?;
        store.remove_session_catalog(parent, 1).await?;
        store.restore_session_catalog(parent, 2).await?;

        let page = store.session_catalog_page(None, None, None, 16).await?;
        let parent_record = page
            .records
            .iter()
            .find(|record| record.session_id == parent)
            .context("parent catalog row")?;
        let child_record = page
            .records
            .iter()
            .find(|record| record.session_id == child)
            .context("child catalog row")?;
        let grandchild_record = page
            .records
            .iter()
            .find(|record| record.session_id == grandchild)
            .context("grandchild catalog row")?;
        assert_eq!(parent_record.legacy_prefix.as_ref(), Some(&prefix));
        assert_eq!(child_record.legacy_prefix.as_ref(), Some(&prefix));
        assert_eq!(grandchild_record.legacy_prefix.as_ref(), Some(&prefix));
        assert!(matches!(
            child_record.fork_provenance.as_ref(),
            Some(fork)
                if fork.source_session_id == parent
                    && fork.source_node_id == selected.node_id
                    && fork.source_turn_id == selected.turn_id
                    && fork.source_label == "original parent"
                    && fork.shares_current_project_memory
        ));
        assert!(matches!(
            grandchild_record.fork_provenance.as_ref(),
            Some(fork)
                if fork.source_session_id == child
                    && fork.source_node_id == selected.node_id
                    && fork.source_turn_id == selected.turn_id
                    && fork.source_label == "child label"
                    && fork.shares_current_project_memory
        ));

        let export = store.begin_active_export().await?;
        let mut cursor = None;
        let mut exported = Vec::new();
        loop {
            let page = export.page(cursor).await?;
            exported.extend(page.records);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        export.verify_counts(5, 2, 0, 0, 3, 2)?;
        assert_eq!(
            exported
                .iter()
                .filter(|record| matches!(record, StorageRecord::SessionCatalog { .. }))
                .count(),
            3
        );
        assert_eq!(
            exported
                .iter()
                .filter(|record| matches!(record, StorageRecord::PublicTurn { .. }))
                .count(),
            2
        );
        assert!(exported.iter().any(|record| matches!(
            record,
            StorageRecord::SessionCatalog { record }
                if record == child_record
                    && record.legacy_prefix.as_ref() == Some(&prefix)
        )));
        assert!(exported.iter().any(|record| matches!(
            record,
            StorageRecord::PublicTurn { record } if record == &selected
        )));

        for forked in [child, grandchild] {
            let transcript = store.public_transcript_page(forked, None, 16).await?;
            assert_eq!(transcript.head_node_id.as_deref(), Some(&*selected.node_id));
            assert_eq!(transcript.total_rows, 2);
            assert_eq!(transcript.records, child_before.records);
            assert!(transcript.pending.is_none());
        }
        let parent_after = store.public_transcript_page(parent, None, 16).await?;
        assert_eq!(parent_after.total_rows, 3);
        assert!(parent_after.records.iter().any(|entry| matches!(
            entry,
            PublicTranscriptEntry::Turn { record }
                if record.turn_id == "later-parent-turn"
        )));

        store
            .checkpoint_session_turn(
                namespace,
                parent,
                &[Message::text("user", "pending fork question")],
                &[("legacy-fork-journal-3".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 3,
                    turn_id: "pending-parent-turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        let pending = store
            .public_transcript_page(parent, None, 16)
            .await?
            .pending
            .context("pending parent turn")?;
        let before_refusal = store.revision().await?;
        let pending_error = store
            .fork_session_catalog(
                parent,
                3,
                &pending.node_id,
                "pending-fork-refused",
                "refused pending",
            )
            .await
            .unwrap_err();
        assert_eq!(
            pending_error
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::SourceNodePending)
        );
        assert_eq!(store.revision().await?, before_refusal);

        store.remove_session_catalog(child, 0).await?;
        let before_removed_refusal = store.revision().await?;
        let removed_error = store
            .fork_session_catalog(
                child,
                1,
                &selected.node_id,
                "removed-fork-refused",
                "refused removed",
            )
            .await
            .unwrap_err();
        assert_eq!(
            removed_error
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::SourceRemoved)
        );
        assert_eq!(store.revision().await?, before_removed_refusal);
        let missing_node = public_turn_node_id("absent-session", "absent-turn")?;
        let before_missing_refusal = store.revision().await?;
        let parent_before_refusal = store
            .session_catalog_record(parent)
            .await?
            .context("parent catalog before missing fork refusal")?;
        let missing_error = store
            .fork_session_catalog(parent, 3, &missing_node, "missing-fork-refused", "missing")
            .await
            .unwrap_err();
        assert_eq!(
            missing_error
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::SourceNodeMissing)
        );
        assert_eq!(store.revision().await?, before_missing_refusal);
        assert_eq!(
            store.session_catalog_record(parent).await?,
            Some(parent_before_refusal)
        );

        let other = "unrelated-fork-source";
        store
            .create_session_catalog(other, Mode::Ifs, "other")
            .await?;
        store
            .checkpoint_session_turn(
                "project/transcript/unrelated-fork-source",
                other,
                &[Message::text("user", "other question")],
                &[("other-fork-journal".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "other-turn".into(),
                    label: None,
                    expected_transcript_rows: Some(0),
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                "project/transcript/unrelated-fork-source",
                other,
                &[Message::text("assistant", "other answer")],
                &[("other-fork-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "other-turn".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "other-speaker".into(),
                },
            )
            .await?;
        let unrelated_node = public_turn_node_id(other, "other-turn")?;
        let before_unrelated_refusal = store.revision().await?;
        let parent_before_refusal = store
            .session_catalog_record(parent)
            .await?
            .context("parent catalog before unrelated fork refusal")?;
        let unrelated_error = store
            .fork_session_catalog(
                parent,
                3,
                &unrelated_node,
                "unrelated-fork-refused",
                "unrelated",
            )
            .await
            .unwrap_err();
        assert_eq!(
            unrelated_error
                .downcast_ref::<SessionLifecycleRejected>()
                .map(|error| error.0),
            Some(SessionLifecycleRefusal::SourceNodeUnreachable)
        );
        assert_eq!(store.revision().await?, before_unrelated_refusal);
        assert_eq!(
            store.session_catalog_record(parent).await?,
            Some(parent_before_refusal)
        );
        let final_catalog = store.session_catalog_page(None, None, None, 16).await?;
        assert!(final_catalog.records.iter().all(|record| {
            record.session_id != "pending-fork-refused"
                && record.session_id != "removed-fork-refused"
                && record.session_id != "missing-fork-refused"
                && record.session_id != "unrelated-fork-refused"
        }));

        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn checked_admission_binds_raw_row_and_replays_after_later_append() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let session = "anchored-session";
        let namespace = "project/transcript/anchored-session";
        let journal_key = "anchored-turn-journal";
        store.create_session_catalog(session, Mode::Ifs, "").await?;
        store
            .checkpoint_session(
                namespace,
                session,
                &[Message::text("user", "older raw entry")],
                &[],
            )
            .await?;
        let admission = SessionTurnCheckpoint::Admit {
            expected_generation: 0,
            turn_id: "anchored-turn".into(),
            label: None,
            expected_transcript_rows: Some(1),
        };
        let journal = json!({"id": "anchored-turn", "admitted_transcript_row": 2});
        let messages = [Message::text("user", "new question")];
        let values = [(journal_key.into(), journal.clone())];
        let stale = SessionTurnCheckpoint::Admit {
            expected_generation: 0,
            turn_id: "anchored-turn".into(),
            label: None,
            expected_transcript_rows: Some(0),
        };
        let before = store.revision().await?;
        let error = store
            .checkpoint_session_turn(namespace, session, &messages, &values, &stale)
            .await
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<SessionTurnRejected>()
                .map(|error| error.0),
            Some(SessionTurnRefusal::TranscriptChanged)
        );
        assert_eq!(store.revision().await?, before);
        assert_eq!(store.history_window(namespace, 8).await?.total_rows, 1);
        assert!(store.get(journal_key).await?.is_none());
        assert!(
            store
                .public_transcript_page(session, None, 8)
                .await?
                .pending
                .is_none()
        );

        let receipt = store.with_logical_receipt(
            Uuid::new_v4(),
            "view.checkpoint_session",
            b"anchored turn admission",
        );
        receipt
            .checkpoint_session_turn(namespace, session, &messages, &values, &admission)
            .await?;
        assert_eq!(store.history_window(namespace, 8).await?.total_rows, 2);
        assert_eq!(store.get(journal_key).await?, Some(journal));
        let pending = store
            .public_transcript_page(session, None, 8)
            .await?
            .pending;
        assert!(matches!(pending, Some(record) if record.turn_id == "anchored-turn"));
        store
            .checkpoint_session(
                namespace,
                session,
                &[Message::text("kuru-interruption", "later raw marker")],
                &[],
            )
            .await?;
        let later_revision = store.revision().await?;
        receipt
            .checkpoint_session_turn(namespace, session, &messages, &values, &admission)
            .await?;
        assert_eq!(store.revision().await?, later_revision);
        assert_eq!(store.history_window(namespace, 8).await?.total_rows, 3);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn session_turn_checkpoint_admits_and_settles_once_with_runtime_state() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        store
            .create_session_catalog("turn-session", Mode::Ifs, "")
            .await?;
        let admission = SessionTurnCheckpoint::Admit {
            expected_generation: 0,
            turn_id: "turn-1".into(),
            label: Some("first question".into()),
            expected_transcript_rows: None,
        };
        let admission_id = Uuid::new_v4();
        let admitted =
            store.with_logical_receipt(admission_id, "view.checkpoint_session", b"admit turn-1");
        admitted
            .checkpoint_session_turn(
                "project/transcript/turn-session",
                "turn-session",
                &[Message::text("user", "question")],
                &[("turn-journal".into(), json!({"state": "started"}))],
                &admission,
            )
            .await?;
        let admitted_revision = store.revision().await?;
        admitted
            .checkpoint_session_turn(
                "project/transcript/turn-session",
                "turn-session",
                &[Message::text("user", "question")],
                &[("turn-journal".into(), json!({"state": "started"}))],
                &admission,
            )
            .await?;
        assert_eq!(store.revision().await?, admitted_revision);

        let pending_page = store
            .public_transcript_page("turn-session", None, 16)
            .await?;
        assert!(pending_page.records.is_empty());
        assert!(matches!(
            pending_page.pending.as_ref(),
            Some(record)
                if record.turn_id == "turn-1"
                    && record.settlement == PublicTurnSettlement::Pending
                    && record.user_entry == Some(Message::text("user", "question"))
        ));
        assert_eq!(
            store.history("project/transcript/turn-session", 16).await?,
            [Message::text("user", "question")]
        );

        store
            .checkpoint_session_turn(
                "project/transcript/turn-session",
                "turn-session",
                &[Message::text("kuru-interruption", "interrupted")],
                &[("turn-journal".into(), json!({"state": "interrupted"}))],
                &SessionTurnCheckpoint::MarkRetryableInterruption {
                    expected_generation: 0,
                    turn_id: "turn-1".into(),
                    speaker_id: "kuru-interruption".into(),
                },
            )
            .await?;
        let retryable = store
            .public_transcript_page("turn-session", None, 16)
            .await?;
        assert!(matches!(
            retryable.pending.as_ref(),
            Some(record)
                if record.settlement == PublicTurnSettlement::Pending
                    && record.speaker_id.as_deref() == Some("kuru-interruption")
                    && record.terminal_entries
                        == [Message::text("kuru-interruption", "interrupted")]
        ));

        let stale_revision = store.revision().await?;
        let stale = store
            .checkpoint_session_turn(
                "project/transcript/turn-session",
                "turn-session",
                &[Message::text("assistant", "answer")],
                &[("turn-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 1,
                    turn_id: "turn-1".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "part-a".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            stale
                .downcast_ref::<SessionTurnRejected>()
                .map(|error| error.0),
            Some(SessionTurnRefusal::GenerationChanged)
        );
        assert_eq!(store.revision().await?, stale_revision);

        let settlement = SessionTurnCheckpoint::Settle {
            expected_generation: 0,
            turn_id: "turn-1".into(),
            settlement: PublicTurnSettlement::Completed,
            speaker_id: "part-a".into(),
        };
        let settled =
            store.with_logical_receipt(Uuid::new_v4(), "view.checkpoint_session", b"settle turn-1");
        settled
            .checkpoint_session_turn(
                "project/transcript/turn-session",
                "turn-session",
                &[Message::text("assistant", "answer")],
                &[("turn-journal".into(), json!({"state": "ended"}))],
                &settlement,
            )
            .await?;
        let page = store
            .public_transcript_page("turn-session", None, 16)
            .await?;
        assert!(page.pending.is_none());
        assert!(matches!(
            page.records.as_slice(),
            [PublicTranscriptEntry::Turn { record }]
                if record.turn_id == "turn-1"
                    && record.settlement == PublicTurnSettlement::Completed
                    && record.speaker_id.as_deref() == Some("part-a")
                    && record.terminal_entries == [
                        Message::text("kuru-interruption", "interrupted"),
                        Message::text("assistant", "answer")
                    ]
        ));
        assert_eq!(
            store.get("turn-journal").await?,
            Some(json!({"state": "ended"}))
        );
        assert_eq!(
            store.history("project/transcript/turn-session", 16).await?,
            [
                Message::text("user", "question"),
                Message::text("kuru-interruption", "interrupted"),
                Message::text("assistant", "answer")
            ]
        );

        let revision = store.revision().await?;
        let late = store
            .checkpoint_session_turn(
                "project/transcript/turn-session",
                "turn-session",
                &[Message::text("kuru-interruption", "late")],
                &[("turn-journal".into(), json!({"state": "interrupted"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "turn-1".into(),
                    settlement: PublicTurnSettlement::Interrupted,
                    speaker_id: "kuru-interruption".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            late.downcast_ref::<SessionTurnRejected>()
                .map(|error| error.0),
            Some(SessionTurnRefusal::PendingTurnMissing)
        );
        assert_eq!(store.revision().await?, revision);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn older_safe_turn_continues_after_intervening_settled_turns_without_another_user_row()
    -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let namespace = "project/transcript/continuation-session";
        let session = "continuation-session";
        store
            .create_session_catalog(session, Mode::Ifs, "continuations")
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("user", "old question")],
                &[("old-journal".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "old-turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("kuru-interruption", "interrupted")],
                &[("old-journal".into(), json!({"state": "interrupted"}))],
                &SessionTurnCheckpoint::MarkRetryableInterruption {
                    expected_generation: 0,
                    turn_id: "old-turn".into(),
                    speaker_id: "kuru-interruption".into(),
                },
            )
            .await?;

        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("user", "later question")],
                &[("later-journal".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "later-turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("assistant", "later answer")],
                &[("later-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "later-turn".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "part-later".into(),
                },
            )
            .await?;

        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[],
                &[("old-journal".into(), json!({"state": "resumed"}))],
                &SessionTurnCheckpoint::Resume {
                    expected_generation: 0,
                    turn_id: "old-turn".into(),
                    legacy: None,
                },
            )
            .await?;
        let first_continuation = store.public_transcript_page(session, None, 16).await?;
        let continuation = first_continuation
            .pending
            .as_ref()
            .context("older retry did not create a pending continuation")?;
        assert_eq!(continuation.kind, PublicTurnKind::Continuation);
        assert_eq!(
            continuation.node_id,
            public_turn_continuation_node_id(session, "old-turn")?
        );
        assert_eq!(
            continuation.continuation_of_node_id.as_deref(),
            Some(public_turn_node_id(session, "old-turn")?.as_str())
        );
        assert!(continuation.user_entry.is_none());

        // A distinct turn parks the empty continuation. Its next exact resume
        // reuses the same node identity and rebases only the still-pending link.
        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("user", "newest question")],
                &[("newest-journal".into(), json!({"state": "started"}))],
                &SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "newest-turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("assistant", "newest answer")],
                &[("newest-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "newest-turn".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "part-newest".into(),
                },
            )
            .await?;
        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[],
                &[("old-journal".into(), json!({"state": "resumed-again"}))],
                &SessionTurnCheckpoint::Resume {
                    expected_generation: 0,
                    turn_id: "old-turn".into(),
                    legacy: None,
                },
            )
            .await?;
        let rebound = store.public_transcript_page(session, None, 16).await?;
        let rebound_continuation = rebound.pending.as_ref().unwrap();
        assert_eq!(rebound_continuation.node_id, continuation.node_id);
        assert_eq!(
            rebound_continuation.predecessor_node_id,
            rebound.head_node_id
        );

        store
            .checkpoint_session_turn(
                namespace,
                session,
                &[Message::text("assistant", "old answer")],
                &[("old-journal".into(), json!({"state": "ended"}))],
                &SessionTurnCheckpoint::Settle {
                    expected_generation: 0,
                    turn_id: "old-turn".into(),
                    settlement: PublicTurnSettlement::Completed,
                    speaker_id: "part-old".into(),
                },
            )
            .await?;
        let settled = store.public_transcript_page(session, None, 16).await?;
        assert!(settled.pending.is_none());
        assert!(matches!(
            settled.records.first(),
            Some(PublicTranscriptEntry::Turn { record })
                if record.kind == PublicTurnKind::Continuation
                    && record.turn_id == "old-turn"
                    && record.user_entry.is_none()
                    && record.terminal_entries == [Message::text("assistant", "old answer")]
        ));
        assert_eq!(
            store.history(namespace, 16).await?,
            [
                Message::text("user", "old question"),
                Message::text("kuru-interruption", "interrupted"),
                Message::text("user", "later question"),
                Message::text("assistant", "later answer"),
                Message::text("user", "newest question"),
                Message::text("assistant", "newest answer"),
                Message::text("assistant", "old answer"),
            ]
        );
        store.close().await?;
        Ok(())
    }

    fn reasoning_summary(text: impl Into<String>) -> ReasoningSummaryRecord {
        ReasoningSummaryRecord {
            session_id: "session".into(),
            turn_id: Some("turn".into()),
            operation_id: None,
            actor_id: "actor".into(),
            invocation_id: "invocation".into(),
            item_id: Some("item".into()),
            output_index: Some(0),
            summary_index: 0,
            text: text.into(),
        }
    }

    #[tokio::test]
    async fn private_reasoning_summary_batch_is_atomic_and_idempotent() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let first = reasoning_summary("first");
        store
            .put_reasoning_summaries(std::slice::from_ref(&first))
            .await?;
        store
            .put_reasoning_summaries(std::slice::from_ref(&first))
            .await?;

        let mut fresh = reasoning_summary("fresh");
        fresh.summary_index = 1;
        let mut conflicting = first.clone();
        conflicting.text = "conflicting".into();
        let error = store
            .put_reasoning_summaries(&[fresh.clone(), conflicting])
            .await
            .unwrap_err();
        ensure!(
            error.downcast_ref::<ReasoningSummaryConflict>().is_some(),
            "different payload for a settled summary identity was not a typed conflict: {error:#}"
        );
        let first_key = reasoning_summary_key(&first)?;
        let fresh_key = reasoning_summary_key(&fresh)?;
        ensure!(
            store.get(&first_key).await? == Some(serde_json::to_value(&first)?),
            "idempotent reasoning summary changed its original durable payload"
        );
        ensure!(
            store.get(&fresh_key).await?.is_none(),
            "summary batch committed a prefix before its later identity conflict"
        );
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn private_reasoning_summary_identity_keeps_equal_text_in_its_admitted_tuple()
    -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let first = reasoning_summary("same provider text");
        let mut other_session = first.clone();
        other_session.session_id = "other-session".into();
        let mut other_actor = first.clone();
        other_actor.actor_id = "other-actor".into();
        let mut other_turn = first.clone();
        other_turn.turn_id = Some("other-turn".into());
        let mut other_invocation = first.clone();
        other_invocation.invocation_id = "other-invocation".into();
        let records = [
            first.clone(),
            other_session.clone(),
            other_actor.clone(),
            other_turn.clone(),
            other_invocation.clone(),
        ];
        store.put_reasoning_summaries(&records).await?;

        let keys = records
            .iter()
            .map(reasoning_summary_key)
            .collect::<Result<std::collections::BTreeSet<_>>>()?;
        ensure!(
            keys.len() == records.len(),
            "distinct admitted reasoning-summary identities shared one durable key"
        );
        for record in records {
            let key = reasoning_summary_key(&record)?;
            ensure!(
                store.get(&key).await? == Some(serde_json::to_value(record)?),
                "reasoning summary identity did not retain its exact admitted record"
            );
        }
        store.close().await?;
        Ok(())
    }

    #[test]
    fn private_reasoning_summary_batch_bounds_total_identity_and_text_bytes() -> Result<()> {
        let mut at_limit = reasoning_summary("");
        let identity_bytes = at_limit.session_id.len()
            + at_limit.turn_id.as_ref().map_or(0, String::len)
            + at_limit.operation_id.as_ref().map_or(0, String::len)
            + at_limit.actor_id.len()
            + at_limit.invocation_id.len()
            + at_limit.item_id.as_ref().map_or(0, String::len);
        at_limit.text = "x".repeat(MAX_REASONING_SUMMARY_BATCH_STRING_BYTES - identity_bytes);
        encode_reasoning_summaries(&[at_limit.clone()])?;
        at_limit.text.push('x');
        ensure!(
            encode_reasoning_summaries(&[at_limit]).is_err(),
            "reasoning summary aggregate accepted one byte over its 16 MiB bound"
        );
        Ok(())
    }

    #[test]
    fn legacy_summary_serialization_and_identities_remain_byte_exact() -> Result<()> {
        let reasoning = reasoning_summary("legacy summary");
        assert_eq!(
            serde_json::to_string(&reasoning)?,
            r#"{"session_id":"session","turn_id":"turn","actor_id":"actor","invocation_id":"invocation","item_id":"item","output_index":0,"summary_index":0,"text":"legacy summary"}"#
        );
        let old_reasoning_digest = Sha256::digest(serde_json::to_vec(&(
            &reasoning.session_id,
            "turn",
            &reasoning.actor_id,
            &reasoning.invocation_id,
            &reasoning.item_id,
            reasoning.output_index,
            reasoning.summary_index,
        ))?);
        let old_reasoning_key = format!(
            "{PRIVATE_REASONING_SUMMARY_PREFIX}{}",
            old_reasoning_digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        assert_eq!(reasoning_summary_key(&reasoning)?, old_reasoning_key);

        let context = ContextSummaryRecord {
            actor_namespace: "actor namespace".into(),
            session_id: "session".into(),
            source_namespace: "source namespace".into(),
            summary_namespace: "summary namespace".into(),
            source_view: "main".into(),
            source_revision: "a".repeat(64),
            after_sequence: 0,
            through_sequence: 7,
            turn_id: Some("turn".into()),
            operation_id: None,
            producer_actor_id: None,
            invocation_id: "invocation".into(),
            summary: "legacy context".into(),
        };
        assert_eq!(
            serde_json::to_string(&context)?,
            format!(
                r#"{{"actor_namespace":"actor namespace","session_id":"session","source_namespace":"source namespace","summary_namespace":"summary namespace","source_view":"main","source_revision":"{}","after_sequence":0,"through_sequence":7,"turn_id":"turn","invocation_id":"invocation","summary":"legacy context"}}"#,
                "a".repeat(64)
            )
        );
        let mut old_context_digest = Sha256::new();
        for value in [
            "kuru.context-summary.v1",
            &context.actor_namespace,
            &context.session_id,
            &context.source_namespace,
            &context.summary_namespace,
            &context.source_view,
            &context.source_revision,
            "turn",
            &context.invocation_id,
        ] {
            hash_context_field(&mut old_context_digest, value.as_bytes());
        }
        hash_context_field(
            &mut old_context_digest,
            &context.after_sequence.to_be_bytes(),
        );
        hash_context_field(
            &mut old_context_digest,
            &context.through_sequence.to_be_bytes(),
        );
        let old_context_id = old_context_digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(context_summary_id(&context)?, old_context_id);
        assert_eq!(context_summary_format(&context)?, CONTEXT_SUMMARY_FORMAT);

        let operation_reasoning = ReasoningSummaryRecord {
            turn_id: None,
            operation_id: Some("operation".into()),
            ..reasoning.clone()
        };
        assert_ne!(
            reasoning_summary_key(&operation_reasoning)?,
            old_reasoning_key
        );
        let operation_context = ContextSummaryRecord {
            turn_id: None,
            operation_id: Some("operation".into()),
            producer_actor_id: Some("producer".into()),
            ..context
        };
        assert_ne!(context_summary_id(&operation_context)?, old_context_id);
        assert_eq!(
            context_summary_format(&operation_context)?,
            CONTEXT_SUMMARY_OPERATION_FORMAT
        );
        Ok(())
    }

    #[tokio::test]
    async fn compact_checkpoint_sidecars_are_atomic_on_stale_and_late_conflict() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().join("private"),
            format!("project/{}", "7".repeat(64)),
        )?;
        let store = MemoryStore::open(options.clone()).await?;
        let actor_namespace = "project/example/actor";
        let producer_actor = "actor-id";
        let session_id = "session";
        let operation_id = "compact-operation";
        let invocation_id = "compact-invocation";
        store
            .append_session_message(
                actor_namespace,
                session_id,
                &Message::text("user", "raw source"),
            )
            .await?;

        let existing = ReasoningSummaryRecord {
            session_id: session_id.into(),
            turn_id: None,
            operation_id: Some(operation_id.into()),
            actor_id: producer_actor.into(),
            invocation_id: invocation_id.into(),
            item_id: Some("item".into()),
            output_index: Some(0),
            summary_index: 0,
            text: "settled existing sidecar".into(),
        };
        store
            .put_reasoning_summaries(std::slice::from_ref(&existing))
            .await?;
        let existing_key = reasoning_summary_key(&existing)?;
        let fresh = (1..1024)
            .map(|summary_index| ReasoningSummaryRecord {
                summary_index,
                text: "fresh sidecar".into(),
                ..existing.clone()
            })
            .find(|record| {
                reasoning_summary_key(record).is_ok_and(|candidate| candidate < existing_key)
            })
            .context("fixture could not order a fresh sidecar before the conflict")?;
        let fresh_key = reasoning_summary_key(&fresh)?;
        let mut conflicting = existing.clone();
        conflicting.text = "conflicting settled payload".into();

        let snapshot = store
            .session_source_snapshot(actor_namespace, session_id, actor_namespace, 0, 16)
            .await?;
        let through = snapshot
            .through_inclusive
            .context("compact source snapshot omitted its boundary")?;
        let mut checkpoint = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                actor_namespace: actor_namespace.into(),
                session_id: session_id.into(),
                source_namespace: actor_namespace.into(),
                summary_namespace: format!("{actor_namespace}/session/{session_id}/summaries"),
                source_view: snapshot.view,
                source_revision: snapshot.revision,
                after_sequence: 0,
                through_sequence: through,
                turn_id: None,
                operation_id: Some(operation_id.into()),
                producer_actor_id: Some(producer_actor.into()),
                invocation_id: invocation_id.into(),
                summary: "compact context".into(),
            },
            private_reasoning: vec![fresh.clone(), conflicting],
        };

        sqlx::query("UPDATE messages SET content = ? WHERE namespace = ? AND session_id = ? AND sequence = ?")
            .bind(encode_typed_message(&Message::text("user", "changed raw source"))?)
            .bind(actor_namespace.as_bytes())
            .bind(session_id.as_bytes())
            .bind(through)
            .execute(store.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind("change compact checkpoint source")
            .bind(AUTHOR)
            .fetch_all(store.pool.as_ref())
            .await?;
        let stale = store
            .checkpoint_context_summary(&checkpoint)
            .await
            .expect_err("stale compact checkpoint was accepted");
        assert!(stale.downcast_ref::<ContextSummaryStale>().is_some());
        assert!(store.get(&fresh_key).await?.is_none());

        checkpoint.record.source_revision = store
            .session_source_snapshot(actor_namespace, session_id, actor_namespace, 0, 16)
            .await?
            .revision;
        let conflict = store
            .checkpoint_context_summary(&checkpoint)
            .await
            .expect_err("later sidecar conflict committed a prefix");
        assert!(
            conflict
                .downcast_ref::<ReasoningSummaryConflict>()
                .is_some()
        );
        assert!(store.get(&fresh_key).await?.is_none());
        assert!(
            store
                .context_summary_cursor(actor_namespace, session_id, actor_namespace)
                .await?
                .is_none()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM context_summaries")
                .fetch_one(store.pool.as_ref())
                .await?,
            0
        );

        checkpoint.private_reasoning = vec![fresh.clone(), existing.clone()];
        store.checkpoint_context_summary(&checkpoint).await?;
        assert_eq!(
            store.get(&fresh_key).await?,
            Some(serde_json::to_value(&fresh)?)
        );
        assert_eq!(
            store.get(&existing_key).await?,
            Some(serde_json::to_value(&existing)?)
        );
        let current = store
            .context_summary_window(
                actor_namespace,
                &checkpoint.record.summary_namespace,
                Some(session_id),
                Some(actor_namespace),
                16,
            )
            .await?;
        assert_eq!(current.records.len(), 1);
        assert_eq!(current.records[0].record, checkpoint.record);
        let format: String =
            sqlx::query_scalar("SELECT record_format FROM context_summaries LIMIT 1")
                .fetch_one(store.pool.as_ref())
                .await?;
        assert_eq!(format, CONTEXT_SUMMARY_OPERATION_FORMAT);
        store.close().await?;

        let reopened = MemoryStore::open(options).await?;
        assert_eq!(
            reopened.get(&fresh_key).await?,
            Some(serde_json::to_value(&fresh)?)
        );
        assert_eq!(
            reopened.get(&existing_key).await?,
            Some(serde_json::to_value(&existing)?)
        );
        let reopened_current = reopened
            .context_summary_window(
                actor_namespace,
                &checkpoint.record.summary_namespace,
                Some(session_id),
                Some(actor_namespace),
                16,
            )
            .await?;
        assert_eq!(reopened_current.records.len(), 1);
        assert_eq!(reopened_current.records[0].record, checkpoint.record);
        reopened.close().await
    }

    #[test]
    fn session_source_budget_rejects_the_next_row_or_byte_without_partial_accounting() -> Result<()>
    {
        let mut row_budget = SessionSourceBudget::new(2);
        assert!(row_budget.try_include(1)?);
        assert!(row_budget.try_include(1)?);
        assert!(!row_budget.try_include(1)?);
        assert_eq!(row_budget.rows, 2);
        assert_eq!(row_budget.serialized_bytes, 2);

        let mut byte_budget = SessionSourceBudget::new(MAX_SESSION_SOURCE_ROWS);
        assert!(byte_budget.try_include(MAX_SESSION_SOURCE_BYTES - 1)?);
        assert!(!byte_budget.try_include(2)?);
        assert_eq!(byte_budget.rows, 1);
        assert_eq!(byte_budget.serialized_bytes, MAX_SESSION_SOURCE_BYTES - 1);
        assert!(byte_budget.try_include(1)?);
        assert_eq!(byte_budget.serialized_bytes, MAX_SESSION_SOURCE_BYTES);

        let serialized_row = serialized_session_source_row_bytes(&SequencedMessage {
            sequence: 1,
            message: Message::text("user", "bounded row"),
        })?;
        let mut serialized_budget = SessionSourceBudget::new(MAX_SESSION_SOURCE_ROWS);
        assert!(
            serialized_budget.try_include(
                MAX_SESSION_SOURCE_BYTES
                    .checked_sub(serialized_row)
                    .context("serialized row exceeds the source budget")?
                    + 1,
            )?
        );
        assert!(!serialized_budget.try_include(serialized_row)?);
        assert_eq!(serialized_budget.rows, 1);
        Ok(())
    }

    #[tokio::test]
    async fn service_disconnect_and_owner_restart_preserve_unresolved_candidate() -> Result<()> {
        // Real lifecycles: owner, local open, successor owner and final local
        // reopen.
        let deadline = crate::test_support::fixture_deadline(4);
        tokio::time::timeout(deadline, async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = sha2::Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let options = crate::test_support::open_options(root.path().join("private"), scope)?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = service::ServiceOwner::open(options.clone(), &project).await?;
            let served = tokio::spawn(owner.serve());
            let mut client = service::attach_existing(&options, &project)
                .await?
                .context("service endpoint did not admit a candidate client")?;
            let ServiceValue::CandidateStarted { handle, .. } = client
                .call(ServiceCall::BeginCandidate {
                    label: "disconnect-dream".into(),
                })
                .await?
            else {
                bail!("service did not return the candidate identity");
            };
            ensure!(
                matches!(
                    client
                        .call(ServiceCall::View {
                            candidate: Some(handle),
                            operation: Box::new(ViewOperation::PutMany {
                                values: vec![("dream-private".into(), json!("retained"))],
                            }),
                        })
                        .await?,
                    ServiceValue::Unit
                ),
                "service did not accept the candidate write"
            );
            drop(client);
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("first owner did not reap after candidate client disconnected")???;
            drop(permit);

            let first = MemoryStore::open(options.clone()).await?;
            let candidate_refs = candidate_refs_with_value(&first).await?;
            ensure!(
                candidate_refs.len() == 1,
                "candidate ref was deleted on disconnect"
            );
            first.close().await?;

            let successor = service::ServiceOwner::open(options.clone(), &project).await?;
            successor.close().await?;
            let reopened = MemoryStore::open(options).await?;
            ensure!(
                candidate_refs_with_value(&reopened).await? == candidate_refs,
                "owner restart changed the unresolved candidate ref, head or private rows"
            );
            reopened.close().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("candidate disconnect/restart fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    async fn candidate_refs_with_value(
        store: &MemoryStore,
    ) -> Result<Vec<(String, String, String)>> {
        let refs: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, hash FROM dolt_branches WHERE LEFT(BINARY name, ?) = BINARY ? ORDER BY BINARY name LIMIT 3",
        )
        .bind(CANDIDATE_PREFIX.len() as i64)
        .bind(CANDIDATE_PREFIX)
        .fetch_all(store.pool.as_ref())
        .await?;
        let mut observed = Vec::new();
        for (name, head) in refs {
            let pool = store.shared.server.pool(&name).await?;
            let value: String = sqlx::query_scalar("SELECT value FROM state WHERE `key` = ?")
                .bind(b"dream-private".as_slice())
                .fetch_one(pool.as_ref())
                .await?;
            observed.push((name, head, value));
            pool.close().await;
        }
        Ok(observed)
    }

    #[tokio::test]
    async fn observed_open_reports_ready_only_after_a_usable_store() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().to_owned(),
            format!("project/{}", "d".repeat(64)),
        )?;
        let (mut progress, opening) = MemoryStore::open_observed(options);
        let store = opening.await?;
        assert!(!store.revision().await?.is_empty());
        store.close().await?;

        let mut stages = Vec::new();
        while let Some(stage) = progress.recv().await {
            stages.push(stage);
        }
        assert_eq!(
            stages.first(),
            Some(&MemoryOpenStage::WaitingForProjectOwnership)
        );
        assert!(stages.contains(&MemoryOpenStage::PreparingDatabase));
        assert!(stages.contains(&MemoryOpenStage::OpeningDatabase));
        assert_eq!(stages.last(), Some(&MemoryOpenStage::Ready));
        assert_eq!(
            stages
                .iter()
                .filter(|stage| **stage == MemoryOpenStage::Ready)
                .count(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn dropping_open_observer_does_not_cancel_the_store() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().to_owned(),
            format!("project/{}", "e".repeat(64)),
        )?;
        let fixture_options = options.clone();
        let (progress, opening) = MemoryStore::open_observed(options);
        drop(progress);
        let store = opening
            .await
            .map_err(|error| crate::test_support::fixture_startup_error(&fixture_options, error))?;
        assert!(!store.revision().await?.is_empty());
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn failed_observed_open_never_reports_ready() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let mut options = OpenOptions::new(
            root.path().to_owned(),
            format!("project/{}", "f".repeat(64)),
        );
        options.config.startup_timeout_secs = 0;
        let (mut progress, opening) = MemoryStore::open_observed(options);
        assert!(opening.await.is_err());
        let mut stages = Vec::new();
        while let Some(stage) = progress.recv().await {
            stages.push(stage);
        }
        assert!(!stages.contains(&MemoryOpenStage::Ready));
        Ok(())
    }

    #[tokio::test]
    async fn explicit_close_rejects_reads_and_writes_through_retained_clone() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let options = crate::test_support::open_options(
            root.path().to_owned(),
            format!("project/{}", "c".repeat(64)),
        )?;
        let store = MemoryStore::open(options.clone()).await?;
        store.append("close", "user", "before shutdown").await?;
        let released = store.clone();
        drop(released);
        store.append("close", "user", "after view drop").await?;
        let retained = store.clone();
        store.close().await?;

        for error in [
            retained
                .history("close", 10)
                .await
                .expect_err("closed history"),
            retained.get("close").await.expect_err("closed state read"),
            retained.revision().await.expect_err("closed revision read"),
            retained
                .revisions(10)
                .await
                .expect_err("closed revisions read"),
            retained.status().await.expect_err("closed status read"),
            retained
                .reconcile()
                .await
                .expect_err("closed reconciliation"),
            retained
                .append("close", "user", "after shutdown")
                .await
                .expect_err("closed append"),
            retained
                .put("close", &json!(true))
                .await
                .expect_err("closed state write"),
        ] {
            assert_eq!(error.to_string(), "memory store is closed");
        }
        drop(retained);

        let reopened = MemoryStore::open(options).await?;
        reopened.append("close", "user", "after reopen").await?;
        assert_eq!(
            reopened
                .history("close", 10)
                .await?
                .last()
                .expect("reopened history")
                .plain_text(),
            Some("after reopen")
        );
        reopened.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn revisions_use_graph_order_when_ancestor_dates_tie() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let date = "2025-01-02T03:04:05Z";
        let first_key = format!("graph-order-first-{}", Uuid::new_v4());
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(first_key.as_bytes())
            .bind("true")
            .execute(store.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?, '--date', ?)")
            .bind("graph-order ancestor")
            .bind(AUTHOR)
            .bind(date)
            .fetch_all(store.pool.as_ref())
            .await?;
        let ancestor = store.revision().await?;

        let descendant_key = format!("graph-order-descendant-{}", Uuid::new_v4());
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(descendant_key.as_bytes())
            .bind("true")
            .execute(store.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?, '--date', ?)")
            .bind("graph-order descendant")
            .bind(AUTHOR)
            .bind(date)
            .fetch_all(store.pool.as_ref())
            .await?;
        let descendant = store.revision().await?;

        let rows = sqlx::query(
            "SELECT commit_hash, commit_order, CAST(date AS CHAR) AS date FROM dolt_log WHERE commit_hash IN (?, ?)",
        )
        .bind(&ancestor)
        .bind(&descendant)
        .fetch_all(store.pool.as_ref())
        .await?;
        assert_eq!(rows.len(), 2, "pinned Dolt must expose both graph entries");
        let ancestor_row = rows
            .iter()
            .find(|row| {
                row.try_get::<String, _>("commit_hash")
                    .is_ok_and(|hash| hash == ancestor)
            })
            .expect("ancestor log row");
        let descendant_row = rows
            .iter()
            .find(|row| {
                row.try_get::<String, _>("commit_hash")
                    .is_ok_and(|hash| hash == descendant)
            })
            .expect("descendant log row");
        assert_eq!(
            ancestor_row.try_get::<String, _>("date")?,
            descendant_row.try_get::<String, _>("date")?,
            "fixture commits must tie on their wall-clock date"
        );
        assert!(
            descendant_row.try_get::<u64, _>("commit_order")?
                > ancestor_row.try_get::<u64, _>("commit_order")?,
            "pinned Dolt must order a descendant before its tied-date ancestor"
        );

        let revisions = store.revisions(2).await?;
        assert_eq!(
            revisions
                .iter()
                .map(|revision| revision.hash.as_str())
                .collect::<Vec<_>>(),
            vec![descendant.as_str(), ancestor.as_str()],
            "public revision order must follow pinned Dolt graph order before the hash tie-break"
        );
        store.close().await?;
        Ok(())
    }

    /// Materialize a released v1 store without going through `open`: production
    /// writable open intentionally upgrades it immediately, so this fixture
    /// must stop a real v1 server and preserve the ordinary format-1 marker.
    pub(super) async fn released_v1(options: &OpenOptions) -> Result<()> {
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        private_dir(&options.data_dir)?;
        let parent = directory
            .parent()
            .context("released fixture has no parent")?;
        private_dir(parent)?;
        private_dir(&directory)?;
        let binary =
            provision::provision(&options.config, &options.data_dir.join("tools/dolt")).await?;
        let server = Server::open(ServerOptions {
            binary,
            directory: directory.clone(),
            project_scope: options.project_scope.clone(),
            supervisor: options
                .supervisor
                .clone()
                .context("released fixture needs supervisor")?,
            timeout: Duration::from_secs(options.config.startup_timeout_secs),
            read_only: false,
            retained: None,
            lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
        })
        .await?;
        let pool = server.pool("main").await?;
        initialize(&pool).await?;
        let activation = Activation {
            format: 1,
            project_scope: options.project_scope.clone(),
            initial_revision: revision(&pool).await?,
            migration: None,
        };
        write_json(&directory.join("ready.json"), &activation)?;
        pool.close().await;
        server.close().await?;
        Ok(())
    }

    pub(super) async fn released_server(options: &OpenOptions) -> Result<Server> {
        let directory = project_directory(&options.data_dir, &options.project_scope)?;
        released_server_at(options, directory).await
    }

    async fn released_server_at(options: &OpenOptions, directory: PathBuf) -> Result<Server> {
        Server::open(ServerOptions {
            binary: provision::provision(&options.config, &options.data_dir.join("tools/dolt"))
                .await?,
            directory,
            project_scope: options.project_scope.clone(),
            supervisor: options
                .supervisor
                .clone()
                .context("released fixture needs supervisor")?,
            timeout: Duration::from_secs(options.config.startup_timeout_secs),
            read_only: false,
            retained: None,
            lifecycle_root: cfg!(windows).then(|| options.data_dir.join("memory/lifecycles")),
        })
        .await
    }

    async fn stopped_ready_v1_stage(
        options: &OpenOptions,
    ) -> Result<(PathBuf, PathBuf, Vec<u8>, String)> {
        released_v1(options).await?;
        let active = project_directory(&options.data_dir, &options.project_scope)?;
        let marker = fs::read(active.join("ready.json"))?;
        let server = released_server(options).await?;
        let pool = server.pool("main").await?;
        let head = revision(&pool).await?;
        pool.close().await;
        server.close().await?;
        let stage = active.with_file_name(format!(
            "{}.staging-{}",
            active
                .file_name()
                .context("released v1 path has no name")?
                .to_string_lossy(),
            Uuid::new_v4()
        ));
        fs::rename(&active, &stage)?;
        Ok((active, stage, marker, head))
    }

    #[tokio::test]
    async fn ready_released_v1_stage_activates_its_original_marker_then_upgrades() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "9".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
        let (active, stage, _initial_marker, _base) = stopped_ready_v1_stage(&options).await?;
        // Model a released v1 stage whose first activation already contained
        // durable user state. The marker is written once with that exact v1
        // head, then the production opener must preserve it byte-for-byte.
        let server = released_server_at(&options, stage.clone()).await?;
        let pool = server.pool("main").await?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(b"ready-v1-state".as_slice())
            .bind("{\"retained\":true}")
            .execute(pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'released v1 ready payload', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        let base = revision(&pool).await?;
        write_json(
            &stage.join("ready.json"),
            &Activation {
                format: 1,
                project_scope: options.project_scope.clone(),
                initial_revision: base.clone(),
                migration: None,
            },
        )?;
        pool.close().await;
        server.close().await?;
        let marker = fs::read(stage.join("ready.json"))?;

        let store = MemoryStore::open(options.clone()).await?;
        assert!(!stage.exists());
        assert_eq!(fs::read(active.join("ready.json"))?, marker);
        assert_eq!(
            migrations::version(&store.pool).await?,
            migrations::CURRENT_VERSION
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT value FROM state WHERE `key` = ?")
                .bind(b"ready-v1-state".as_slice())
                .fetch_one(store.pool.as_ref())
                .await?,
            "{\"retained\":true}"
        );
        let upgraded = store.revision().await?;
        let parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&upgraded)
        .fetch_one(store.pool.as_ref())
        .await?;
        let grandparent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&parent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let great_grandparent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&grandparent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let fourth_parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&great_grandparent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let fifth_parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&fourth_parent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let sixth_parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&fifth_parent)
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(
            sixth_parent, base,
            "v1 to v7 must contain six ordered upgrades"
        );
        let commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'",
        )
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(commits, 1);
        let typed_commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 3%'",
        )
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(typed_commits, 1);
        let receipt_commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 4%'",
        )
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(receipt_commits, 1);
        let session_commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 5%'",
        )
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(session_commits, 1);
        let provenance_commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 6%'",
        )
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(provenance_commits, 1);
        let public_turn_commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 7%'",
        )
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(public_turn_commits, 1);
        store.close().await?;

        let reopened = MemoryStore::open(options).await?;
        assert_eq!(reopened.revision().await?, upgraded);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT value FROM state WHERE `key` = ?")
                .bind(b"ready-v1-state".as_slice())
                .fetch_one(reopened.pool.as_ref())
                .await?,
            "{\"retained\":true}"
        );
        let commits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dolt_log WHERE message LIKE 'Upgrade Kuru memory schema 2%'",
        )
        .fetch_one(reopened.pool.as_ref())
        .await?;
        assert_eq!(commits, 1, "reopen must not add a second upgrade commit");
        reopened.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn ready_released_v1_stage_rejects_dirty_or_newer_attempts() -> Result<()> {
        for (case, target, future_schema) in [
            ("dirty", None, false),
            ("v2 attempt", Some(2), false),
            ("v3 attempt", Some(3), false),
            ("v4 attempt", Some(4), false),
            ("v5 attempt", Some(5), false),
            ("v6 attempt", Some(6), false),
            ("v7 attempt", Some(7), false),
            ("future schema", None, true),
        ] {
            let root = crate::test_support::tempdir()?;
            let scope = format!("project/{}", "d".repeat(64));
            let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
            let (active, stage, marker, base) = stopped_ready_v1_stage(&options).await?;
            let server = released_server_at(&options, stage.clone()).await?;
            let pool = server.pool("main").await?;
            let attempt = match target {
                Some(target) => {
                    let name = format!("kuru_migration_v{target:010}_{}", Uuid::new_v4().simple());
                    sqlx::query("CALL DOLT_BRANCH(?, ?)")
                        .bind(&name)
                        .bind(&base)
                        .fetch_all(pool.as_ref())
                        .await?;
                    Some(name)
                }
                None if future_schema => {
                    sqlx::query("UPDATE kuru_schema SET version = 8 WHERE id = 1")
                        .execute(pool.as_ref())
                        .await?;
                    None
                }
                None => {
                    sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
                        .bind(b"dirty".as_slice())
                        .bind("true")
                        .execute(pool.as_ref())
                        .await?;
                    None
                }
            };
            pool.close().await;
            server.close().await?;

            let error = MemoryStore::open(options.clone()).await.unwrap_err();
            let rendered = format!("{error:#}");
            let expected = match (target, future_schema) {
                (None, false) => "uncommitted changes",
                (Some(2), false) => "attempt newer than its schema",
                (Some(3), false) => "attempt newer than its schema",
                (Some(4), false) => "attempt newer than its schema",
                (Some(5), false) => "attempt newer than its schema",
                (Some(6), false) => "attempt newer than its schema",
                (Some(7), false) => "attempt newer than its schema",
                (None, true) => "unsupported Dolt memory schema version 8",
                _ => unreachable!(),
            };
            assert!(rendered.contains(expected), "{case}: {rendered}");
            assert!(!active.exists(), "invalid ready stage must not activate");
            assert_eq!(fs::read(stage.join("ready.json"))?, marker);
            let inspector = released_server_at(&options, stage.clone()).await?;
            let inspected = inspector.pool("main").await?;
            assert_eq!(
                revision(&inspected).await?,
                base,
                "{case} changed stage HEAD"
            );
            let status: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
                .fetch_one(inspected.as_ref())
                .await?;
            if target.is_some() {
                assert_eq!(status, 0, "{case} must retain a clean main working set");
                let name = attempt.as_ref().expect("attempt name");
                assert_eq!(
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM dolt_branches WHERE name = ?"
                    )
                    .bind(name)
                    .fetch_one(inspected.as_ref())
                    .await?,
                    1,
                    "{case} must retain its canonical branch for inspection"
                );
            } else {
                assert!(status > 0, "{case} must retain its rejected dirty shape");
            }
            inspected.close().await;
            inspector.close().await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn stopped_v1_candidate_survives_upgrade_and_stays_stale() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "e".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
        released_v1(&options).await?;
        let server = released_server(&options).await?;
        let main = server.pool("main").await?;
        sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
            .bind(b"main".as_slice())
            .bind(b"user".as_slice())
            .bind("v1 main")
            .execute(main.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'v1 main', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(main.as_ref())
            .await?;
        let base = revision(&main).await?;
        let branch = format!("candidate_{}", Uuid::new_v4().simple());
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind(&branch)
            .bind(&base)
            .fetch_all(main.as_ref())
            .await?;
        let candidate_pool = server.pool(&branch).await?;
        sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
            .bind(b"candidate".as_slice())
            .bind(b"assistant".as_slice())
            .bind("v1 only")
            .execute(candidate_pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'v1 candidate', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(candidate_pool.as_ref())
            .await?;
        let candidate_head = revision(&candidate_pool).await?;
        candidate_pool.close().await;
        main.close().await;
        server.close().await?;

        let store = MemoryStore::open(options.clone()).await?;
        let old_pool = store.shared.server.pool(&branch).await?;
        assert_eq!(revision(&old_pool).await?, candidate_head);
        let old = MemoryStore {
            shared: store.shared.clone(),
            pool: old_pool.clone(),
            branch: branch.clone(),
            logical_receipt: None,
        };
        sqlx::query("CREATE TABLE historical_dirty_probe (id INT PRIMARY KEY)")
            .execute(old_pool.as_ref())
            .await?;
        let old_before = inspection_snapshot(&old_pool).await?;
        assert_eq!(
            old.history("candidate", 10).await?[0].plain_text(),
            Some("v1 only")
        );
        assert_eq!(inspection_snapshot(&old_pool).await?, old_before);
        assert_eq!(migrations::version(&old_pool).await?, 1);
        let stale = Candidate {
            live: store.clone(),
            view: old.clone(),
            base,
            promoted: Arc::new(StdMutex::new(None)),
        };
        assert!(stale.promote().await.is_err());
        assert_eq!(revision(&old_pool).await?, candidate_head);
        assert_eq!(inspection_snapshot(&old_pool).await?, old_before);
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT hash FROM dolt_branches WHERE name = ?")
                .bind(&branch)
                .fetch_one(store.pool.as_ref())
                .await?,
            candidate_head
        );
        let fresh = store.begin_candidate("current").await?;
        fresh
            .view()
            .append("candidate", "assistant", "v2 current")
            .await?;
        fresh.promote().await?;
        store.append("main", "assistant", "later").await?;
        let head = store.revision().await?;
        old_pool.close().await;
        store.close().await?;

        let reopened = MemoryStore::open(options).await?;
        assert_eq!(reopened.revision().await?, head);
        assert_eq!(
            reopened
                .revisions(20)
                .await?
                .iter()
                .filter(|r| r.message.starts_with("Upgrade Kuru memory schema 2"))
                .count(),
            1
        );
        assert_eq!(
            reopened
                .history("main", 10)
                .await?
                .iter()
                .map(|m| m.plain_text().expect("text fixture"))
                .collect::<Vec<_>>(),
            ["v1 main", "later"]
        );
        assert_eq!(
            reopened.history("candidate", 10).await?[0].plain_text(),
            Some("v2 current"),
            "the successful current candidate promotion must publish its data"
        );
        let preserved = reopened.shared.server.pool(&branch).await?;
        let preserved_view = MemoryStore {
            shared: reopened.shared.clone(),
            pool: preserved.clone(),
            branch,
            logical_receipt: None,
        };
        assert_eq!(revision(&preserved).await?, candidate_head);
        assert_eq!(migrations::version(&preserved).await?, 1);
        assert_eq!(
            preserved_view.history("candidate", 10).await?[0].plain_text(),
            Some("v1 only")
        );
        preserved.close().await;
        reopened.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn stopped_released_v1_upgrades_once_and_readonly_preserves_it() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "b".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        released_v1(&options).await?;
        let directory = project_directory(root.path(), &scope)?;
        let marker = fs::read(directory.join("ready.json"))?;
        let server = released_server(&options).await?;
        let pool = server.pool("main").await?;
        sqlx::query("INSERT INTO messages (namespace, role, content) VALUES (?, ?, ?)")
            .bind(vec![0, 0xff, b'n'])
            .bind(vec![0x80, b'r'])
            .bind("released \u{1f642} message")
            .execute(pool.as_ref())
            .await?;
        sqlx::query("INSERT INTO state (`key`, value) VALUES (?, ?)")
            .bind(vec![0, 0xfe, b'k'])
            .bind("{\"released\":true,\"emoji\":\"🙂\"}")
            .execute(pool.as_ref())
            .await?;
        let operation = Uuid::new_v4().hyphenated().to_string();
        sqlx::query("INSERT INTO operations (id, label) VALUES (?, ?)")
            .bind(&operation)
            .bind("released operation")
            .execute(pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'released v1 payload', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        let base = revision(&pool).await?;
        let messages: Vec<(i64, Vec<u8>, Vec<u8>, String)> = sqlx::query_as(
            "SELECT sequence, namespace, role, content FROM messages ORDER BY sequence",
        )
        .fetch_all(pool.as_ref())
        .await?;
        let state: Vec<(Vec<u8>, String)> =
            sqlx::query_as("SELECT `key`, value FROM state ORDER BY `key`")
                .fetch_all(pool.as_ref())
                .await?;
        let operations: Vec<(String, String)> =
            sqlx::query_as("SELECT id, label FROM operations ORDER BY id")
                .fetch_all(pool.as_ref())
                .await?;
        pool.close().await;
        server.close().await?;

        let mut readonly = options.clone();
        readonly.read_only = true;
        let error = MemoryStore::open(readonly).await.unwrap_err();
        let error = format!("{error:#}");
        assert!(
            error.contains("version 1 requires writable upgrade to 7"),
            "unexpected read-only v1 open error: {error}"
        );
        assert_eq!(fs::read(directory.join("ready.json"))?, marker);
        let inspector = released_server(&options).await?;
        let inspected = inspector.pool("main").await?;
        assert_eq!(revision(&inspected).await?, base);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_status")
                .fetch_one(inspected.as_ref())
                .await?,
            0,
            "a rejected read-only open must not dirty released v1"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dolt_branches")
                .fetch_one(inspected.as_ref())
                .await?,
            1,
            "a rejected read-only open must not create a migration ref"
        );
        inspected.close().await;
        inspector.close().await?;

        let store = MemoryStore::open(options.clone()).await?;
        assert_eq!(
            migrations::version(&store.pool).await?,
            migrations::CURRENT_VERSION
        );
        let first = store.revision().await?;
        let parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&first)
        .fetch_one(store.pool.as_ref())
        .await?;
        let grandparent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&parent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let great_grandparent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&grandparent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let fourth_parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&great_grandparent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let fifth_parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&fourth_parent)
        .fetch_one(store.pool.as_ref())
        .await?;
        let sixth_parent: String = sqlx::query_scalar(
            "SELECT parent_hash FROM dolt_commit_ancestors WHERE commit_hash = ? AND parent_index = 0",
        )
        .bind(&fifth_parent)
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(
            sixth_parent, base,
            "upgrade must retain all six ordered commits"
        );
        assert_eq!(
            sqlx::query_as::<_, (i64, Vec<u8>, Vec<u8>, String)>(
                "SELECT sequence, namespace, role, content FROM messages ORDER BY sequence",
            )
            .fetch_all(store.pool.as_ref())
            .await?,
            messages
        );
        assert_eq!(
            sqlx::query_as::<_, (Vec<u8>, String)>("SELECT `key`, value FROM state ORDER BY `key`")
                .fetch_all(store.pool.as_ref())
                .await?,
            state
        );
        assert_eq!(
            sqlx::query_as::<_, (String, String)>("SELECT id, label FROM operations ORDER BY id")
                .fetch_all(store.pool.as_ref())
                .await?,
            operations
        );
        assert_eq!(fs::read(directory.join("ready.json"))?, marker);
        let receipts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kuru_migrations")
            .fetch_one(store.pool.as_ref())
            .await?;
        assert_eq!(receipts, i64::from(migrations::CURRENT_VERSION - 1));
        let receipt: Vec<(i32, String, String, String)> = sqlx::query_as(
            "SELECT version, id, digest, operation FROM kuru_migrations ORDER BY version",
        )
        .fetch_all(store.pool.as_ref())
        .await?;
        assert_eq!(
            receipt.iter().map(|row| row.0).collect::<Vec<_>>(),
            [2, 3, 4, 5, 6, 7]
        );
        assert!(receipt.iter().all(|row| Uuid::parse_str(&row.3).is_ok()));
        store.close().await?;

        let mut current_readonly = options.clone();
        current_readonly.read_only = true;
        let reader = MemoryStore::open(current_readonly).await?;
        assert_eq!(reader.revision().await?, first);
        reader.close().await?;

        let reopened = MemoryStore::open(options).await?;
        assert_eq!(reopened.revision().await?, first);
        assert_eq!(
            sqlx::query_as::<_, (i64, Vec<u8>, Vec<u8>, String)>(
                "SELECT sequence, namespace, role, content FROM messages ORDER BY sequence",
            )
            .fetch_all(reopened.pool.as_ref())
            .await?,
            messages
        );
        assert_eq!(
            sqlx::query_as::<_, (Vec<u8>, String)>("SELECT `key`, value FROM state ORDER BY `key`")
                .fetch_all(reopened.pool.as_ref())
                .await?,
            state
        );
        assert_eq!(
            sqlx::query_as::<_, (String, String)>("SELECT id, label FROM operations ORDER BY id")
                .fetch_all(reopened.pool.as_ref())
                .await?,
            operations
        );
        assert_eq!(fs::read(directory.join("ready.json"))?, marker);
        let reopened_receipt: Vec<(i32, String, String, String)> = sqlx::query_as(
            "SELECT version, id, digest, operation FROM kuru_migrations ORDER BY version",
        )
        .fetch_all(reopened.pool.as_ref())
        .await?;
        assert_eq!(reopened_receipt, receipt);
        reopened.close().await?;
        Ok(())
    }

    async fn raw_branch(store: &MemoryStore, name: &str) -> Result<()> {
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind(name)
            .bind(store.revision().await?)
            .fetch_all(store.pool.as_ref())
            .await?;
        Ok(())
    }

    async fn inspection_snapshot(
        pool: &MySqlPool,
    ) -> Result<(String, Vec<(String, String)>, Vec<(String, i64, String)>)> {
        Ok((
            revision(pool).await?,
            sqlx::query_as("SELECT name, hash FROM dolt_branches ORDER BY BINARY name LIMIT 65")
                .fetch_all(pool)
                .await?,
            sqlx::query_as(
                "SELECT table_name, staged, status FROM dolt_status ORDER BY BINARY table_name, staged, BINARY status LIMIT 65",
            )
            .fetch_all(pool)
            .await?,
        ))
    }

    #[tokio::test]
    async fn readonly_inspection_preserves_dirty_data_and_rejects_dirty_authority() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "4".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
        let store = MemoryStore::open(options.clone()).await?;
        sqlx::query("CREATE TABLE inspection_dirty_probe (id INT PRIMARY KEY)")
            .execute(store.pool.as_ref())
            .await?;
        let before = inspection_snapshot(&store.pool).await?;
        assert!(
            migrations::validate_active(&store.shared.server, &store.pool)
                .await
                .is_err(),
            "the writable validator must retain its clean-working-set rule"
        );

        let mut readonly = options.clone();
        readonly.read_only = true;
        let attached = MemoryStore::open(readonly.clone()).await?;
        assert_eq!(attached.revision().await?, before.0);
        assert_eq!(inspection_snapshot(&store.pool).await?, before);
        attached.close().await?;
        store.close().await?;

        let stopped = MemoryStore::open(readonly).await?;
        assert_eq!(stopped.revision().await?, before.0);
        assert_eq!(inspection_snapshot(&stopped.pool).await?, before);
        stopped.close().await?;

        for (index, change, expected) in [
            (
                0,
                "ALTER TABLE kuru_schema ADD COLUMN inspection_guard INT NULL",
                "schema or migration receipt authority has uncommitted changes",
            ),
            (
                1,
                "ALTER TABLE kuru_migrations ADD COLUMN inspection_guard INT NULL",
                "schema or migration receipt authority has uncommitted changes",
            ),
        ] {
            let root = crate::test_support::tempdir()?;
            let scope = format!("project/{index:064x}");
            let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
            let store = MemoryStore::open(options.clone()).await?;
            sqlx::query(change).execute(store.pool.as_ref()).await?;
            let before = inspection_snapshot(&store.pool).await?;
            let mut readonly = options.clone();
            readonly.read_only = true;
            let error = MemoryStore::open(readonly)
                .await
                .expect_err("dirty authority was accepted by read-only inspection");
            assert!(format!("{error:#}").contains(expected));
            assert_eq!(inspection_snapshot(&store.pool).await?, before);
            store.close().await?;
        }

        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "5".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
        let store = MemoryStore::open(options.clone()).await?;
        raw_branch(&store, "kuru_migration_bad").await?;
        let before = inspection_snapshot(&store.pool).await?;
        let mut readonly = options.clone();
        readonly.read_only = true;
        let error = MemoryStore::open(readonly)
            .await
            .expect_err("malformed reserved ref was accepted by read-only inspection");
        assert!(format!("{error:#}").contains("reserved migration branch is malformed"));
        assert_eq!(inspection_snapshot(&store.pool).await?, before);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn historical_v1_read_rejects_dirty_dropped_receipt_authority() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "6".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope.clone())?;
        released_v1(&options).await?;
        let directory = project_directory(&options.data_dir, &scope)?;
        let server = released_server(&options).await?;
        let main = server.pool("main").await?;
        let branch = format!("candidate_{}", Uuid::new_v4().simple());
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind(&branch)
            .bind(revision(&main).await?)
            .fetch_all(main.as_ref())
            .await?;
        main.close().await;
        let pool = server.pool(&branch).await?;
        sqlx::query("CREATE TABLE kuru_migrations (malformed INT PRIMARY KEY)")
            .execute(pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', 'malformed v1 receipt authority', '--author', ?)")
            .bind(AUTHOR)
            .fetch_all(pool.as_ref())
            .await?;
        sqlx::query("DROP TABLE kuru_migrations")
            .execute(pool.as_ref())
            .await?;
        let before = inspection_snapshot(&pool).await?;
        assert!(
            before
                .2
                .iter()
                .any(|(table, _, _)| table == "kuru_migrations"),
            "pinned Dolt must expose the dirty receipt drop"
        );
        let history = MemoryStore {
            shared: Arc::new(Shared {
                server,
                directory,
                project_scope: scope,
                fixture_unbound_scope: false,
                read_only: true,
                write: Arc::new(Mutex::new(())),
                dream: Arc::new(Mutex::new(())),
                uncertain: StdMutex::new(None),
                usage_pool: StdMutex::new(None),
                candidate_recovery_pause: None,
                candidate_cleanup_failure: None,
                _permit: None,
            }),
            pool: pool.clone(),
            branch,
            logical_receipt: None,
        };
        let error = history
            .history("missing", 1)
            .await
            .expect_err("dirty-dropped v1 receipt authority was accepted");
        assert!(
            format!("{error:#}")
                .contains("schema or migration receipt authority has uncommitted changes")
        );
        assert_eq!(inspection_snapshot(&pool).await?, before);
        history.close().await?;

        let store = MemoryStore::temporary().await?;
        let candidate = store.begin_candidate("dirty historical receipt").await?;
        let view = candidate.view();
        sqlx::query("ALTER TABLE kuru_migrations ADD COLUMN inspection_guard INT NULL")
            .execute(view.pool.as_ref())
            .await?;
        let before = inspection_snapshot(&view.pool).await?;
        let error = view
            .history("missing", 1)
            .await
            .expect_err("dirty v2 receipt authority was accepted by historical read");
        assert!(
            format!("{error:#}")
                .contains("schema or migration receipt authority has uncommitted changes")
        );
        assert_eq!(inspection_snapshot(&view.pool).await?, before);
        drop(candidate);
        view.pool.close().await;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn migration_inventory_uses_a_literal_lowercase_bounded_prefix() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "c".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
        let store = MemoryStore::open(options.clone()).await?;
        for name in [
            "kuruXmigration_v0000000002_00000000000000000000000000000000",
            "kuru_migrationXv0000000002_00000000000000000000000000000000",
            "KURU_MIGRATION_v0000000002_00000000000000000000000000000000",
        ] {
            raw_branch(&store, name).await?;
        }
        store.close().await?;

        let ignored = MemoryStore::open(options.clone()).await?;
        ignored.close().await?;

        let valid = MemoryStore::open(options.clone()).await?;
        raw_branch(&valid, "kuru_migration_bad").await?;
        valid.close().await?;

        assert!(MemoryStore::open(options).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn migration_inventory_refuses_more_than_sixty_four_literal_attempts() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let scope = format!("project/{}", "d".repeat(64));
        let options = crate::test_support::open_options(root.path().to_owned(), scope)?;
        let store = MemoryStore::open(options.clone()).await?;
        for number in 0..=64u128 {
            raw_branch(&store, &format!("kuru_migration_v0000000002_{number:032x}")).await?;
        }
        store.close().await?;

        let error = MemoryStore::open(options).await.unwrap_err();
        assert!(format!("{error:#}").contains("too many retained Dolt migration attempts"));
        Ok(())
    }

    #[tokio::test]
    async fn migration_validation_rejects_extra_schema_or_receipt_authority() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        sqlx::query("INSERT INTO kuru_schema (id, version) VALUES (2, 2)")
            .execute(store.pool.as_ref())
            .await?;
        assert!(migrations::validate_current(&store.pool).await.is_err());
        store.close().await?;

        let store = MemoryStore::temporary().await?;
        sqlx::query("ALTER TABLE messages DROP INDEX messages_namespace_session_sequence")
            .execute(store.pool.as_ref())
            .await?;
        assert!(migrations::validate_current(&store.pool).await.is_err());
        store.close().await?;

        let store = MemoryStore::temporary().await?;
        sqlx::query("ALTER TABLE context_summary_cursors ADD COLUMN forged BIGINT NULL")
            .execute(store.pool.as_ref())
            .await?;
        assert!(migrations::validate_current(&store.pool).await.is_err());
        store.close().await?;

        let store = MemoryStore::temporary().await?;
        sqlx::query(
            "INSERT INTO kuru_migrations (version, id, digest, operation) VALUES (?, 'forged', ?, ?)",
        )
        .bind(migrations::CURRENT_VERSION + 1)
        .bind("0".repeat(64))
        .bind(Uuid::new_v4().hyphenated().to_string())
        .execute(store.pool.as_ref())
        .await?;
        assert!(migrations::validate_current(&store.pool).await.is_err());
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn open_rejects_invalid_memory_config_before_creating_store_state() {
        for (field, config) in [
            (
                "cache_dir",
                MemoryConfig {
                    cache_dir: Some("relative-cache".into()),
                    ..MemoryConfig::default()
                },
            ),
            (
                "dolt_binary",
                MemoryConfig {
                    dolt_binary: Some("relative-dolt".into()),
                    ..MemoryConfig::default()
                },
            ),
        ] {
            let root = crate::test_support::tempdir().unwrap();
            let data_dir = root.path().join(field).join("not-created");
            let mut options =
                OpenOptions::new(data_dir.clone(), format!("project/{}", "0".repeat(64)));
            options.config = config;

            let error = MemoryStore::open(options).await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("memory.{field} must be an absolute path"))
            );
            assert!(!data_dir.exists());
        }
    }

    #[tokio::test]
    async fn notes_preserve_and_select_a_signed_legacy_sequence() {
        let store = MemoryStore::temporary().await.unwrap();
        let namespace = "project/example/ifs/identity/legacy/notes";
        sqlx::query(
            "INSERT INTO messages (sequence, namespace, role, content) VALUES (?, ?, ?, ?)",
        )
        .bind(-7_i64)
        .bind(namespace.as_bytes())
        .bind(b"dream".as_slice())
        .bind("signed legacy note")
        .execute(store.pool.as_ref())
        .await
        .unwrap();
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind("signed legacy note")
            .bind(AUTHOR)
            .fetch_all(store.pool.as_ref())
            .await
            .unwrap();
        assert_eq!(
            store.notes(namespace, 10).await.unwrap(),
            [StoredNote {
                sequence: -7,
                role: "dream".into(),
                content: "signed legacy note".into(),
            }]
        );
        store.forget_note(namespace, -7).await.unwrap();
        assert!(store.notes(namespace, 10).await.unwrap().is_empty());
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn session_source_snapshot_and_context_checkpoint_preserve_exact_provenance() -> Result<()>
    {
        let store = MemoryStore::temporary().await?;
        let actor = "project/example/ifs/identity/actor";
        store
            .append_message(actor, &Message::text("user", "legacy unattributed"))
            .await?;
        store
            .append_session_message(actor, "session-a", &Message::text("user", "a-one"))
            .await?;
        store
            .append_session_message(actor, "session-b", &Message::text("user", "b-one"))
            .await?;
        store
            .append_session_message(actor, "session-a", &Message::text("assistant", "a-two"))
            .await?;

        let session_window = store.session_history_window(actor, "session-a", 16).await?;
        assert_eq!(session_window.total_rows, 2);
        assert_eq!(
            session_window
                .messages
                .iter()
                .map(Message::plain_text)
                .collect::<Vec<_>>(),
            [Some("a-one"), Some("a-two")]
        );
        let newest = store.session_history_window(actor, "session-a", 1).await?;
        assert_eq!(newest.total_rows, 2);
        assert_eq!(newest.messages[0].plain_text(), Some("a-two"));
        let count_only = store.session_history_window(actor, "session-a", 0).await?;
        assert_eq!(count_only.total_rows, 2);
        assert!(count_only.messages.is_empty());
        assert_eq!(
            store
                .session_history_window(actor, "session-b", 16)
                .await?
                .messages[0]
                .plain_text(),
            Some("b-one")
        );
        assert!(
            store
                .session_history_window(actor, "session-a", MAX_SESSION_SOURCE_ROWS + 1)
                .await
                .is_err()
        );

        assert!(
            store
                .session_source_snapshot(actor, "session-a", actor, 0, 0)
                .await
                .is_err()
        );
        assert!(
            store
                .session_source_snapshot(actor, "session-a", actor, 0, MAX_SESSION_SOURCE_ROWS + 1,)
                .await
                .is_err()
        );
        assert!(
            store
                .session_source_snapshot(actor, " ", actor, 0, 1)
                .await
                .is_err()
        );
        assert!(
            store
                .session_source_snapshot(actor, "session-a", actor, -1, 1)
                .await
                .is_err()
        );

        let snapshot = store
            .session_source_snapshot(actor, "session-a", actor, 0, 16)
            .await?;
        assert_eq!(snapshot.view, "main");
        assert_eq!(snapshot.rows.len(), 2);
        assert_eq!(
            snapshot
                .rows
                .iter()
                .map(|row| row.message.plain_text())
                .collect::<Vec<_>>(),
            [Some("a-one"), Some("a-two")]
        );
        assert!(
            snapshot
                .rows
                .windows(2)
                .all(|rows| rows[0].sequence < rows[1].sequence)
        );
        let through = snapshot
            .through_inclusive
            .context("missing source boundary")?;
        let legacy_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE namespace = ? AND session_id IS NULL",
        )
        .bind(actor.as_bytes())
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(legacy_rows, 1);

        let candidate = store.begin_candidate("session snapshot candidate").await?;
        candidate
            .view()
            .append_session_message(
                actor,
                "session-a",
                &Message::text("assistant", "candidate-only"),
            )
            .await?;
        let candidate_snapshot = candidate
            .view()
            .session_source_snapshot(actor, "session-a", actor, through, 16)
            .await?;
        assert_ne!(candidate_snapshot.view, snapshot.view);
        assert_ne!(candidate_snapshot.revision, snapshot.revision);
        assert_eq!(candidate_snapshot.rows.len(), 1);
        assert_eq!(
            candidate
                .view()
                .session_history_window(actor, "session-a", 16)
                .await?
                .messages
                .iter()
                .map(Message::plain_text)
                .collect::<Vec<_>>(),
            [Some("a-one"), Some("a-two"), Some("candidate-only")]
        );
        assert_eq!(
            store
                .session_history_window(actor, "session-a", 16)
                .await?
                .messages
                .len(),
            2
        );
        assert!(
            store
                .session_source_snapshot(actor, "session-a", actor, through, 16)
                .await?
                .rows
                .is_empty()
        );
        let candidate_suffix = candidate
            .view()
            .session_history_window_after(actor, "session-a", through, 16)
            .await?;
        assert_eq!(candidate_suffix.view, candidate.view().pinned_view());
        assert_eq!(candidate_suffix.total_rows, 1);
        assert_eq!(
            candidate_suffix.rows[0].message.plain_text(),
            Some("candidate-only")
        );
        let main_suffix = store
            .session_history_window_after(actor, "session-a", through, 16)
            .await?;
        assert_eq!(main_suffix.view, "main");
        assert_eq!(main_suffix.total_rows, 0);
        assert!(main_suffix.rows.is_empty());

        let record = ContextSummaryRecord {
            actor_namespace: actor.into(),
            session_id: "session-a".into(),
            source_namespace: actor.into(),
            summary_namespace: format!("{actor}/session/session-a/summaries"),
            source_view: snapshot.view.clone(),
            source_revision: snapshot.revision.clone(),
            after_sequence: snapshot.after_exclusive,
            through_sequence: through,
            turn_id: Some("turn-a".into()),
            operation_id: None,
            producer_actor_id: None,
            invocation_id: "invocation-a".into(),
            summary: "a bounded summary".into(),
        };
        store.put("unrelated/revision-race", &json!(true)).await?;
        assert!(
            record
                .source_revision
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
        let historical_source_count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM messages AS OF '{}' WHERE namespace = ? AND session_id = ? AND sequence > ? AND sequence <= ?",
            record.source_revision
        )))
        .bind(actor.as_bytes())
        .bind("session-a".as_bytes())
        .bind(record.after_sequence)
        .bind(record.through_sequence)
        .fetch_one(store.pool.as_ref())
        .await?;
        assert_eq!(historical_source_count, 2);
        store
            .checkpoint_context_summary(&ContextSummaryCheckpoint {
                record: record.clone(),
                private_reasoning: Vec::new(),
            })
            .await?;
        let cursor = store
            .context_summary_cursor(actor, "session-a", actor)
            .await?
            .context("missing context summary cursor")?;
        assert_eq!(cursor.through_sequence, through);
        assert_eq!(cursor.source_view, "main");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM context_summaries")
                .fetch_one(store.pool.as_ref())
                .await?,
            1
        );
        let format: String =
            sqlx::query_scalar("SELECT record_format FROM context_summaries LIMIT 1")
                .fetch_one(store.pool.as_ref())
                .await?;
        assert_eq!(format, CONTEXT_SUMMARY_FORMAT);

        store
            .append_session_message(
                actor,
                "session-a",
                &Message::text("assistant", "after summary"),
            )
            .await?;
        let later = store
            .session_source_snapshot(actor, "session-a", actor, through, 16)
            .await?;
        let cursor_stale_record = ContextSummaryRecord {
            source_revision: later.revision.clone(),
            after_sequence: 0,
            through_sequence: later
                .through_inclusive
                .context("missing later source boundary")?,
            turn_id: Some("turn-b".into()),
            invocation_id: "invocation-b".into(),
            summary: "must not publish across a stale cursor".into(),
            ..record.clone()
        };
        let stale = store
            .checkpoint_context_summary(&ContextSummaryCheckpoint {
                record: cursor_stale_record.clone(),
                private_reasoning: Vec::new(),
            })
            .await
            .expect_err("a competing cursor accepted an old source boundary");
        assert!(stale.downcast_ref::<ContextSummaryStale>().is_some());
        assert_eq!(
            store
                .context_summary_cursor(actor, "session-a", actor)
                .await?
                .context("missing retained cursor")?
                .through_sequence,
            through
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM context_summaries")
                .fetch_one(store.pool.as_ref())
                .await?,
            1
        );

        let current_record = ContextSummaryRecord {
            source_revision: later.revision,
            after_sequence: through,
            through_sequence: later
                .through_inclusive
                .context("missing current source boundary")?,
            turn_id: Some("turn-c".into()),
            invocation_id: "invocation-c".into(),
            summary: "current rolling summary".into(),
            ..record.clone()
        };
        store
            .checkpoint_context_summary(&ContextSummaryCheckpoint {
                record: current_record.clone(),
                private_reasoning: Vec::new(),
            })
            .await?;
        let current_cursor = store
            .context_summary_cursor(actor, "session-a", actor)
            .await?
            .context("missing advanced context cursor")?;
        let current = store
            .context_summary_window(
                actor,
                &current_record.summary_namespace,
                Some("session-a"),
                Some(actor),
                16,
            )
            .await?;
        assert_eq!(current.view, "main");
        assert_eq!(current.total_rows, 1);
        assert_eq!(current.records.len(), 1);
        assert_eq!(current.records[0].summary_id, current_cursor.summary_id);
        assert_eq!(current.records[0].record, current_record);
        assert!(
            candidate
                .view()
                .context_summary_window(
                    actor,
                    &record.summary_namespace,
                    Some("session-a"),
                    Some(actor),
                    16,
                )
                .await?
                .records
                .is_empty()
        );
        assert_eq!(
            store
                .context_summary_window(actor, &record.summary_namespace, None, None, 16)
                .await?
                .records,
            current.records
        );
        let count_only = store
            .context_summary_window(
                actor,
                &record.summary_namespace,
                Some("session-a"),
                Some(actor),
                0,
            )
            .await?;
        assert_eq!(count_only.total_rows, 1);
        assert!(count_only.records.is_empty());
        assert_eq!(
            store
                .context_summary_window(
                    actor,
                    &record.summary_namespace,
                    Some("session-b"),
                    Some(actor),
                    16,
                )
                .await?
                .total_rows,
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM context_summaries")
                .fetch_one(store.pool.as_ref())
                .await?,
            2
        );

        let export = store.begin_active_export().await?;
        assert_eq!(export.provenance().message_count, 5);
        assert_eq!(export.provenance().context_summary_count, 2);
        assert_eq!(export.provenance().context_cursor_count, 1);
        let mut export_cursor = None;
        let mut exported = Vec::new();
        loop {
            let page = export.page(export_cursor).await?;
            exported.extend(page.records);
            export_cursor = page.next;
            if export_cursor.is_none() {
                break;
            }
        }
        export.verify_counts(5, 1, 2, 1, 0, 0)?;
        assert!(exported.iter().any(|row| matches!(
            row,
            StorageRecord::Message { session_id: None, content, .. }
                if content.contains("legacy unattributed")
        )));
        assert_eq!(
            exported
                .iter()
                .filter(|row| matches!(
                    row,
                    StorageRecord::Message { session_id: Some(session), .. }
                        if session == "session-a"
                ))
                .count(),
            3
        );
        assert!(exported.iter().any(|row| matches!(
            row,
            StorageRecord::ContextSummary { record: exported_record, record_format, .. }
                if exported_record == &record && record_format == CONTEXT_SUMMARY_FORMAT
        )));
        assert!(exported.iter().any(|row| matches!(
            row,
            StorageRecord::ContextSummary { record: exported_record, record_format, .. }
                if exported_record == &current_record && record_format == CONTEXT_SUMMARY_FORMAT
        )));
        assert!(exported.iter().any(|row| matches!(
            row,
            StorageRecord::ContextCursor { cursor: exported_cursor }
                if exported_cursor == &current_cursor
        )));

        let bounded_actor = "project/example/ifs/identity/bounded";
        let bounded_session = "session-bounded";
        let bounded_messages = (0..=MAX_SESSION_SOURCE_ROWS)
            .map(|index| Message::text("user", format!("bounded-{index}")))
            .collect::<Vec<_>>();
        store
            .checkpoint_session(
                bounded_actor,
                bounded_session,
                &bounded_messages[..512],
                &[],
            )
            .await?;
        store
            .append_session_message(
                bounded_actor,
                "session-sibling",
                &Message::text("user", "sibling session"),
            )
            .await?;
        store
            .append_session_message(
                "project/example/ifs/identity/sibling",
                bounded_session,
                &Message::text("user", "sibling namespace"),
            )
            .await?;
        store
            .append_message(bounded_actor, &Message::text("user", "legacy sibling"))
            .await?;
        store
            .checkpoint_session(
                bounded_actor,
                bounded_session,
                &bounded_messages[512..],
                &[],
            )
            .await?;
        let newest_suffix = store
            .session_history_window_after(bounded_actor, bounded_session, 0, 16)
            .await?;
        assert_eq!(newest_suffix.namespace, bounded_actor);
        assert_eq!(newest_suffix.session_id, bounded_session);
        assert_eq!(newest_suffix.view, "main");
        assert_eq!(newest_suffix.total_rows, 1025);
        assert_eq!(newest_suffix.rows.len(), 16);
        assert!(
            newest_suffix
                .rows
                .windows(2)
                .all(|rows| rows[0].sequence < rows[1].sequence)
        );
        assert_eq!(
            newest_suffix
                .rows
                .iter()
                .map(|row| row.message.plain_text().map(str::to_owned))
                .collect::<Vec<_>>(),
            (1009..=1024)
                .map(|index| Some(format!("bounded-{index}")))
                .collect::<Vec<_>>()
        );
        let count_only = store
            .session_history_window_after(bounded_actor, bounded_session, 0, 0)
            .await?;
        assert_eq!(count_only.total_rows, 1025);
        assert!(count_only.rows.is_empty());
        assert!(
            store
                .session_history_window_after(bounded_actor, bounded_session, -1, 1)
                .await
                .is_err()
        );
        assert!(
            store
                .session_history_window_after(
                    bounded_actor,
                    bounded_session,
                    0,
                    MAX_SESSION_SOURCE_ROWS + 1,
                )
                .await
                .is_err()
        );
        let bounded_page = store
            .session_source_snapshot(
                bounded_actor,
                bounded_session,
                bounded_actor,
                0,
                MAX_SESSION_SOURCE_ROWS,
            )
            .await?;
        assert_eq!(bounded_page.rows.len(), MAX_SESSION_SOURCE_ROWS);
        let retained_through = bounded_page
            .through_inclusive
            .context("bounded page omitted its retained boundary")?;
        let actual_through: i64 = sqlx::query_scalar(
            "SELECT MAX(sequence) FROM messages WHERE namespace = ? AND session_id = ?",
        )
        .bind(bounded_actor.as_bytes())
        .bind(bounded_session.as_bytes())
        .fetch_one(store.pool.as_ref())
        .await?;
        assert!(actual_through > retained_through);
        let empty_after_latest = store
            .session_history_window_after(bounded_actor, bounded_session, actual_through, 16)
            .await?;
        assert_eq!(empty_after_latest.after_exclusive, actual_through);
        assert_eq!(empty_after_latest.total_rows, 0);
        assert!(empty_after_latest.rows.is_empty());
        let over_bound = ContextSummaryRecord {
            actor_namespace: bounded_actor.into(),
            session_id: bounded_session.into(),
            source_namespace: bounded_actor.into(),
            summary_namespace: format!("{bounded_actor}/session/{bounded_session}/summaries"),
            source_view: bounded_page.view,
            source_revision: bounded_page.revision,
            after_sequence: 0,
            through_sequence: actual_through,
            turn_id: Some("turn-bounded".into()),
            operation_id: None,
            producer_actor_id: None,
            invocation_id: "invocation-bounded".into(),
            summary: "must not skip an oversized source range".into(),
        };
        let over_bound_error = store
            .checkpoint_context_summary(&ContextSummaryCheckpoint {
                record: over_bound,
                private_reasoning: Vec::new(),
            })
            .await
            .expect_err("an oversized source range advanced its cursor");
        assert!(
            over_bound_error
                .downcast_ref::<ContextSummaryStale>()
                .is_some()
        );
        assert!(
            store
                .context_summary_cursor(bounded_actor, bounded_session, bounded_actor)
                .await?
                .is_none()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM context_summaries WHERE actor_namespace = ?",
            )
            .bind(bounded_actor.as_bytes())
            .fetch_one(store.pool.as_ref())
            .await?,
            0
        );

        candidate.abandon().await?;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn context_checkpoint_rejects_changed_content_and_prior_summary_at_old_revision()
    -> Result<()> {
        let actor = "project/example/ifs/identity/source-proof";
        let session = "proof-session";
        let store = MemoryStore::temporary().await?;
        store
            .append_session_message(actor, session, &Message::text("user", "original"))
            .await?;
        let original = store
            .session_source_snapshot(actor, session, actor, 0, 16)
            .await?;
        let checkpoint = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                actor_namespace: actor.into(),
                session_id: session.into(),
                source_namespace: actor.into(),
                summary_namespace: format!("{actor}/summaries"),
                source_view: original.view.clone(),
                source_revision: original.revision.clone(),
                after_sequence: 0,
                through_sequence: original.through_inclusive.unwrap(),
                turn_id: Some("proof-turn-a".into()),
                operation_id: None,
                producer_actor_id: None,
                invocation_id: "proof-invocation-a".into(),
                summary: "original summary".into(),
            },
            private_reasoning: vec![],
        };
        sqlx::query("UPDATE messages SET content = ? WHERE namespace = ? AND session_id = ? AND sequence = ?")
            .bind(encode_typed_message(&Message::text("user", "changed in place"))?)
            .bind(actor.as_bytes())
            .bind(session.as_bytes())
            .bind(checkpoint.record.through_sequence)
            .execute(store.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind("change selected source content")
            .bind(AUTHOR)
            .fetch_all(store.pool.as_ref())
            .await?;
        let changed = store
            .session_source_snapshot(actor, session, actor, 0, 16)
            .await?;
        assert_eq!(changed.through_inclusive, original.through_inclusive);
        assert_eq!(changed.rows.len(), original.rows.len());
        assert_ne!(changed.rows, original.rows);
        let stale = store
            .checkpoint_context_summary(&checkpoint)
            .await
            .expect_err("changed source contents published under old provenance");
        assert!(stale.downcast_ref::<ContextSummaryStale>().is_some());
        assert!(
            store
                .context_summary_cursor(actor, session, actor)
                .await?
                .is_none()
        );

        let first = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                source_revision: changed.revision.clone(),
                summary: "first prior summary".into(),
                ..checkpoint.record.clone()
            },
            private_reasoning: vec![],
        };
        store.checkpoint_context_summary(&first).await?;
        let first_id = context_summary_id(&first.record)?;
        store
            .append_session_message(actor, session, &Message::text("assistant", "next source"))
            .await?;
        let next = store
            .session_source_snapshot(actor, session, actor, first.record.through_sequence, 16)
            .await?;
        let second = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                source_revision: next.revision,
                after_sequence: first.record.through_sequence,
                through_sequence: next.through_inclusive.unwrap(),
                turn_id: Some("proof-turn-b".into()),
                invocation_id: "proof-invocation-b".into(),
                summary: "must not replace changed prior".into(),
                ..first.record.clone()
            },
            private_reasoning: vec![],
        };
        sqlx::query("UPDATE context_summaries SET summary = ? WHERE summary_id = ?")
            .bind("prior summary changed without cursor move")
            .bind(&first_id)
            .execute(store.pool.as_ref())
            .await?;
        sqlx::query("CALL DOLT_COMMIT('-Am', ?, '--author', ?)")
            .bind("change selected prior summary content")
            .bind(AUTHOR)
            .fetch_all(store.pool.as_ref())
            .await?;
        let stale = store
            .checkpoint_context_summary(&second)
            .await
            .expect_err("changed prior summary published under old provenance");
        assert!(stale.downcast_ref::<ContextSummaryStale>().is_some());
        let cursor = store
            .context_summary_cursor(actor, session, actor)
            .await?
            .context("prior cursor disappeared")?;
        assert_eq!(cursor.summary_id, first_id);
        assert_eq!(cursor.through_sequence, first.record.through_sequence);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_context_checkpoint_proves_ancestor_without_accepting_sibling_hash()
    -> Result<()> {
        let actor = "project/example/ifs/identity/candidate-proof";
        let session = "candidate-proof-session";
        let store = MemoryStore::temporary().await?;
        store
            .append_session_message(actor, session, &Message::text("user", "shared base"))
            .await?;
        let candidate = store.begin_candidate("candidate source proof").await?;
        candidate
            .view()
            .append_session_message(
                actor,
                session,
                &Message::text("assistant", "candidate only"),
            )
            .await?;
        let captured = candidate
            .view()
            .session_source_snapshot(actor, session, actor, 0, 16)
            .await?;
        let candidate_checkpoint = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                actor_namespace: actor.into(),
                session_id: session.into(),
                source_namespace: actor.into(),
                summary_namespace: format!("{actor}/summaries"),
                source_view: captured.view,
                source_revision: captured.revision,
                after_sequence: 0,
                through_sequence: captured.through_inclusive.unwrap(),
                turn_id: Some("candidate-proof-turn".into()),
                operation_id: None,
                producer_actor_id: None,
                invocation_id: "candidate-proof-invocation".into(),
                summary: "candidate private summary".into(),
            },
            private_reasoning: vec![],
        };
        candidate
            .view()
            .put("unrelated/candidate", &json!(true))
            .await?;
        candidate
            .view()
            .checkpoint_context_summary(&candidate_checkpoint)
            .await?;
        let candidate_cursor = candidate
            .view()
            .context_summary_cursor(actor, session, actor)
            .await?;
        assert!(candidate_cursor.is_some());
        assert!(
            store
                .context_summary_cursor(actor, session, actor)
                .await?
                .is_none()
        );
        let rejected = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                source_view: "main".into(),
                through_sequence: 1,
                summary: "must not cross view lineage".into(),
                ..candidate_checkpoint.record.clone()
            },
            private_reasoning: vec![],
        };
        store.put("unrelated/main", &json!(true)).await?;
        let stale = store
            .checkpoint_context_summary(&rejected)
            .await
            .expect_err("candidate-only revision authenticated main source");
        assert!(stale.downcast_ref::<ContextSummaryStale>().is_some());
        assert!(
            store
                .context_summary_cursor(actor, session, actor)
                .await?
                .is_none()
        );
        let mut malformed = rejected;
        malformed.record.source_revision = "invalid' revision".into();
        let stale = store
            .checkpoint_context_summary(&malformed)
            .await
            .expect_err("malformed historical hash reached AS OF SQL");
        assert!(stale.downcast_ref::<ContextSummaryStale>().is_some());
        candidate.abandon().await?;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn failed_database_batch_rolls_back_and_committed_receipt_reconciles() {
        let store = MemoryStore::temporary().await.unwrap();
        store.put("one", &json!(1)).await.unwrap();
        let operation = Uuid::new_v4().to_string();
        let (mut connection, id) = owned_connection(&store.pool).await.unwrap();
        apply(
            &mut connection,
            &operation,
            "receipt",
            Mutation::Checkpoint {
                namespace: "turn".into(),
                session_id: None,
                messages: vec![(
                    "assistant".into(),
                    encode_typed_message(&Message::text("assistant", "answer")).unwrap(),
                )],
                values: vec![("two".into(), "2".into())],
                public_turn: None,
                mode: None,
            },
            None,
        )
        .await
        .unwrap();
        let before = store.revision().await.unwrap();
        // The overlong receipt label fails AFTER the row update, proving SQL rollback.
        let rejected_operation = Uuid::new_v4().to_string();
        assert!(
            apply(
                &mut connection,
                &rejected_operation,
                &"x".repeat(129),
                Mutation::Checkpoint {
                    namespace: "turn".into(),
                    session_id: None,
                    messages: vec![(
                        "assistant".into(),
                        encode_typed_message(&Message::text("assistant", "duplicate")).unwrap(),
                    )],
                    values: vec![("one".into(), "10".into())],
                    public_turn: None,
                    mode: None,
                },
                None,
            )
            .await
            .is_err()
        );
        drop(connection);
        await_session_end(&store.pool, id, QUERY_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(store.get("one").await.unwrap(), Some(json!(1)));
        assert_eq!(
            store.history("turn", 10).await.unwrap(),
            [Message::text("assistant", "answer")]
        );
        assert_eq!(store.revision().await.unwrap(), before);
        assert!(operation_exists(&store.pool, &operation).await.unwrap());
        assert!(
            !operation_exists(&store.pool, &Uuid::new_v4().to_string())
                .await
                .unwrap()
        );
        // Simulate the caller losing the commit reply: the durable operation is
        // authoritative and reconciliation drains it without replaying its write.
        *store.shared.uncertain.lock().unwrap() = Some(Pending {
            pool: store.pool.clone(),
            connection: id,
            receipt: Receipt::Operation(operation),
        });
        assert_eq!(store.reconcile().await.unwrap(), Some(true));
        assert!(store.shared.uncertain.lock().unwrap().is_none());
        assert_eq!(store.get("two").await.unwrap(), Some(json!(2)));
        assert_eq!(store.revision().await.unwrap(), before);
        assert_eq!(store.reconcile().await.unwrap(), None);
        *store.shared.uncertain.lock().unwrap() = Some(Pending {
            pool: store.pool.clone(),
            connection: id,
            receipt: Receipt::Operation(Uuid::new_v4().to_string()),
        });
        assert_eq!(store.reconcile().await.unwrap(), Some(false));
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn indexed_logical_receipt_survives_sibling_write_and_scopes_exact_retry_to_view()
    -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let id = Uuid::new_v4();
        let original = store.with_logical_receipt(id, "view.append", b"same encoded call");
        original.append("private/actor", "user", "once").await?;
        let first_revision = store.revision().await?;
        store
            .append("private/actor", "assistant", "later sibling")
            .await?;
        let later_revision = store.revision().await?;
        ensure!(later_revision != first_revision);
        original.append("private/actor", "user", "once").await?;
        ensure!(store.revision().await? == later_revision);
        ensure!(store.history("private/actor", 10).await?.len() == 2);
        let conflicting = store.with_logical_receipt(id, "view.append", b"different encoded call");
        ensure!(
            conflicting
                .append("private/actor", "user", "different")
                .await
                .unwrap_err()
                .to_string()
                .contains("conflicts"),
            "same-view logical ID reuse with different work was accepted"
        );
        let candidate = store.begin_candidate("same UUID on another view").await?;
        let candidate_write =
            candidate
                .view()
                .with_logical_receipt(id, "view.append", b"different encoded call");
        let candidate_receipt = candidate_write
            .logical_receipt
            .as_ref()
            .context("candidate request has no logical receipt")?;
        ensure!(
            candidate_receipt.physical_id
                != original
                    .logical_receipt
                    .as_ref()
                    .context("main request has no logical receipt")?
                    .physical_id,
            "candidate inherited the main view's physical receipt identity"
        );
        candidate_write
            .append("private/actor", "user", "private candidate")
            .await?;
        ensure!(store.history("private/actor", 10).await?.len() == 2);
        ensure!(candidate.view().history("private/actor", 10).await?.len() == 3);
        candidate.promote().await?;
        ensure!(store.history("private/actor", 10).await?.len() == 3);
        ensure!(
            operation_receipt_matches(&store.pool, candidate_receipt).await?,
            "promotion lost the candidate view's exact indexed receipt"
        );
        ensure!(
            operation_receipt_matches(
                &store.pool,
                original
                    .logical_receipt
                    .as_ref()
                    .context("main request has no logical receipt")?
            )
            .await?,
            "promotion erased the inherited main receipt"
        );
        let abandoned = store.begin_candidate("reclaimed outcome view").await?;
        let abandoned_branch = abandoned.view().pinned_view().to_owned();
        let abandoned_id = Uuid::new_v4();
        let encoded = b"abandoned private write";
        abandoned
            .view()
            .with_logical_receipt(abandoned_id, "view.append", encoded)
            .append("private/actor", "user", "discarded")
            .await?;
        abandoned.abandon().await?;
        let argument_digest = Sha256::digest(encoded)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        ensure!(
            store
                .indexed_logical_outcome(
                    &abandoned_branch,
                    abandoned_id,
                    "view.append",
                    &argument_digest,
                )
                .await?
                .is_none(),
            "reclaimed private candidate falsely proved its accepted write absent"
        );
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn committed_promotion_retries_cleanup_before_caching_success() -> Result<()> {
        // Real lifecycles: one local store.
        let deadline = crate::test_support::fixture_deadline(1);
        tokio::time::timeout(deadline, async {
            let root = crate::test_support::tempdir()?;
            let mut options = crate::test_support::open_options(
                root.path().to_owned(),
                format!("project/{}", "7".repeat(64)),
            )?;
            options.candidate_cleanup_failure = Some(Arc::new(AtomicBool::new(true)));
            let store = MemoryStore::open(options).await?;
            let candidate = store.begin_candidate("retry resolved cleanup").await?;
            let retained = candidate.view();
            let branch = candidate.branch().to_owned();
            let base = candidate.base().to_owned();
            retained.put("retry-promoted", &json!(true)).await?;
            let target = retained.revision().await?;

            let error = candidate
                .promote_exact(&target)
                .await
                .expect_err("injected cleanup failure was ignored");
            ensure!(
                error
                    .to_string()
                    .contains("injected candidate cleanup failure"),
                "unexpected injected cleanup error: {error:#}"
            );
            ensure!(store.revision().await? == target);
            ensure!(
                retained.get("retry-promoted").await.is_err(),
                "committed promotion retained a usable candidate view after cleanup failure"
            );
            ensure!(
                store
                    .candidate_transition_observation(&branch, &base, &target)
                    .await?
                    == CandidateTransitionObservation::Promoted
            );

            ensure!(candidate.promote_exact(&target).await? == target);
            ensure!(
                retained.get("retry-promoted").await.is_err(),
                "cleanup retry left the resolved candidate view usable"
            );
            ensure!(store.get("retry-promoted").await? == Some(json!(true)));
            ensure!(
                candidate_heads(&store.pool, &CandidateNames::from_open(&branch)?)
                    .await?
                    .is_empty()
            );
            store.close().await
        })
        .await
        .with_context(|| {
            format!("candidate cleanup retry fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_transition_observation_uses_exact_refs_and_revisions() -> Result<()> {
        // Real lifecycles: one temporary local store.
        let deadline = crate::test_support::fixture_deadline(1);
        tokio::time::timeout(deadline, async {
            let store = MemoryStore::temporary().await?;
            let stale = store.begin_candidate("stale transition").await?;
            let stale_view = stale.view();
            let stale_branch = stale.view().pinned_view().to_owned();
            let stale_base = stale.base().to_owned();
            stale.view().put("private-stale", &json!(1)).await?;
            let stale_target = stale.view().revision().await?;
            ensure!(stale.promote_exact(&stale_base).await.is_err());
            ensure!(stale_view.get("private-stale").await? == Some(json!(1)));
            ensure!(
                store
                    .candidate_transition_observation(&stale_branch, &stale_base, &stale_target)
                    .await?
                    == CandidateTransitionObservation::OpenUnchanged
            );
            store.put("sibling", &json!(true)).await?;
            ensure!(
                store
                    .candidate_transition_observation(&stale_branch, &stale_base, &stale_target)
                    .await?
                    == CandidateTransitionObservation::OpenConflict
            );
            ensure!(stale.promote_exact(&stale_target).await.is_err());
            ensure!(stale_view.get("private-stale").await? == Some(json!(1)));
            stale.abandon_exact(&stale_target).await?;
            ensure!(
                stale_view.get("private-stale").await.is_err(),
                "resolved abandoned candidate view remained usable"
            );

            let promoted = store.begin_candidate("promoted transition").await?;
            let promoted_view = promoted.view();
            let branch = promoted.view().pinned_view().to_owned();
            let base = promoted.base().to_owned();
            promoted.view().put("private-promoted", &json!(2)).await?;
            let target = promoted.view().revision().await?;
            ensure!(promoted.promote_exact(&target).await? == target);
            ensure!(
                promoted_view.get("private-promoted").await.is_err(),
                "resolved promoted candidate view remained usable"
            );
            ensure!(store.get("private-promoted").await? == Some(json!(2)));
            store.put("later", &json!(true)).await?;
            ensure!(
                store
                    .candidate_transition_observation(&branch, &base, &target)
                    .await?
                    == CandidateTransitionObservation::Promoted,
                "later main revision hid the exact promoted candidate head"
            );
            store.close().await
        })
        .await
        .with_context(|| {
            format!("candidate transition observation fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn selected_candidate_inventory_pages_past_foreign_prefix_and_abandons_exact_ref()
    -> Result<()> {
        // Real lifecycles: one temporary local store.
        let deadline = crate::test_support::fixture_deadline(1);
        tokio::time::timeout(deadline, async {
            let store = MemoryStore::temporary().await?;
            let candidate = store.begin_candidate("retained exact ref").await?;
            let branch = candidate.view().pinned_view().to_owned();
            let base = candidate.base().to_owned();
            candidate
                .view()
                .put("private-selected", &json!(true))
                .await?;
            let head = candidate.view().revision().await?;
            sqlx::query("CALL DOLT_BRANCH(?, ?)")
                .bind("candidate_000")
                .bind(&base)
                .fetch_all(store.pool.as_ref())
                .await?;
            let first = store.candidate_inventory(None, 1).await?;
            ensure!(first.candidates.is_empty());
            ensure!(first.next.as_deref() == Some("000"));
            let second = store.candidate_inventory(first.next.as_deref(), 1).await?;
            ensure!(second.candidates.len() == 1 && second.candidates[0].branch == branch);
            ensure!(second.candidates[0].head.as_deref() == Some(head.as_str()));
            ensure!(second.candidates[0].base.as_deref() == Some(base.as_str()));
            ensure!(second.candidates[0].state == CandidateRefState::OpenUnchanged);
            ensure!(
                store
                    .abandon_candidate_ref(&branch, &base, &head)
                    .await
                    .is_err(),
                "active candidate view was abandoned"
            );
            drop(candidate);
            ensure!(
                store
                    .abandon_candidate_ref(&branch, &base, &base)
                    .await
                    .is_err()
            );
            ensure!(
                store
                    .abandon_candidate_ref(&branch, &head, &head)
                    .await
                    .is_err()
            );
            store.put("later-main", &json!(true)).await?;
            ensure!(
                store.candidate_ref_status(&branch).await?.state == CandidateRefState::OpenConflict
            );
            store.abandon_candidate_ref(&branch, &base, &head).await?;
            ensure!(store.candidate_ref_status(&branch).await?.state == CandidateRefState::Missing);
            ensure!(
                store
                    .candidate_transition_observation(&branch, &base, &head)
                    .await?
                    == CandidateTransitionObservation::AbandonedReclaimed
            );
            ensure!(store.get("private-selected").await?.is_none());
            store.close().await
        })
        .await
        .with_context(|| {
            format!("selected candidate inventory fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_creation_outcome_only_reads_its_exact_ref_across_restart() -> Result<()> {
        // Real lifecycles: one local store, then its reopen.
        let deadline = crate::test_support::fixture_deadline(2);
        tokio::time::timeout(deadline, async {
            let root = crate::test_support::tempdir()?;
            let options = crate::test_support::open_options(
                root.path().to_owned(),
                format!("project/{}", "7".repeat(64)),
            )?;
            let store = MemoryStore::open(options.clone()).await?;
            let id = Uuid::new_v4();
            ensure!(matches!(
                store.candidate_for_id(id).await?,
                CandidateLookup::Missing
            ));
            let candidate = store.begin_candidate_with_id("exact candidate", id).await?;
            let branch = candidate.view().pinned_view().to_owned();
            let base = candidate.base().to_owned();
            candidate.view().put("private", &json!("retained")).await?;
            store.put("sibling", &json!("later")).await?;
            let before = candidate_heads(&store.pool, &CandidateNames::from_open(&branch)?).await?;
            let CandidateLookup::Open(recovered) = store.candidate_for_id(id).await? else {
                bail!("exact candidate was not readable after both histories advanced")
            };
            ensure!(recovered.base() == base, "candidate creation base changed");
            ensure!(recovered.view().pinned_view() == branch);
            ensure!(recovered.view().get("private").await? == Some(json!("retained")));
            ensure!(
                candidate_heads(&store.pool, &CandidateNames::from_open(&branch)?).await? == before,
                "read-only candidate outcome changed the exact refs"
            );
            ensure!(
                store
                    .begin_candidate_with_id("exact candidate", id)
                    .await
                    .is_err(),
                "a second Begin reused an active candidate identity"
            );
            drop(recovered);
            drop(candidate);
            store.close().await?;

            let reopened = MemoryStore::open(options).await?;
            let CandidateLookup::Open(recovered) = reopened.candidate_for_id(id).await? else {
                bail!("owner restart lost the caller-derived candidate ref")
            };
            ensure!(recovered.base() == base);
            ensure!(recovered.view().get("private").await? == Some(json!("retained")));
            recovered.abandon().await?;
            drop(recovered);
            ensure!(matches!(
                reopened.candidate_for_id(id).await?,
                CandidateLookup::Resolved | CandidateLookup::Missing
            ));
            ensure!(
                candidate_heads(&reopened.pool, &CandidateNames::from_open(&branch)?)
                    .await?
                    .is_empty(),
                "explicit abandonment did not reclaim the exact ref"
            );
            reopened.close().await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("exact candidate outcome fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn mode_checkpoint_updates_catalog_and_state_without_a_public_turn() -> Result<()> {
        let store = MemoryStore::temporary().await?;
        let session = "mode-session";
        let namespace = "project/transcript/mode-session";
        let state_key = "project/session/mode-session";
        store.create_session_catalog(session, Mode::Ifs, "").await?;
        let mode = SessionModeCheckpoint {
            expected_generation: 0,
            expected_mode: Mode::Ifs,
            mode: Mode::Jungian,
        };
        let state = json!({"id": session, "mode": Mode::Jungian, "lifecycle_generation": 0});
        let updates = vec![(state_key.into(), state.clone())];
        let before = store.revision().await?;
        assert!(
            store
                .checkpoint_session_mode(
                    namespace,
                    session,
                    &[(
                        state_key.into(),
                        json!({"id": session, "mode": Mode::Ifs, "lifecycle_generation": 0})
                    )],
                    &mode
                )
                .await
                .is_err()
        );
        assert_eq!(store.revision().await?, before);
        assert_eq!(store.get(state_key).await?, None);
        store
            .checkpoint_session_mode(namespace, session, &updates, &mode)
            .await?;
        let catalog = store
            .session_catalog_record(session)
            .await?
            .context("catalog row missing")?;
        assert_eq!(catalog.mode, Mode::Jungian);
        assert_eq!(catalog.lifecycle_generation, 0);
        assert_eq!(store.get(state_key).await?, Some(state));
        assert!(
            store
                .public_transcript_page(session, None, 16)
                .await?
                .records
                .is_empty()
        );
        let committed = store.revision().await?;
        assert!(
            store
                .checkpoint_session_mode(namespace, session, &updates, &mode)
                .await
                .is_err()
        );
        assert_eq!(store.revision().await?, committed);
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_validates_one_namespace_and_atomic_state_inputs() {
        let store = MemoryStore::temporary().await.unwrap();
        let before = store.revision().await.unwrap();
        for result in [
            store
                .checkpoint("", &[Message::text("user", "prompt")], &[])
                .await,
            store
                .checkpoint("turn", &[Message::text("", "prompt")], &[])
                .await,
            store.checkpoint("turn", &[], &[]).await,
            store
                .checkpoint(
                    "turn",
                    &[],
                    &[("same".into(), json!(1)), ("same".into(), json!(2))],
                )
                .await,
        ] {
            assert!(result.is_err());
        }
        assert_eq!(store.revision().await.unwrap(), before);
        assert!(store.history("turn", 10).await.unwrap().is_empty());
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn durable_store_reopens_readonly_and_rejects_schema_drift() {
        let directory = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "a".repeat(64));
        let options =
            crate::test_support::open_options(directory.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        store.append("retained", "user", "hello").await.unwrap();
        store.put("choice", &json!("jungian")).await.unwrap();
        let revision = store.revision().await.unwrap();
        store.close().await.unwrap();

        let mut readonly = options.clone();
        readonly.read_only = true;
        let reader = MemoryStore::open(readonly).await.unwrap();
        assert_eq!(reader.revision().await.unwrap(), revision);
        assert_eq!(
            reader.history("retained", 10).await.unwrap()[0].plain_text(),
            Some("hello")
        );
        assert_eq!(reader.get("choice").await.unwrap(), Some(json!("jungian")));
        assert!(reader.put("choice", &json!("ifs")).await.is_err());
        assert!(reader.begin_candidate("dream").await.is_err());
        assert!(reader.status().await.unwrap().read_only);
        reader.close().await.unwrap();

        let store = MemoryStore::open(options.clone()).await.unwrap();
        sqlx::query("UPDATE kuru_schema SET version = 99")
            .execute(store.pool.as_ref())
            .await
            .unwrap();
        assert!(
            migrations::validate_supported(&store.pool)
                .await
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
        store.close().await.unwrap();
        assert!(MemoryStore::open(options).await.is_err());
    }

    #[tokio::test]
    async fn private_paths_and_stable_lock_fail_closed() {
        // Held for the whole test: every lock acquisition here is a real
        // flock the assertions below expect to succeed or fail outright, and
        // this test never itself spawns; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking_async().await;
        let directory = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "a".repeat(64));
        assert!(!MemoryStore::exists(directory.path(), &scope).unwrap());
        for invalid in ["project/../../escape", "project/ABC", "bad"] {
            assert!(project_directory(directory.path(), invalid).is_err());
        }
        let file = directory.path().join("file");
        fs::write(&file, "retained").unwrap();
        assert!(private_dir(&file).is_err());
        assert!(private_file(directory.path()).is_err());
        let link = directory.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&file, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&file, &link).unwrap();
        assert!(private_file(&link).is_err());
        assert!(private_dir(&link).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "retained");
        let lock_path = directory.path().join("lease");
        let held = private_file(&lock_path).unwrap();
        held.lock().unwrap();
        assert!(
            acquire_lock(private_file(&lock_path).unwrap(), Duration::from_millis(20))
                .await
                .is_err()
        );
        drop(held);
        acquire_lock(private_file(&lock_path).unwrap(), Duration::from_millis(20))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn real_directory_move_completion_error_reconciles_identity_before_stage_recovery() {
        let data = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "7".repeat(64));
        let options =
            crate::test_support::open_options(data.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        let initial = store.revision().await.unwrap();
        let active = project_directory(data.path(), &scope).unwrap();
        store.close().await.unwrap();

        let namespace = cfg!(windows).then(|| data.path().join("memory/lifecycles"));
        let mut lease =
            Server::quiescence_at(&active, namespace.as_deref(), Duration::from_secs(5))
                .await
                .unwrap();
        let identity = lease.directory.identity();
        let marker = fs::read(active.join("ready.json")).unwrap();
        let occupied = active.with_file_name("unrelated destination café 東京");
        private_dir(&occupied).unwrap();
        files::write(&occupied.join("evidence"), b"unrelated retained bytes").unwrap();
        assert!(files::move_directory(&lease.directory, &occupied).is_err());
        assert_eq!(files::directory(&active).unwrap().identity(), identity);
        assert_eq!(
            fs::read(occupied.join("evidence")).unwrap(),
            b"unrelated retained bytes"
        );
        let stage = active.with_file_name(format!("{}.staging-{}", "7".repeat(64), Uuid::new_v4()));
        let observed = std::cell::Cell::new(false);
        lease
            .move_to_observed(&stage, |moved| {
                assert_eq!(moved.identity(), identity);
                assert_eq!(fs::read(moved.path().join("ready.json"))?, marker);
                assert!(!active.exists());
                observed.set(true);
                Err(anyhow::anyhow!(
                    "fixture completion error after the actual directory move"
                ))
            })
            .unwrap();
        assert!(
            observed.get(),
            "the fixture must cross the actual native move boundary"
        );
        assert_eq!(lease.directory.identity(), identity);
        assert!(
            Server::quiescence_at(&stage, namespace.as_deref(), Duration::from_millis(20))
                .await
                .is_err()
        );
        drop(lease);
        let recovered = MemoryStore::open(options).await.unwrap();
        assert_eq!(recovered.revision().await.unwrap(), initial);
        assert_eq!(files::directory(&active).unwrap().identity(), identity);
        assert!(!stage.exists());
        assert_eq!(fs::read(active.join("ready.json")).unwrap(), marker);
        assert_eq!(
            fs::read(occupied.join("evidence")).unwrap(),
            b"unrelated retained bytes"
        );
        recovered.close().await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_activation_reuses_the_committed_stage_and_preserves_incomplete_work() {
        let data = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "f".repeat(64));
        let options =
            crate::test_support::open_options(data.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        let initial = store.revision().await.unwrap();
        let active = project_directory(data.path(), &scope).unwrap();
        store.close().await.unwrap();

        // Model interruption at the actual durable boundary: the complete store
        // and activation receipt exist, but directory publication did not happen.
        let staging =
            active.with_file_name(format!("{}.staging-{}", "f".repeat(64), Uuid::new_v4()));
        fs::rename(&active, &staging).unwrap();
        let incomplete =
            active.with_file_name(format!("{}.staging-{}", "f".repeat(64), Uuid::new_v4()));
        private_dir(&incomplete).unwrap();
        // The supervisor creates its stable lock before writing store identity.
        #[cfg(unix)]
        private_file(&incomplete.join("lifecycle.lock")).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        assert_eq!(
            store.revision().await.unwrap(),
            initial,
            "reuse the completed import without a second schema revision"
        );
        assert!(!staging.exists());
        assert!(!incomplete.exists());
        assert!(
            data.path()
                .join("memory/interrupted")
                .join(incomplete.file_name().unwrap())
                .is_dir()
        );
        assert!(MemoryStore::exists(data.path(), &scope).unwrap());
        store.close().await.unwrap();

        fs::remove_file(active.join("ready.json")).unwrap();
        assert!(
            MemoryStore::exists(data.path(), &scope).is_err(),
            "unknown active data must not appear empty"
        );
        assert!(MemoryStore::open(options).await.is_err());
    }

    #[tokio::test]
    async fn recovery_cannot_move_a_stage_while_an_attached_owner_is_still_running() {
        let data = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "8".repeat(64));
        let mut options =
            crate::test_support::open_options(data.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options.clone()).await.unwrap();
        let initial = store.revision().await.unwrap();
        store.close().await.unwrap();

        let active = project_directory(data.path(), &scope).unwrap();
        let stage = active.with_file_name(format!("{}.staging-{}", "8".repeat(64), Uuid::new_v4()));
        fs::rename(&active, &stage).unwrap();
        let binary = provision::provision(&options.config, &data.path().join("tools/dolt"))
            .await
            .unwrap();
        let owner = Server::open(ServerOptions {
            binary,
            directory: stage.clone(),
            project_scope: scope,
            supervisor: options.supervisor.clone().unwrap(),
            timeout: Duration::from_secs(30),
            read_only: false,
            retained: None,
            lifecycle_root: if cfg!(windows) {
                Some(data.path().join("memory/lifecycles"))
            } else {
                None
            },
        })
        .await
        .unwrap();
        options.config.startup_timeout_secs = 1;
        let error = MemoryStore::open(options.clone()).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("lifecycle remains active"),
            "{error:#}"
        );
        assert!(stage.is_dir());
        assert!(!active.exists());
        let pool = owner.pool("main").await.unwrap();
        assert_eq!(revision(&pool).await.unwrap(), initial);
        pool.close().await;
        owner.close().await.unwrap();
        options.config.startup_timeout_secs = 30;
        let recovered = MemoryStore::open(options).await.unwrap();
        assert_eq!(recovered.revision().await.unwrap(), initial);
        assert!(!stage.exists());
        recovered.close().await.unwrap();
    }

    #[tokio::test]
    async fn stopped_store_backup_restores_revisions_and_candidate_history_in_a_new_data_directory()
    {
        async fn export_records(
            store: &MemoryStore,
        ) -> Result<(ExportProvenance, Vec<StorageRecord>)> {
            let snapshot = store.begin_active_export().await?;
            let provenance = snapshot.provenance().clone();
            let mut cursor = None;
            let mut records = Vec::new();
            loop {
                let page = snapshot.page(cursor).await?;
                records.extend(page.records);
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            Ok((provenance, records))
        }

        fn copy_tree(source: &Path, target: &Path) {
            private_dir(target).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let destination = target.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &destination);
                } else {
                    assert!(entry.file_type().unwrap().is_file());
                    fs::copy(entry.path(), destination).unwrap();
                }
            }
        }
        let source = crate::test_support::tempdir().unwrap();
        let restored = crate::test_support::tempdir().unwrap();
        let scope = format!("project/{}", "9".repeat(64));
        let options =
            crate::test_support::open_options(source.path().to_owned(), scope.clone()).unwrap();
        let store = MemoryStore::open(options).await.unwrap();
        store
            .append("session", "user", "retained transcript")
            .await
            .unwrap();
        let parent = "backup-parent";
        let child = "backup-child";
        let transcript = format!("{scope}/transcript/{parent}");
        let actor = format!("{scope}/actor/part-a");
        store
            .append_session_message(
                &transcript,
                parent,
                &Message::text("assistant", "exact legacy prefix"),
            )
            .await
            .unwrap();
        store
            .append_session_message(
                &actor,
                parent,
                &Message::text("assistant", "parent private"),
            )
            .await
            .unwrap();
        let legacy = store
            .session_history_window_after(&transcript, parent, 0, 16)
            .await
            .unwrap();
        let legacy_row = legacy.rows.first().unwrap();
        let prefix = LegacyTranscriptPrefix {
            namespace: transcript.clone(),
            source_session_id: parent.into(),
            source_revision: legacy.revision,
            first_sequence: legacy_row.sequence,
            through_sequence: legacy_row.sequence,
            row_count: 1,
            record_format: LEGACY_PREFIX_RECORD_FORMAT.into(),
        };
        let turn = PublicTurnRecord {
            node_id: public_turn_node_id(parent, "backup-turn").unwrap(),
            origin_session_id: parent.into(),
            turn_id: "backup-turn".into(),
            kind: PublicTurnKind::Primary,
            continuation_of_node_id: None,
            predecessor_node_id: None,
            settlement: PublicTurnSettlement::Completed,
            user_entry: Some(Message::text("user", "backup question")),
            speaker_id: Some("part-a".into()),
            terminal_entries: vec![Message::text("assistant", "backup answer")],
            record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
        };
        store
            .fixture_insert_public_session(
                &SessionCatalogRecord {
                    session_id: parent.into(),
                    mode: Mode::Jungian,
                    label: "original parent".into(),
                    created_order: 1,
                    updated_order: 1,
                    lifecycle_generation: 0,
                    lifecycle_state: SessionLifecycleState::Active,
                    head_node_id: Some(turn.node_id.clone()),
                    pending_node_id: None,
                    legacy_prefix: Some(prefix.clone()),
                    fork_provenance: None,
                    record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
                },
                std::slice::from_ref(&turn),
            )
            .await
            .unwrap();
        let child_outcome = store
            .fork_session_catalog(parent, 0, &turn.node_id, child, "forked child")
            .await
            .unwrap();
        assert_eq!(child_outcome.session_id, child);
        let child_catalog = store.session_catalog_record(child).await.unwrap().unwrap();
        assert_eq!(child_catalog.legacy_prefix.as_ref(), Some(&prefix));
        let before = export_records(&store).await.unwrap();
        assert_eq!(before.0.message_count, 3);
        assert_eq!(before.0.session_catalog_count, 2);
        assert_eq!(before.0.public_turn_count, 1);
        assert!(before.1.iter().any(|record| matches!(record,
            StorageRecord::SessionCatalog { record } if record == &child_catalog)));
        assert!(before.1.iter().any(|record| matches!(record,
            StorageRecord::PublicTurn { record } if record == &turn)));
        let revision = store.revision().await.unwrap();
        let candidate = store.begin_candidate("unpublished dream").await.unwrap();
        let view = candidate.view();
        view.append("candidate", "assistant", "private retained branch")
            .await
            .unwrap();
        let branch = view.branch.clone();
        drop(view);
        drop(candidate);
        store.close().await.unwrap();
        copy_tree(
            &source.path().join("memory"),
            &restored.path().join("memory"),
        );
        let options = crate::test_support::open_options(restored.path().to_owned(), scope).unwrap();
        let store = MemoryStore::open(options).await.unwrap();
        assert_eq!(store.revision().await.unwrap(), revision);
        assert_eq!(export_records(&store).await.unwrap(), before);
        let restored_parent = store.session_catalog_record(parent).await.unwrap().unwrap();
        assert_eq!(restored_parent.mode, Mode::Jungian);
        assert_eq!(
            restored_parent.head_node_id.as_deref(),
            Some(turn.node_id.as_str())
        );
        assert_eq!(restored_parent.legacy_prefix.as_ref(), Some(&prefix));
        assert_eq!(
            store.session_catalog_record(child).await.unwrap(),
            Some(child_catalog)
        );
        let child_page = store.public_transcript_page(child, None, 16).await.unwrap();
        assert!(matches!(child_page.records.as_slice(),
            [PublicTranscriptEntry::Turn { record }, PublicTranscriptEntry::Legacy { message, .. }]
                if record == &turn && message.text_projection() == "exact legacy prefix"));
        assert_eq!(
            store
                .session_history_window(&actor, parent, 16)
                .await
                .unwrap()
                .messages
                .len(),
            1
        );
        assert!(
            store
                .session_history_window(&actor, child, 16)
                .await
                .unwrap()
                .messages
                .is_empty()
        );
        assert_eq!(
            store.history("session", 10).await.unwrap()[0].plain_text(),
            Some("retained transcript")
        );
        assert!(store.history("candidate", 10).await.unwrap().is_empty());
        let candidate_pool = store.shared.server.pool(&branch).await.unwrap();
        let content: String =
            sqlx::query_scalar("SELECT content FROM messages WHERE namespace = ?")
                .bind(b"candidate".as_slice())
                .fetch_one(candidate_pool.as_ref())
                .await
                .unwrap();
        assert_eq!(content, "private retained branch");
        store.close().await.unwrap();
    }

    #[test]
    fn typed_row_codec_preserves_literal_json_text_and_bounds_parse_errors() {
        let literal = r#"{"blocks":[{"type":"tool_use","id":"still text"}]}"#;
        assert_eq!(
            decode_message("user".into(), TEXT_FORMAT, literal).unwrap(),
            Message::text("user", literal)
        );
        let message = Message {
            role: "assistant".into(),
            blocks: vec![
                ContentBlock::Text {
                    text: "first".into(),
                },
                ContentBlock::ToolUse {
                    id: "call-1".into(),
                    name: "file_read".into(),
                    arguments: serde_json::json!({"path":"README.md"}),
                },
                ContentBlock::Text {
                    text: "last".into(),
                },
            ],
        };
        let encoded = encode_typed_message(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).unwrap(),
            serde_json::json!({"blocks": message.blocks.clone()})
        );
        assert_eq!(
            decode_message(message.role.clone(), TYPED_FORMAT, &encoded).unwrap(),
            message
        );
        let sentinel = "secret-unknown-block-tag-should-not-leak";
        let malformed = format!(r#"{{"blocks":[{{"type":"{sentinel}"}}]}}"#);
        let error = decode_message("user".into(), TYPED_FORMAT, &malformed)
            .expect_err("unknown block tag was accepted");
        assert!(format!("{error:#}").contains("invalid block payload"));
        assert!(!format!("{error:#}").contains(sentinel));
        assert!(decode_message("user".into(), TYPED_FORMAT, r#"{"blocks":[],"extra":1}"#).is_err());
        assert!(decode_message("user".into(), "future-v9", literal).is_err());
    }

    #[tokio::test]
    async fn committed_unknown_and_malformed_formats_refuse_history_and_export() {
        let store = MemoryStore::temporary().await.unwrap();
        store
            .append("format", "user", "safe original")
            .await
            .unwrap();
        for (format, content, expected) in [
            ("future-v9", "safe original", "unsupported content format"),
            (
                TYPED_FORMAT,
                r#"{"blocks":[{"type":"private-sentinel-should-not-leak"}]}"#,
                "invalid block payload",
            ),
        ] {
            sqlx::query("UPDATE messages SET content_format = ?, content = ? WHERE namespace = ?")
                .bind(format)
                .bind(content)
                .bind(b"format".as_slice())
                .execute(store.pool.as_ref())
                .await
                .unwrap();
            sqlx::query("CALL DOLT_COMMIT('-Am', 'format refusal fixture', '--author', ?)")
                .bind(AUTHOR)
                .fetch_all(store.pool.as_ref())
                .await
                .unwrap();
            let error = store
                .history("format", 10)
                .await
                .expect_err("invalid format was read");
            assert!(format!("{error:#}").contains(expected));
            assert!(!format!("{error:#}").contains("private-sentinel"));
            let export = store.begin_active_export().await.unwrap();
            let error = export
                .page(None)
                .await
                .expect_err("invalid format was exported");
            assert!(format!("{error:#}").contains(expected));
            assert!(!format!("{error:#}").contains("private-sentinel"));
        }
        store.close().await.unwrap();
    }
}
