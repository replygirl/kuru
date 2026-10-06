//! Typed storage operations carried only after an authenticated generation handshake.
//! A failed response or broken connection is never permission to replay a write.

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{
    InvocationOutcome, InvocationStart, Message, Mode, SessionUsage, UsageObservation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use uuid::Uuid;

use super::{EndpointAuthority, read_frame, write_frame};
use crate::{
    CandidateInventoryPage, CandidateRefStatus, ExportProvenance, HistoryWindow, Revision,
    StoredNote,
    store::{
        ActiveExportSnapshot, Candidate, CandidateLookup, CandidateRefRefusal,
        CandidateRefRejected, ExportCursor, ExportPage, LogicalReceiptConflict, MemoryStore,
        UsageProof,
    },
};

// JSON can escape a valid 16 MiB typed message by up to six times. Keep the
// frame bounded while leaving the existing message limit representable.
pub(crate) const OPERATION_FRAME_LIMIT: usize = 100 * 1024 * 1024;
pub(crate) const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(35);
pub(super) const FRAME_BUDGET_MIB: usize = 128;
const MIB: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequest {
    pub id: Uuid,
    pub generation: String,
    pub call: ServiceCall,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<crate::SessionDriverProof>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceCall {
    SelectSessionDriver {
        selection: crate::SessionDriverSelection,
    },
    SessionDriverOutcome {
        original_id: Uuid,
        original_generation: String,
        selection: crate::SessionDriverSelection,
    },
    ReattachSessionDriver {
        original_id: Uuid,
        original_generation: String,
        selection: crate::SessionDriverSelection,
    },
    LiveSessionDrivers,
    RetireIfIdle,
    TryAcquireDreamLease,
    AppendMessage {
        namespace: String,
        message: Message,
    },
    HistoryWindow {
        namespace: String,
        limit: usize,
    },
    Notes {
        namespace: String,
        limit: usize,
    },
    PutMany {
        values: Vec<(String, Value)>,
    },
    PutReasoningSummaries {
        records: Vec<crate::ReasoningSummaryRecord>,
    },
    Get {
        key: String,
    },
    Reconcile,
    Revision,
    Outcome {
        original_id: Uuid,
        original_generation: String,
        view: String,
        method: String,
        argument_digest: String,
    },
    /// The ordinary main view or one candidate owned by this attachment.
    View {
        candidate: Option<Uuid>,
        operation: Box<ViewOperation>,
    },
    BeginCandidate {
        label: String,
    },
    CandidateOutcome {
        original_id: Uuid,
        original_generation: String,
    },
    PromoteCandidate {
        handle: Uuid,
        branch: String,
        base: String,
        target: String,
    },
    AbandonCandidate {
        handle: Uuid,
        branch: String,
        base: String,
        target: String,
    },
    ReconcileCandidate {
        handle: Uuid,
        branch: String,
        from: String,
        live: String,
    },
    CandidateReconciliationOutcome {
        original_id: Uuid,
        original_generation: String,
        branch: String,
        from: String,
        live: String,
    },
    CandidateTransitionOutcome {
        original_id: Uuid,
        original_generation: String,
        transition: CandidateTransitionKind,
        branch: String,
        base: String,
        target: String,
    },
    SelectedAbandonOutcome {
        original_id: Uuid,
        original_generation: String,
        branch: String,
        base: String,
        target: String,
    },
    CandidateInventory {
        after: Option<String>,
        limit: usize,
    },
    CandidateRefStatus {
        branch: String,
    },
    AbandonCandidateRef {
        branch: String,
        base: String,
        target: String,
    },
    Ledger {
        operation: Box<LedgerOperation>,
    },
    LedgerOutcome {
        original_id: Uuid,
        original_generation: String,
        proof: UsageProof,
    },
    BeginExport,
    ExportPage {
        handle: Uuid,
        cursor: Option<ExportCursor>,
    },
    BeginStateReadCut {
        candidate: Option<StateReadCandidate>,
    },
    StateReadCutGet {
        handle: Uuid,
        key: String,
    },
    StateReadCutPage {
        handle: Uuid,
        prefix: String,
        cursor: Option<crate::store::StateReadCursor>,
    },
    CloseStateReadCut {
        handle: Uuid,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateReadCandidate {
    pub branch: String,
    pub revision: String,
}

/// Whether a lost reply may conceal an accepted effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mutation {
    Read,
    Write,
}

/// The durable proof a request leaves for its lost-reply outcome query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Receipt {
    /// No durable data receipt. Idle retirement and connection-bound driver
    /// selection/reattachment use this for writes: they change owned resources,
    /// not stored data. Driver recovery requires completed handler evidence
    /// for the exact selection tuple and the retained current claim.
    None,
    /// A logical unit receipt stored with the write, named by this method.
    Unit(&'static str),
    /// The deterministic candidate branch for the request ID.
    CandidateCreation,
    /// The exact candidate ref transition, keyed by its pinned branch.
    CandidateTransition,
    /// The immutable branch/from/live tuple selected for private merge.
    CandidateReconciliation,
    /// The exact inspected branch/base/head chosen for abandonment.
    SelectedAbandon,
    /// The usage ledger's natural-key proof.
    UsageProof,
}

/// How long an attached client waits for one reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplyBudget {
    Operation,
}

impl ReplyBudget {
    pub(crate) const fn deadline(self) -> std::time::Duration {
        match self {
            Self::Operation => OPERATION_TIMEOUT,
        }
    }
}

/// Every operation's handling, decided in one exhaustive place so that a new
/// operation cannot default to "read-only, no receipt".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperationContract {
    pub mutation: Mutation,
    pub receipt: Receipt,
    pub reply: ReplyBudget,
}

impl OperationContract {
    const READ: Self = Self {
        mutation: Mutation::Read,
        receipt: Receipt::None,
        reply: ReplyBudget::Operation,
    };

    const fn write(receipt: Receipt) -> Self {
        Self {
            mutation: Mutation::Write,
            receipt,
            reply: ReplyBudget::Operation,
        }
    }
}

impl ServiceCall {
    /// Keep every arm explicit: no wildcard may classify a new variant.
    pub(crate) fn contract(&self) -> OperationContract {
        use OperationContract as C;
        match self {
            Self::SelectSessionDriver { .. } | Self::ReattachSessionDriver { .. } => {
                C::write(Receipt::None)
            }
            Self::SessionDriverOutcome { .. } | Self::LiveSessionDrivers => C::READ,
            Self::RetireIfIdle => C::write(Receipt::None),
            // Attachment-local lease state; nothing durable changes.
            Self::TryAcquireDreamLease => C::READ,
            Self::AppendMessage { .. } => C::write(Receipt::Unit("append_message")),
            Self::HistoryWindow { .. } => C::READ,
            Self::Notes { .. } => C::READ,
            Self::PutMany { .. } => C::write(Receipt::Unit("put_many")),
            Self::PutReasoningSummaries { .. } => {
                C::write(Receipt::Unit("put_reasoning_summaries"))
            }
            Self::Get { .. } => C::READ,
            Self::Reconcile => C::READ,
            Self::Revision => C::READ,
            Self::Outcome { .. } => C::READ,
            Self::View { operation, .. } => operation.contract(),
            Self::BeginCandidate { .. } => C::write(Receipt::CandidateCreation),
            Self::CandidateOutcome { .. } => C::READ,
            Self::PromoteCandidate { .. } => C::write(Receipt::CandidateTransition),
            Self::AbandonCandidate { .. } => C::write(Receipt::CandidateTransition),
            Self::ReconcileCandidate { .. } => C::write(Receipt::CandidateReconciliation),
            Self::CandidateReconciliationOutcome { .. } => C::READ,
            Self::CandidateTransitionOutcome { .. } => C::READ,
            Self::SelectedAbandonOutcome { .. } => C::READ,
            Self::CandidateInventory { .. } => C::READ,
            Self::CandidateRefStatus { .. } => C::READ,
            Self::AbandonCandidateRef { .. } => C::write(Receipt::SelectedAbandon),
            Self::Ledger { operation } => operation.contract(),
            Self::LedgerOutcome { .. } => C::READ,
            Self::BeginExport => C::READ,
            Self::ExportPage { .. } => C::READ,
            Self::BeginStateReadCut { .. }
            | Self::StateReadCutGet { .. }
            | Self::StateReadCutPage { .. }
            | Self::CloseStateReadCut { .. } => C::READ,
        }
    }

    /// A lost reply to one of these calls may conceal an accepted effect.
    /// Callers must not issue another mutation through a sibling attachment.
    pub(crate) fn may_mutate(&self) -> bool {
        match self.contract().mutation {
            Mutation::Write => true,
            Mutation::Read => false,
        }
    }

    /// Only receipt-bearing unit writes use this path. Candidate transitions
    /// and usage records need their existing typed ref/natural-key outcomes.
    fn unit_receipt_method(&self) -> Option<&'static str> {
        match self.contract().receipt {
            Receipt::Unit(method) => Some(method),
            Receipt::None
            | Receipt::CandidateCreation
            | Receipt::CandidateTransition
            | Receipt::CandidateReconciliation
            | Receipt::SelectedAbandon
            | Receipt::UsageProof => None,
        }
    }

    fn unit_receipt_bytes(&self, view: &str) -> Result<Option<(&'static str, Vec<u8>)>> {
        let Some(method) = self.unit_receipt_method() else {
            return Ok(None);
        };
        // Candidate handles belong to an attachment and change after owner
        // restart. The durable receipt is pinned to the branch, so encode its
        // stable view identity and the operation, never the wire handle.
        let encoded = match self {
            Self::View { operation, .. } => {
                serde_json::to_vec(&("kuru.unit.receipt.v1", view, method, operation))?
            }
            _ => serde_json::to_vec(&("kuru.unit.receipt.v1", view, method, self))?,
        };
        Ok(Some((method, encoded)))
    }

    pub(crate) fn unit_receipt_fingerprint(
        &self,
        view: &str,
    ) -> Result<Option<(&'static str, String)>> {
        let Some((method, encoded)) = self.unit_receipt_bytes(view)? else {
            return Ok(None);
        };
        let digest = Sha256::digest(encoded)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Some((method, digest)))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewOperation {
    Append {
        namespace: String,
        role: String,
        content: String,
    },
    AppendMessage {
        namespace: String,
        message: Message,
    },
    AppendSessionMessage {
        namespace: String,
        session_id: String,
        message: Message,
    },
    Checkpoint {
        namespace: String,
        messages: Vec<Message>,
        values: Vec<(String, Value)>,
    },
    CheckpointSession {
        namespace: String,
        session_id: String,
        messages: Vec<Message>,
        values: Vec<(String, Value)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        public_turn: Option<crate::SessionTurnCheckpoint>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<crate::SessionModeCheckpoint>,
    },
    History {
        namespace: String,
        limit: usize,
    },
    HistoryWindow {
        namespace: String,
        limit: usize,
    },
    SessionHistoryWindow {
        namespace: String,
        session_id: String,
        limit: usize,
    },
    SessionHistoryWindowAfter {
        namespace: String,
        session_id: String,
        after_exclusive: i64,
        limit: usize,
    },
    SessionCatalogPage {
        lifecycle_state: Option<crate::SessionLifecycleState>,
        cursor: Option<crate::SessionCatalogCursor>,
        expected_revision: Option<String>,
        limit: usize,
    },
    SessionCatalogRecord {
        session_id: String,
    },
    CreateSession {
        session_id: String,
        mode: Mode,
        label: String,
    },
    RenameSession {
        session_id: String,
        expected_generation: u64,
        label: String,
    },
    RemoveSession {
        session_id: String,
        expected_generation: u64,
    },
    RestoreSession {
        session_id: String,
        expected_generation: u64,
    },
    ForkSession {
        source_session_id: String,
        expected_source_generation: u64,
        source_node_id: String,
        child_session_id: String,
        label: String,
    },
    PublicTranscriptPage {
        session_id: String,
        cursor: Option<crate::PublicTranscriptCursor>,
        limit: usize,
    },
    SessionSourceSnapshot {
        actor_namespace: String,
        session_id: String,
        source_namespace: String,
        after_exclusive: i64,
        limit: usize,
    },
    CheckpointContextSummary {
        record: crate::ContextSummaryRecord,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        private_reasoning: Vec<crate::ReasoningSummaryRecord>,
    },
    ContextSummaryConfirmation {
        summary_id: String,
    },
    ContextSummaryCursor {
        actor_namespace: String,
        session_id: String,
        source_namespace: String,
    },
    ContextSummaryWindow {
        actor_namespace: String,
        summary_namespace: String,
        session_id: Option<String>,
        source_namespace: Option<String>,
        limit: usize,
    },
    Notes {
        namespace: String,
        limit: usize,
    },
    ForgetNote {
        namespace: String,
        sequence: i64,
    },
    PutMany {
        values: Vec<(String, Value)>,
    },
    PutManyConditional {
        expected: Vec<(String, crate::StateExpectation)>,
        values: Vec<(String, Value)>,
    },
    GetVersioned {
        key: String,
    },
    GetMany {
        keys: Vec<String>,
    },
    GetManyVersioned {
        keys: Vec<String>,
    },
    Get {
        key: String,
    },
    Clear {
        namespace: String,
    },
    Reconcile,
    Revision,
    Revisions {
        limit: usize,
    },
    Status,
}

impl ViewOperation {
    /// Keep every arm explicit: no wildcard may classify a new variant.
    pub(crate) fn contract(&self) -> OperationContract {
        use OperationContract as C;
        match self {
            Self::Append { .. } => C::write(Receipt::Unit("view.append")),
            Self::AppendMessage { .. } => C::write(Receipt::Unit("view.append_message")),
            Self::AppendSessionMessage { .. } => {
                C::write(Receipt::Unit("view.append_session_message"))
            }
            Self::Checkpoint { .. } => C::write(Receipt::Unit("view.checkpoint")),
            Self::CheckpointSession { .. } => C::write(Receipt::Unit("view.checkpoint_session")),
            Self::History { .. } => C::READ,
            Self::HistoryWindow { .. } => C::READ,
            Self::SessionHistoryWindow { .. } => C::READ,
            Self::SessionHistoryWindowAfter { .. } => C::READ,
            Self::SessionCatalogPage { .. } => C::READ,
            Self::SessionCatalogRecord { .. } => C::READ,
            Self::CreateSession { .. } => C::write(Receipt::Unit("view.create_session")),
            Self::RenameSession { .. } => C::write(Receipt::Unit("view.rename_session")),
            Self::RemoveSession { .. } => C::write(Receipt::Unit("view.remove_session")),
            Self::RestoreSession { .. } => C::write(Receipt::Unit("view.restore_session")),
            Self::ForkSession { .. } => C::write(Receipt::Unit("view.fork_session")),
            Self::PublicTranscriptPage { .. } => C::READ,
            Self::SessionSourceSnapshot { .. } => C::READ,
            Self::CheckpointContextSummary { .. } => {
                C::write(Receipt::Unit("view.checkpoint_context_summary"))
            }
            Self::ContextSummaryConfirmation { .. } => C::READ,
            Self::ContextSummaryCursor { .. } => C::READ,
            Self::ContextSummaryWindow { .. } => C::READ,
            Self::Notes { .. } => C::READ,
            Self::ForgetNote { .. } => C::write(Receipt::Unit("view.forget_note")),
            Self::PutMany { .. } => C::write(Receipt::Unit("view.put_many")),
            Self::PutManyConditional { .. } => C::write(Receipt::Unit("view.put_many_conditional")),
            Self::GetVersioned { .. } => C::READ,
            Self::GetMany { .. } => C::READ,
            Self::GetManyVersioned { .. } => C::READ,
            Self::Get { .. } => C::READ,
            Self::Clear { .. } => C::write(Receipt::Unit("view.clear")),
            Self::Reconcile => C::READ,
            Self::Revision => C::READ,
            Self::Revisions { .. } => C::READ,
            Self::Status => C::READ,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum LedgerOperation {
    MarkNewSession {
        session_id: String,
    },
    Admit {
        start: Box<InvocationStart>,
    },
    Observe {
        invocation_id: String,
        observation: UsageObservation,
    },
    Settle {
        invocation_id: String,
        outcome: InvocationOutcome,
    },
    Session {
        session_id: String,
    },
}

impl LedgerOperation {
    /// Keep every arm explicit: no wildcard may classify a new variant.
    pub(crate) fn contract(&self) -> OperationContract {
        use OperationContract as C;
        match self {
            Self::MarkNewSession { .. } => C::write(Receipt::UsageProof),
            Self::Admit { .. } => C::write(Receipt::UsageProof),
            Self::Observe { .. } => C::write(Receipt::UsageProof),
            Self::Settle { .. } => C::write(Receipt::UsageProof),
            Self::Session { .. } => C::READ,
        }
    }

    pub(crate) fn proof(&self) -> Result<Option<UsageProof>> {
        Ok(match self {
            Self::MarkNewSession { session_id } => Some(UsageProof::new_session(session_id)),
            Self::Admit { start } => Some(UsageProof::admit(start)?),
            Self::Observe {
                invocation_id,
                observation,
            } => Some(UsageProof::observe(invocation_id, observation)?),
            Self::Settle {
                invocation_id,
                outcome,
            } => Some(UsageProof::settle(invocation_id, *outcome)?),
            Self::Session { .. } => None,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceStatus {
    pub project: String,
    pub directory: PathBuf,
    pub branch: String,
    pub revision: String,
    pub read_only: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceReply {
    pub id: Uuid,
    pub generation: String,
    pub response: ServiceResponse,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServiceResponse {
    Success(Box<ServiceValue>),
    Rejected(ServiceFault),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServiceValue {
    SessionDriver(crate::SessionDriverProof),
    SessionDriverOutcome(crate::SessionDriverOutcome),
    LiveSessionDrivers(Vec<crate::LiveSessionDriver>),
    Unit,
    DreamLease {
        acquired: bool,
    },
    Retirement {
        accepted: bool,
    },
    Messages(Vec<Message>),
    HistoryWindow(HistoryWindow),
    SessionHistoryWindowAfter(crate::SessionHistoryWindowAfter),
    SessionCatalogPage(crate::SessionCatalogPage),
    SessionCatalogRecord(Option<crate::SessionCatalogRecord>),
    SessionLifecycleOutcome(crate::SessionLifecycleOutcome),
    PublicTranscriptPage(crate::PublicTranscriptPage),
    SessionSourceSnapshot(crate::SessionSourceSnapshot),
    ContextSummaryConfirmation(Option<crate::ContextSummaryConfirmation>),
    ContextSummaryCursor(Option<crate::ContextSummaryCursor>),
    ContextSummaryWindow(crate::ContextSummaryWindow),
    Notes(Vec<StoredNote>),
    StoredValue(Option<Value>),
    VersionedValue(Option<crate::VersionedValue>),
    StoredValues(Vec<(String, Option<Value>)>),
    VersionedValues(Vec<(String, Option<crate::VersionedValue>)>),
    Reconciled(Option<bool>),
    Outcome(OutcomeStatus),
    Revision(String),
    Revisions(Vec<Revision>),
    Status(ServiceStatus),
    CandidateStarted {
        handle: Uuid,
        base: String,
        branch: String,
    },
    CandidateOutcome(CandidateCreationOutcome),
    CandidateTransitionOutcome(CandidateTransitionResult),
    CandidateReconciled {
        result: crate::CandidateReconciliationResult,
        handle: Option<Uuid>,
    },
    CandidateReconciliationOutcome(CandidateReconciliationOutcome),
    CandidateInventory(CandidateInventoryPage),
    CandidateRefStatus(CandidateRefStatus),
    ExportStarted {
        handle: Uuid,
        provenance: ExportProvenance,
    },
    ExportPage(ExportPage),
    StateReadCutStarted {
        handle: Uuid,
        provenance: crate::store::StateReadProvenance,
    },
    StateReadPage(crate::store::StateReadPage),
    SessionUsage(SessionUsage),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    InFlight,
    Committed,
    Absent,
    StillUncertain,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateCreationOutcome {
    InFlight,
    Open {
        handle: Uuid,
        base: String,
        branch: String,
    },
    Resolved,
    StillUncertain,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateTransitionKind {
    Promote,
    Abandon,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateReconciliationOutcome {
    InFlight,
    Committed {
        handle: Uuid,
        status: CandidateRefStatus,
    },
    NotCommitted {
        handle: Uuid,
        status: CandidateRefStatus,
    },
    StillUncertain,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CandidateTransitionResult {
    InFlight,
    Promoted { revision: String },
    Abandoned,
    OpenUnchanged,
    OpenConflict,
    PreservedConflict,
    StillUncertain,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceFault {
    SessionDriverRejected(crate::SessionDriverRefusal),
    GenerationChanged,
    StorageFailed,
    CandidateConflict,
    ContextSummaryStale,
    StateStale(crate::StateStale),
    ReasoningSummaryConflict,
    ReceiptConflict,
    CandidateRefRejected(CandidateRefRefusal),
    SessionLifecycleRejected(crate::SessionLifecycleRefusal),
    SessionTurnRejected(crate::SessionTurnRefusal),
}

#[derive(Default)]
struct AttachmentState {
    client: Option<Uuid>,
    connection: Uuid,
    generation: String,
    driver: Option<crate::store::SessionClaimHandle>,
    candidates: HashMap<Uuid, Candidate>,
    exports: HashMap<Uuid, ActiveExportSnapshot>,
    state_cuts: HashMap<Uuid, crate::store::StateReadCut>,
    dream_lease: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl AttachmentState {
    /// Connection-local candidate or export handles.
    fn holds_handles(&self) -> bool {
        !self.candidates.is_empty() || !self.exports.is_empty() || !self.state_cuts.is_empty()
    }

    /// Every attachment-held resource. An attachment holding any of them
    /// never accepts idle retirement.
    fn holds_resources(&self) -> bool {
        self.holds_handles() || self.dream_lease.is_some() || self.driver.is_some()
    }
}

const COMPLETED_RECEIPT_WINDOW: usize = 4096;

/// What the operation budget allows beyond one memory statement budget
/// (`QUERY_TIMEOUT`): the time an outcome handler keeps for writing its
/// reply. A receipt-bearing write's own work fits the same arithmetic: one
/// write budget, taken before its pool acquisition, bounds the acquisition,
/// its identity statement, any validation before its pending record, the
/// write and its session's return within `QUERY_TIMEOUT`, leaving this margin
/// of the client's `OPERATION_TIMEOUT`. A store mutation, session catalog
/// write, candidate creation and usage ledger change take it once they hold
/// the write lock, so it also covers their reads before the pending record.
/// The write-lock wait, the earlier reads of writers that take their budget
/// at the acquisition (candidate promotion, transition, deletion, exclusion),
/// reconciliation after a write that ends without its receipt and multi-write
/// candidate operations are outside it, so a service write as a whole is not
/// bounded by `OPERATION_TIMEOUT`; past it, the outcome query and the
/// uncertain-write fence recover the outcome.
const REPLY_MARGIN: std::time::Duration =
    OPERATION_TIMEOUT.saturating_sub(crate::store::QUERY_TIMEOUT);
/// An outcome handler answers within this budget from its entry, so its reply
/// fits the client's `OPERATION_TIMEOUT` from sending the request.
const HANDLER_BUDGET: std::time::Duration = OPERATION_TIMEOUT.saturating_sub(REPLY_MARGIN);
/// One lock-free probe: a pool acquire and one point read. Positive evidence
/// only: a probe that does not finish within it answers nothing.
const PROBE_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
const _: () =
    assert!(HANDLER_BUDGET.as_nanos() > REPLY_MARGIN.as_nanos() + PROBE_BUDGET.as_nanos());

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReceiptKey {
    view: String,
    id: Uuid,
}

#[derive(Default)]
struct ProgressState {
    running: HashMap<ReceiptKey, usize>,
    completed: HashSet<ReceiptKey>,
    completed_order: VecDeque<ReceiptKey>,
    /// Keys a request ended without settling: its handler was dropped, so its
    /// effect may still start or run. Sticky for this generation.
    unsettled: HashSet<ReceiptKey>,
    /// Set when `unsettled` outgrew its bound; every idle key is then unknown.
    unsettled_overflow: bool,
}

/// In-process proof for a same-generation outcome query. Eviction can only
/// turn a definitive absence into "still uncertain", never the reverse.
/// A key completes only when every request registered for it settled.
#[derive(Default)]
pub(super) struct ReceiptProgress {
    state: StdMutex<ProgressState>,
    /// Woken whenever a registered request ends; waiters re-sample their key.
    settled: Notify,
    #[cfg(test)]
    seams: ProgressSeams,
}

/// Owner-local test seams. None of them changes what a request does.
#[cfg(test)]
#[derive(Default)]
struct ProgressSeams {
    registered: StdMutex<Option<Arc<RegisteredPause>>>,
    settlement: StdMutex<Option<Arc<SettlementPause>>>,
    settlement_wait: StdMutex<Option<std::time::Duration>>,
    events: StdMutex<Option<tokio::sync::mpsc::UnboundedSender<WaitEvent>>>,
    waiters: AtomicUsize,
}

/// How long an owner-local test pause waits for its release. The paused
/// request's client gives up on its reply at its reply deadline
/// (`OPERATION_TIMEOUT`), so a test that held the pause longer could not
/// observe that reply anyway.
#[cfg(test)]
const TEST_PAUSE_RELEASE_WITHIN: std::time::Duration = OPERATION_TIMEOUT;

/// One owner-local test barrier after a mutating request has registered its
/// receipt key and before it can reach Dolt. Outcome requests remain unpaused.
#[cfg(test)]
#[derive(Default)]
pub(super) struct RegisteredPause {
    pub entered: Notify,
    pub release: Notify,
}

/// One owner-local test barrier after a registered request's handler returned
/// its value and before the request settles: its effect is already durable
/// while its key still reads as running. Outcome requests remain unpaused.
#[cfg(test)]
#[derive(Default)]
pub(super) struct SettlementPause {
    pub entered: Notify,
    pub release: Notify,
}

/// Ordered observations of outcome queries waiting for settlement.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WaitEvent {
    Entered,
    Ended(SettlementWaitEnd),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SettlementWaitEnd {
    Settled,
    Exhausted,
    ClientGone,
    ClientProtocolViolation,
}

#[derive(Clone, Copy)]
enum ReceiptProgressState {
    Running,
    Completed,
    Unknown,
}

/// How a wait for one registered request to settle ended.
enum SettlementWait {
    /// A fresh sample that is no longer running.
    Settled(ReceiptProgressState),
    Exhausted,
    ClientGone,
    ClientProtocolViolation,
}

#[cfg(test)]
impl SettlementWait {
    fn end(&self) -> SettlementWaitEnd {
        match self {
            Self::Settled(_) => SettlementWaitEnd::Settled,
            Self::Exhausted => SettlementWaitEnd::Exhausted,
            Self::ClientGone => SettlementWaitEnd::ClientGone,
            Self::ClientProtocolViolation => SettlementWaitEnd::ClientProtocolViolation,
        }
    }
}

/// An outcome query's client left while its owner waited for settlement. The
/// attachment ends without a reply.
#[derive(Clone, Copy, Debug)]
enum OutcomeClientLeft {
    Gone,
    ProtocolViolation,
}

impl std::fmt::Display for OutcomeClientLeft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Gone => "outcome query client left during its settlement wait",
            Self::ProtocolViolation => {
                "outcome query client sent data while its reply was outstanding"
            }
        })
    }
}

impl std::error::Error for OutcomeClientLeft {}

impl OutcomeClientLeft {
    fn io_kind(self) -> std::io::ErrorKind {
        match self {
            Self::Gone => std::io::ErrorKind::BrokenPipe,
            Self::ProtocolViolation => std::io::ErrorKind::InvalidData,
        }
    }
}

struct RunningReceipt {
    progress: Arc<ReceiptProgress>,
    key: ReceiptKey,
    settled: bool,
}

impl RunningReceipt {
    /// The request's handler returned a value, success or fault.
    fn settle(mut self) {
        self.settled = true;
    }
}

impl ReceiptProgress {
    fn begin(self: &Arc<Self>, key: ReceiptKey) -> RunningReceipt {
        let mut state = self.state.lock().expect("receipt progress lock");
        *state.running.entry(key.clone()).or_default() += 1;
        RunningReceipt {
            progress: self.clone(),
            key,
            settled: false,
        }
    }

    fn status(&self, key: &ReceiptKey) -> ReceiptProgressState {
        let state = self.state.lock().expect("receipt progress lock");
        if state.running.contains_key(key) {
            ReceiptProgressState::Running
        } else if state.unsettled_overflow || state.unsettled.contains(key) {
            ReceiptProgressState::Unknown
        } else if state.completed.contains(key) {
            ReceiptProgressState::Completed
        } else {
            ReceiptProgressState::Unknown
        }
    }

    fn settlement_wait_limit(&self) -> std::time::Duration {
        #[cfg(test)]
        if let Some(limit) = *self
            .seams
            .settlement_wait
            .lock()
            .expect("settlement wait lock")
        {
            return limit;
        }
        crate::store::QUERY_TIMEOUT
    }

    /// Wait until `key` is no longer running, `wait_end` passes or the
    /// querying client leaves. Holds no lock and no store connection. Enabling
    /// the notification before each sample means no request end is missed.
    async fn await_settlement<S: AsyncRead + Unpin>(
        &self,
        key: &ReceiptKey,
        wait_end: Instant,
        client: &mut S,
    ) -> SettlementWait {
        #[cfg(test)]
        let _waiting = Waiting::enter(&self.seams.waiters);
        #[cfg(test)]
        let mut entered = false;
        let ended = loop {
            let notified = self.settled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let sample = self.status(key);
            if !matches!(sample, ReceiptProgressState::Running) {
                break SettlementWait::Settled(sample);
            }
            #[cfg(test)]
            if !entered {
                entered = true;
                self.wait_event(WaitEvent::Entered);
            }
            let mut byte = [0u8; 1];
            tokio::select! {
                biased;
                () = &mut notified => {}
                // Cancel-safe: a losing read consumes nothing.
                read = client.read(&mut byte) => break match read {
                    Ok(0) | Err(_) => SettlementWait::ClientGone,
                    // Framing is lost once a new request arrives unanswered.
                    Ok(_) => SettlementWait::ClientProtocolViolation,
                },
                () = tokio::time::sleep_until(wait_end) => break match self.status(key) {
                    ReceiptProgressState::Running => SettlementWait::Exhausted,
                    sample => SettlementWait::Settled(sample),
                },
            }
        };
        #[cfg(test)]
        self.wait_event(WaitEvent::Ended(ended.end()));
        ended
    }

    #[cfg(test)]
    fn wait_event(&self, event: WaitEvent) {
        if let Some(events) = &*self.seams.events.lock().expect("wait event lock") {
            let _ = events.send(event);
        }
    }

    #[cfg(test)]
    pub(super) fn pause_next(&self, pause: Arc<RegisteredPause>) {
        *self.seams.registered.lock().expect("receipt pause lock") = Some(pause);
    }

    #[cfg(test)]
    pub(super) fn pause_settlement_next(&self, pause: Arc<SettlementPause>) {
        *self.seams.settlement.lock().expect("receipt pause lock") = Some(pause);
    }

    /// Replace the settlement wait limit; `ZERO` answers in flight at once.
    #[cfg(test)]
    pub(super) fn set_settlement_wait(&self, limit: Option<std::time::Duration>) {
        *self
            .seams
            .settlement_wait
            .lock()
            .expect("settlement wait lock") = limit;
    }

    #[cfg(test)]
    pub(super) fn watch_waits(&self) -> tokio::sync::mpsc::UnboundedReceiver<WaitEvent> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        *self.seams.events.lock().expect("wait event lock") = Some(sender);
        receiver
    }

    #[cfg(test)]
    pub(super) fn waiters_for_test(&self) -> usize {
        self.seams.waiters.load(Ordering::Acquire)
    }

    #[cfg(test)]
    async fn pause_after_registration(&self) -> Result<()> {
        let pause = self
            .seams
            .registered
            .lock()
            .expect("receipt pause lock")
            .take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            tokio::time::timeout(TEST_PAUSE_RELEASE_WITHIN, pause.release.notified())
                .await
                .context("registered receipt test pause exceeded the reply deadline")?;
        }
        Ok(())
    }

    #[cfg(test)]
    async fn pause_before_settlement(&self) -> Result<()> {
        let pause = self
            .seams
            .settlement
            .lock()
            .expect("receipt pause lock")
            .take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            tokio::time::timeout(TEST_PAUSE_RELEASE_WITHIN, pause.release.notified())
                .await
                .context("settlement test pause exceeded the reply deadline")?;
        }
        Ok(())
    }
}

impl Drop for RunningReceipt {
    fn drop(&mut self) {
        {
            let mut state = self.progress.state.lock().expect("receipt progress lock");
            let remaining = {
                let remaining = state
                    .running
                    .get_mut(&self.key)
                    .expect("registered receipt request");
                *remaining -= 1;
                *remaining
            };
            if !self.settled {
                // Marks are never evicted singly: that would let a later
                // settled same-key request complete the key again.
                state.completed.remove(&self.key);
                if state.unsettled.insert(self.key.clone())
                    && state.unsettled.len() > COMPLETED_RECEIPT_WINDOW
                {
                    state.unsettled_overflow = true;
                    state.unsettled.clear();
                }
            }
            if remaining == 0 {
                state.running.remove(&self.key);
                if self.settled && !state.unsettled_overflow && !state.unsettled.contains(&self.key)
                {
                    if state.completed.insert(self.key.clone()) {
                        state.completed_order.push_back(self.key.clone());
                    }
                    while state.completed_order.len() > COMPLETED_RECEIPT_WINDOW {
                        if let Some(old) = state.completed_order.pop_front() {
                            state.completed.remove(&old);
                        }
                    }
                }
            }
        }
        self.progress.settled.notify_waiters();
    }
}

/// Counts live settlement waits for tests, including a wait whose future is
/// dropped before it returns.
#[cfg(test)]
struct Waiting<'a>(&'a AtomicUsize);

#[cfg(test)]
impl<'a> Waiting<'a> {
    fn enter(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(count)
    }
}

#[cfg(test)]
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

// Option B keeps a cancelled call's connection open until its replacement
// has connected and handshaken: one connect and a hello write and reply read,
// each bounded by `HANDSHAKE_TIMEOUT`. The owner's earliest end of such an
// abandoned connection on its own is `OPERATION_TIMEOUT` (a partial request
// frame or an unread oversized reply). The replacement must fit inside that
// window, or the owner could see no attachment from a live client.
const _: () = assert!(
    3 * super::HANDSHAKE_TIMEOUT.as_nanos() < OPERATION_TIMEOUT.as_nanos(),
    "a replacement connection must complete before the owner ends an abandoned one"
);

#[derive(Default)]
pub(super) struct Retirement {
    active: AtomicUsize,
    requested: AtomicBool,
    candidate_resolution: AtomicBool,
    notify: Notify,
    admission: super::Admission,
    /// Set once an attachment admitted by `admission` has authenticated.
    /// Before then an empty owner waits for its starter.
    reached: AtomicBool,
    #[cfg(test)]
    dispatch_pause: StdMutex<Option<Arc<DispatchPause>>>,
}

/// One owner-local test barrier after a complete request frame of any kind
/// and before its dispatch. Unlike `RegisteredPause`, it also holds reads.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct DispatchPause {
    pub entered: Notify,
    pub release: Notify,
}

impl Retirement {
    pub(super) fn new(admission: super::Admission) -> Self {
        Self {
            admission,
            ..Self::default()
        }
    }

    pub(super) fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    pub(super) fn reached(&self) -> bool {
        self.reached.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    /// Record an authenticated attachment. Only the starter's token, or any
    /// attachment for an owner started without one, marks the owner reached.
    /// The token is compared plainly: it is not a secret.
    fn admit(&self, presented: Option<&str>) {
        let reached = match self.admission {
            super::Admission::Starter(token) => {
                presented.and_then(|presented| Uuid::parse_str(presented).ok()) == Some(token)
            }
            super::Admission::AnyAttachment => true,
            #[cfg(test)]
            super::Admission::Never => false,
        };
        if reached {
            self.reached.store(true, Ordering::Release);
        }
    }

    #[cfg(test)]
    pub(super) fn pause_next_dispatch(&self, pause: Arc<DispatchPause>) {
        *self.dispatch_pause.lock().expect("dispatch pause lock") = Some(pause);
    }

    #[cfg(test)]
    async fn pause_before_dispatch(&self) -> Result<()> {
        let pause = self
            .dispatch_pause
            .lock()
            .expect("dispatch pause lock")
            .take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            tokio::time::timeout(TEST_PAUSE_RELEASE_WITHIN, pause.release.notified())
                .await
                .context("dispatch test pause exceeded the reply deadline")?;
        }
        Ok(())
    }

    pub(super) fn attached(self: &Arc<Self>) -> Option<RetainedAttachment> {
        if self.requested() || self.candidate_resolution.load(Ordering::Acquire) {
            return None;
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.requested() || self.candidate_resolution.load(Ordering::Acquire) {
            self.active.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(RetainedAttachment(self.clone()))
    }

    fn reserve_candidate_resolution(&self) -> Result<CandidateResolutionReservation<'_>> {
        if self.requested() {
            return Err(CandidateRefRejected(CandidateRefRefusal::Active).into());
        }
        self.candidate_resolution
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CandidateRefRejected(CandidateRefRefusal::Active))?;
        if self.requested() || self.active.load(Ordering::Acquire) != 1 {
            self.candidate_resolution.store(false, Ordering::Release);
            return Err(CandidateRefRejected(CandidateRefRefusal::Active).into());
        }
        Ok(CandidateResolutionReservation(&self.candidate_resolution))
    }

    pub(super) fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.notify.notified()
    }

    #[cfg(test)]
    pub(super) fn active_for_test(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    fn request_if_idle(&self) -> bool {
        if self.candidate_resolution.load(Ordering::Acquire)
            || self.active.load(Ordering::Acquire) != 1
        {
            return false;
        }
        if self.requested.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.notify.notify_one();
        true
    }
}

struct CandidateResolutionReservation<'a>(&'a AtomicBool);

impl Drop for CandidateResolutionReservation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) struct RetainedAttachment(Arc<Retirement>);

impl Drop for RetainedAttachment {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl ServiceRequest {
    pub fn new(generation: &str, call: ServiceCall) -> Self {
        Self::with_id(generation, Uuid::new_v4(), call)
    }

    pub fn with_id(generation: &str, id: Uuid, call: ServiceCall) -> Self {
        Self {
            id,
            generation: generation.to_owned(),
            call,
            driver: None,
        }
    }
}

pub(crate) fn validate_context_summary_checkpoint_request(
    checkpoint: &crate::ContextSummaryCheckpoint,
) -> Result<()> {
    crate::store::validate_context_summary_checkpoint(checkpoint)?;
    let request = ServiceRequest::with_id(
        "00000000-0000-0000-0000-000000000000",
        Uuid::nil(),
        ServiceCall::View {
            candidate: Some(Uuid::nil()),
            operation: Box::new(ViewOperation::CheckpointContextSummary {
                record: checkpoint.record.clone(),
                private_reasoning: checkpoint.private_reasoning.clone(),
            }),
        },
    );
    ensure!(
        serde_json::to_vec(&request)?.len() <= OPERATION_FRAME_LIMIT,
        "context summary checkpoint request exceeds the managed operation frame"
    );
    Ok(())
}

/// Count exact JSON bytes without retaining an additional encoded payload.
pub(crate) fn encoded_bytes(value: &impl Serialize) -> Result<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("encoded operation size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

pub(crate) fn validate_conditional_request(
    expected: &[(String, crate::StateExpectation)],
    values: &[(String, Value)],
    candidate: bool,
) -> Result<()> {
    // All request IDs and checked generation tokens are UUIDs. Serialize the
    // real operation envelope with empty arrays, then count the borrowed arrays
    // in their places; do not clone a potentially large membership payload.
    let empty = ServiceRequest::with_id(
        "00000000-0000-0000-0000-000000000000",
        Uuid::nil(),
        ServiceCall::View {
            candidate: candidate.then(Uuid::nil),
            operation: Box::new(ViewOperation::PutManyConditional {
                expected: Vec::new(),
                values: Vec::new(),
            }),
        },
    );
    let bytes = encoded_bytes(&empty)? - 4 + encoded_bytes(&expected)? + encoded_bytes(&values)?;
    ensure!(
        bytes <= OPERATION_FRAME_LIMIT,
        "conditional state request exceeds the managed operation frame"
    );
    Ok(())
}

fn state_cut_reply_bytes(kind: &str, value: &impl Serialize) -> Result<usize> {
    #[derive(Serialize)]
    struct Tagged<'a, T> {
        kind: &'a str,
        value: &'a T,
    }
    #[derive(Serialize)]
    struct Response<'a, T> {
        status: &'a str,
        value: Tagged<'a, T>,
    }
    #[derive(Serialize)]
    struct Reply<'a, T> {
        id: Uuid,
        generation: &'a str,
        response: Response<'a, T>,
    }
    encoded_bytes(&Reply {
        id: Uuid::nil(),
        generation: "00000000-0000-0000-0000-000000000000",
        response: Response {
            status: "success",
            value: Tagged { kind, value },
        },
    })
}

pub(crate) fn validate_state_cut_value_reply(value: &Option<crate::VersionedValue>) -> Result<()> {
    ensure!(
        state_cut_reply_bytes("versioned_value", value)? <= OPERATION_FRAME_LIMIT,
        "state cut value exceeds the managed operation frame"
    );
    Ok(())
}

pub(crate) fn validate_state_cut_page_reply(page: &crate::store::StateReadPage) -> Result<()> {
    ensure!(
        state_cut_reply_bytes("state_read_page", page)? <= OPERATION_FRAME_LIMIT,
        "state cut page exceeds the managed operation frame"
    );
    Ok(())
}

/// One request at a time per authenticated connection keeps reply ownership
/// unambiguous. The service can concurrently serve independent connections;
/// `MemoryStore` itself serializes short mutations.
#[cfg(test)]
pub async fn serve_one<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
) -> Result<()> {
    serve_one_with_progress(
        stream,
        authority,
        store,
        &Arc::new(ReceiptProgress::default()),
    )
    .await
}

#[cfg(test)]
pub(super) async fn serve_one_with_progress<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    progress: &Arc<ReceiptProgress>,
) -> Result<()> {
    ensure!(
        super::accept_handshake(stream, authority).await?.is_ok(),
        "memory service handshake rejected"
    );
    let request: ServiceRequest =
        read_frame(stream, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    respond(
        stream,
        authority,
        store,
        &mut AttachmentState::default(),
        None,
        progress,
        request,
    )
    .await
}

/// An open connection is an attachment. Waiting for its next complete frame
/// does not impose an idle timeout on an inference turn; partial frames remain
/// bounded, and EOF releases the attachment.
pub(super) async fn serve_attached<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    budget: Arc<Semaphore>,
    retirement: Arc<Retirement>,
    progress: Arc<ReceiptProgress>,
) -> Result<()> {
    let Ok(hello) = super::accept_handshake_identified(stream, authority).await? else {
        bail!("memory service handshake rejected");
    };
    retirement.admit(hello.starter_token.as_deref());
    let store = store.independent_public_reader();
    let mut state = AttachmentState {
        client: Some(hello.client_id.unwrap_or_else(Uuid::new_v4)),
        connection: Uuid::new_v4(),
        generation: authority.service_generation.clone(),
        ..AttachmentState::default()
    };
    let result = async {
        while let Some((request, _bytes)) = read_next(stream, budget.clone()).await? {
            #[cfg(test)]
            retirement.pause_before_dispatch().await?;
            respond(
                stream,
                authority,
                &store,
                &mut state,
                Some(&retirement),
                &progress,
                request,
            )
            .await?;
        }
        Ok(())
    }
    .await;
    // A lost attachment is not an explicit abandon request. Candidate refs
    // and their writes remain durable for exact-ref recovery; only the
    // connection-local handle is released here.
    drop(state);
    result
}

async fn read_next<R: AsyncRead + Unpin>(
    reader: &mut R,
    budget: Arc<Semaphore>,
) -> Result<Option<(ServiceRequest, OwnedSemaphorePermit)>> {
    let mut first = [0u8; 1];
    if reader.read(&mut first).await? == 0 {
        return Ok(None);
    }
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        let mut rest = [0u8; 3];
        reader.read_exact(&mut rest).await?;
        let length = u32::from_be_bytes([first[0], rest[0], rest[1], rest[2]]) as usize;
        ensure!(
            length > 0 && length <= OPERATION_FRAME_LIMIT,
            "memory service frame exceeds its limit"
        );
        let units = u32::try_from(length.div_ceil(MIB))?;
        let bytes = budget
            .acquire_many_owned(units)
            .await
            .context("memory service frame budget closed")?;
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload).await?;
        let request = serde_json::from_slice::<ServiceRequest>(&payload)
            .context("decode typed memory service request")?;
        Ok::<_, anyhow::Error>((request, bytes))
    })
    .await
    .context("memory service request frame deadline exceeded")?
    .map(Some)
}

async fn respond<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    state: &mut AttachmentState,
    retirement: Option<&Retirement>,
    progress: &Arc<ReceiptProgress>,
    request: ServiceRequest,
) -> Result<()> {
    let attached_store = state
        .client
        .map(|client| store.with_session_caller(client, request.driver.clone()));
    let store = attached_store.as_ref().unwrap_or(store);
    let response = if request.generation == authority.service_generation {
        // The receipt outlives the handler's value so that it settles only
        // after the handler returned; a dropped handler never settles.
        let (running, processed) =
            match receipt_progress_key(&request.call, request.id, state, store) {
                Err(error) => (None, Err(error)),
                Ok(key) => {
                    let running = key.map(|key| progress.begin(key));
                    let processed = process(
                        stream,
                        authority,
                        store,
                        state,
                        retirement,
                        progress,
                        #[cfg(test)]
                        running.is_some(),
                        request.id,
                        request.call,
                    )
                    .await;
                    (running, processed)
                }
            };
        #[cfg(test)]
        let processed = match running.is_some() {
            true => progress.pause_before_settlement().await.and(processed),
            false => processed,
        };
        if let Some(running) = running {
            running.settle();
        }
        if let Err(error) = &processed
            && let Some(left) = error.downcast_ref::<OutcomeClientLeft>()
        {
            // No reply: the attachment ends, releasing its budget and slot.
            return Err(std::io::Error::new(left.io_kind(), *left).into());
        }
        match processed {
            Ok(value) => ServiceResponse::Success(Box::new(value)),
            Err(error) => {
                tracing::warn!(error = %error, "memory service operation failed");
                let fault = if let Some(rejected) =
                    error.downcast_ref::<crate::SessionDriverRejected>()
                {
                    ServiceFault::SessionDriverRejected(rejected.0.clone())
                } else if error
                    .downcast_ref::<crate::store::CandidateConflict>()
                    .is_some()
                {
                    ServiceFault::CandidateConflict
                } else if let Some(stale) = error.downcast_ref::<crate::StateStale>() {
                    ServiceFault::StateStale(stale.clone())
                } else if error
                    .downcast_ref::<crate::store::ContextSummaryStale>()
                    .is_some()
                {
                    ServiceFault::ContextSummaryStale
                } else if error
                    .downcast_ref::<crate::store::ReasoningSummaryConflict>()
                    .is_some()
                {
                    ServiceFault::ReasoningSummaryConflict
                } else if error
                    .downcast_ref::<crate::store::LogicalReceiptConflict>()
                    .is_some()
                {
                    ServiceFault::ReceiptConflict
                } else if let Some(rejected) = error.downcast_ref::<CandidateRefRejected>() {
                    ServiceFault::CandidateRefRejected(rejected.0)
                } else if let Some(rejected) =
                    error.downcast_ref::<crate::SessionLifecycleRejected>()
                {
                    ServiceFault::SessionLifecycleRejected(rejected.0)
                } else if let Some(rejected) = error.downcast_ref::<crate::SessionTurnRejected>() {
                    ServiceFault::SessionTurnRejected(rejected.0)
                } else {
                    ServiceFault::StorageFailed
                };
                #[cfg(any(test, feature = "test-support"))]
                if let Some(record) = crate::store::candidate_failure_record(&error) {
                    let kind = match fault {
                        ServiceFault::SessionDriverRejected(_) => "session_driver_rejected",
                        ServiceFault::CandidateRefRejected(_) => "ref_rejected",
                        ServiceFault::CandidateConflict => "candidate_conflict",
                        ServiceFault::ContextSummaryStale => "context_summary_stale",
                        ServiceFault::StateStale(_) => "state_stale",
                        ServiceFault::ReasoningSummaryConflict => "reasoning_summary_conflict",
                        ServiceFault::ReceiptConflict => "receipt_conflict",
                        ServiceFault::SessionLifecycleRejected(_) => "session_lifecycle_rejected",
                        ServiceFault::SessionTurnRejected(_) => "session_turn_rejected",
                        ServiceFault::StorageFailed => "storage_failed",
                        ServiceFault::GenerationChanged => "generation_changed",
                    };
                    eprintln!("{record} fault={kind}");
                }
                ServiceResponse::Rejected(fault)
            }
        }
    } else {
        ServiceResponse::Rejected(ServiceFault::GenerationChanged)
    };
    write_frame(
        stream,
        &ServiceReply {
            id: request.id,
            generation: authority.service_generation.clone(),
            response,
        },
        OPERATION_FRAME_LIMIT,
        OPERATION_TIMEOUT,
    )
    .await
}

/// Route one current-generation call. Outcome queries are answered here,
/// before dispatch, and may watch `client` while they wait for settlement.
#[allow(clippy::too_many_arguments)]
async fn process<S: AsyncRead + Unpin>(
    client: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    state: &mut AttachmentState,
    retirement: Option<&Retirement>,
    progress: &ReceiptProgress,
    #[cfg(test)] registered: bool,
    id: Uuid,
    call: ServiceCall,
) -> Result<ServiceValue> {
    #[cfg(test)]
    if registered {
        progress.pause_after_registration().await?;
    }
    match call {
        ServiceCall::ReattachSessionDriver {
            original_id,
            original_generation,
            selection,
        } => {
            let identity = state
                .client
                .context("presence reattachment requires an authenticated client")?;
            let key = ReceiptKey {
                view: driver_progress_view(identity, &selection)?,
                id: original_id,
            };
            if original_generation != authority.service_generation {
                return Ok(ServiceValue::SessionDriverOutcome(
                    crate::SessionDriverOutcome::StillUncertain,
                ));
            }
            match gate_outcome(progress, &key, OutcomeProbe::None, client).await? {
                OutcomeGate::Read {
                    sample: ReceiptProgressState::Completed,
                    deadline,
                } => tokio::time::timeout_at(
                    deadline,
                    dispatch(
                        store,
                        state,
                        retirement,
                        id,
                        ServiceCall::ReattachSessionDriver {
                            original_id,
                            original_generation,
                            selection,
                        },
                    ),
                )
                .await
                .context("session presence reattachment deadline exceeded")?,
                OutcomeGate::InFlight => Ok(ServiceValue::SessionDriverOutcome(
                    crate::SessionDriverOutcome::InFlight,
                )),
                _ => Ok(ServiceValue::SessionDriverOutcome(
                    crate::SessionDriverOutcome::StillUncertain,
                )),
            }
        }
        ServiceCall::SessionDriverOutcome {
            original_id,
            original_generation,
            selection,
        } => {
            let identity = state
                .client
                .context("driver outcome requires an authenticated client")?;
            let key = ReceiptKey {
                view: driver_progress_view(identity, &selection)?,
                id: original_id,
            };
            let outcome = if original_generation != authority.service_generation {
                crate::SessionDriverOutcome::StillUncertain
            } else {
                match gate_outcome(progress, &key, OutcomeProbe::None, client).await? {
                    OutcomeGate::InFlight => crate::SessionDriverOutcome::InFlight,
                    OutcomeGate::Read {
                        sample: ReceiptProgressState::Completed,
                        deadline,
                    } => {
                        match tokio::time::timeout_at(
                            deadline,
                            store.session_driver_outcome(
                                identity,
                                &original_generation,
                                original_id,
                                &selection,
                            ),
                        )
                        .await
                        {
                            Ok(outcome) => outcome?,
                            Err(_) => crate::SessionDriverOutcome::StillUncertain,
                        }
                    }
                    _ => crate::SessionDriverOutcome::StillUncertain,
                }
            };
            Ok(ServiceValue::SessionDriverOutcome(outcome))
        }
        ServiceCall::Outcome {
            original_id,
            original_generation,
            view,
            method,
            argument_digest,
        } => {
            reconcile_outcome(
                store,
                authority,
                progress,
                OutcomeQuery {
                    id: original_id,
                    original_generation: &original_generation,
                    view: &view,
                    method: &method,
                    argument_digest: &argument_digest,
                },
                client,
            )
            .await
        }
        ServiceCall::CandidateOutcome {
            original_id,
            original_generation,
        } => {
            candidate_outcome(
                store,
                state,
                progress,
                original_id,
                &original_generation,
                client,
            )
            .await
        }
        ServiceCall::LedgerOutcome {
            original_id,
            original_generation,
            proof,
        } => {
            ledger_outcome(
                store,
                authority,
                progress,
                original_id,
                &original_generation,
                &proof,
                client,
            )
            .await
        }
        ServiceCall::CandidateTransitionOutcome {
            original_id,
            original_generation,
            transition,
            branch,
            base,
            target,
        } => {
            candidate_transition_outcome(
                store,
                authority,
                progress,
                TransitionQuery {
                    id: original_id,
                    original_generation: &original_generation,
                    kind: transition,
                    branch: &branch,
                    base: &base,
                    target: &target,
                    selected: false,
                },
                client,
            )
            .await
        }
        ServiceCall::CandidateReconciliationOutcome {
            original_id,
            original_generation,
            branch,
            from,
            live,
        } => {
            candidate_reconciliation_outcome(
                store,
                state,
                authority,
                progress,
                ReconciliationQuery {
                    id: original_id,
                    generation: &original_generation,
                    branch: &branch,
                    from: &from,
                    live: &live,
                },
                client,
            )
            .await
        }
        ServiceCall::SelectedAbandonOutcome {
            original_id,
            original_generation,
            branch,
            base,
            target,
        } => {
            candidate_transition_outcome(
                store,
                authority,
                progress,
                TransitionQuery {
                    id: original_id,
                    original_generation: &original_generation,
                    kind: CandidateTransitionKind::Abandon,
                    branch: &branch,
                    base: &base,
                    target: &target,
                    selected: true,
                },
                client,
            )
            .await
        }
        call => dispatch(store, state, retirement, id, call).await,
    }
}

fn receipt_progress_key(
    call: &ServiceCall,
    id: Uuid,
    state: &AttachmentState,
    store: &MemoryStore,
) -> Result<Option<ReceiptKey>> {
    if let ServiceCall::SelectSessionDriver { selection } = call {
        let client = state
            .client
            .context("session selection requires an authenticated client")?;
        crate::store::validate_session_driver_selection(selection)?;
        return Ok(Some(ReceiptKey {
            view: driver_progress_view(client, selection)?,
            id,
        }));
    }
    let view = match call.contract().receipt {
        Receipt::None => return Ok(None),
        Receipt::Unit(_) => unit_receipt_view(call, state)?,
        Receipt::CandidateCreation => store.candidate_branch_for_id(id),
        Receipt::UsageProof => {
            let ServiceCall::Ledger { operation } = call else {
                bail!("usage-proof contract names a non-ledger call");
            };
            if operation.proof()?.is_none() {
                return Ok(None);
            }
            "kuru_usage_v1".to_owned()
        }
        Receipt::CandidateTransition => {
            let (ServiceCall::PromoteCandidate { handle, branch, .. }
            | ServiceCall::AbandonCandidate { handle, branch, .. }) = call
            else {
                bail!("candidate-transition contract names a non-transition call");
            };
            let candidate = state
                .candidates
                .get(handle)
                .context("candidate does not belong to this attachment")?;
            ensure!(
                candidate.view().pinned_view() == branch,
                "candidate transition names the wrong pinned view"
            );
            branch.clone()
        }
        Receipt::SelectedAbandon => {
            let ServiceCall::AbandonCandidateRef {
                branch,
                base,
                target,
            } = call
            else {
                bail!("selected-abandon contract names a different call");
            };
            selected_abandon_progress_view(branch, base, target)
        }
        Receipt::CandidateReconciliation => {
            let ServiceCall::ReconcileCandidate {
                handle,
                branch,
                from,
                live,
            } = call
            else {
                bail!("candidate-reconciliation contract names a different call");
            };
            let candidate = state
                .candidates
                .get(handle)
                .context("candidate does not belong to this attachment")?;
            ensure!(
                candidate.branch() == branch,
                "candidate reconciliation names a different branch"
            );
            crate::store::validate_reconciliation_heads(from, live)?;
            reconciliation_progress_view(branch, from, live)
        }
    };
    Ok(Some(ReceiptKey { view, id }))
}

/// The stable view a unit receipt is pinned to: a candidate's branch, never
/// its attachment-local handle, or the main view.
fn unit_receipt_view(call: &ServiceCall, state: &AttachmentState) -> Result<String> {
    Ok(match call {
        ServiceCall::View {
            candidate: Some(handle),
            ..
        } => state
            .candidates
            .get(handle)
            .context("candidate does not belong to this attachment")?
            .view()
            .pinned_view()
            .to_owned(),
        // Every other call is named so a new one must choose its view here.
        ServiceCall::View {
            candidate: None, ..
        }
        | ServiceCall::RetireIfIdle
        | ServiceCall::SelectSessionDriver { .. }
        | ServiceCall::SessionDriverOutcome { .. }
        | ServiceCall::ReattachSessionDriver { .. }
        | ServiceCall::LiveSessionDrivers
        | ServiceCall::TryAcquireDreamLease
        | ServiceCall::AppendMessage { .. }
        | ServiceCall::HistoryWindow { .. }
        | ServiceCall::Notes { .. }
        | ServiceCall::PutMany { .. }
        | ServiceCall::PutReasoningSummaries { .. }
        | ServiceCall::Get { .. }
        | ServiceCall::Reconcile
        | ServiceCall::Revision
        | ServiceCall::Outcome { .. }
        | ServiceCall::BeginCandidate { .. }
        | ServiceCall::CandidateOutcome { .. }
        | ServiceCall::PromoteCandidate { .. }
        | ServiceCall::AbandonCandidate { .. }
        | ServiceCall::ReconcileCandidate { .. }
        | ServiceCall::CandidateReconciliationOutcome { .. }
        | ServiceCall::CandidateTransitionOutcome { .. }
        | ServiceCall::SelectedAbandonOutcome { .. }
        | ServiceCall::CandidateInventory { .. }
        | ServiceCall::CandidateRefStatus { .. }
        | ServiceCall::AbandonCandidateRef { .. }
        | ServiceCall::Ledger { .. }
        | ServiceCall::LedgerOutcome { .. }
        | ServiceCall::BeginExport
        | ServiceCall::ExportPage { .. } => "main".to_owned(),
        ServiceCall::BeginStateReadCut { .. }
        | ServiceCall::StateReadCutGet { .. }
        | ServiceCall::StateReadCutPage { .. }
        | ServiceCall::CloseStateReadCut { .. } => "main".to_owned(),
    })
}

fn selected_abandon_progress_view(branch: &str, base: &str, target: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"kuru.selected-abandon.progress.v1\0");
    for field in [branch, base, target] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let digest: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("selected-abandon:{digest}")
}

/// Lock-free positive evidence an outcome query may accept while its original
/// request is still registered. Only main-view unit receipts and usage-ledger
/// proofs qualify: candidate lookups open sessions on candidate refs, so they
/// stay ordered under the write guard.
enum OutcomeProbe<'a> {
    None,
    MainReceipt {
        store: &'a MemoryStore,
        query: &'a OutcomeQuery<'a>,
    },
    UsageProof {
        store: &'a MemoryStore,
        proof: &'a UsageProof,
    },
}

fn reconciliation_progress_view(branch: &str, from: &str, live: &str) -> String {
    let mut digest = Sha256::new();
    for field in ["kuru.candidate.reconciliation.v1", branch, from, live] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let digest: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("candidate-reconciliation:{digest}")
}

fn driver_progress_view(client: Uuid, selection: &crate::SessionDriverSelection) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(b"kuru.session-driver.selection.v1\0");
    hash.update(client.as_bytes());
    hash.update(serde_json::to_vec(selection)?);
    let digest: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("session-driver:{digest}"))
}

struct ReconciliationQuery<'a> {
    id: Uuid,
    generation: &'a str,
    branch: &'a str,
    from: &'a str,
    live: &'a str,
}

async fn candidate_reconciliation_outcome<S: AsyncRead + Unpin>(
    store: &MemoryStore,
    state: &mut AttachmentState,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    query: ReconciliationQuery<'_>,
    client: &mut S,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(query.generation)?.to_string() == query.generation,
        "invalid original service generation"
    );
    crate::store::validate_reconciliation_heads(query.from, query.live)?;
    let key = ReceiptKey {
        view: reconciliation_progress_view(query.branch, query.from, query.live),
        id: query.id,
    };
    let (sample, deadline) = match gate_outcome(progress, &key, OutcomeProbe::None, client).await? {
        OutcomeGate::Committed => unreachable!("candidate reconciliation has no lock-free probe"),
        OutcomeGate::InFlight => {
            return Ok(ServiceValue::CandidateReconciliationOutcome(
                CandidateReconciliationOutcome::InFlight,
            ));
        }
        OutcomeGate::StillUncertain => {
            return Ok(ServiceValue::CandidateReconciliationOutcome(
                CandidateReconciliationOutcome::StillUncertain,
            ));
        }
        OutcomeGate::Read { sample, deadline } => (sample, deadline),
    };
    if !matches!(sample, ReceiptProgressState::Completed)
        && query.generation == authority.service_generation
    {
        return Ok(ServiceValue::CandidateReconciliationOutcome(
            CandidateReconciliationOutcome::StillUncertain,
        ));
    }
    ensure!(
        state.candidates.len() < 8,
        "too many candidates on this attachment"
    );
    let recovered = tokio::time::timeout_at(
        deadline,
        store.candidate_reconciliation_outcome(query.branch, query.from, query.live),
    )
    .await;
    let result = match recovered {
        Ok(Ok(recovery)) => {
            use crate::store::CandidateReconciliationObservation as Observation;
            match recovery.observation {
                Observation::StillUncertain => CandidateReconciliationOutcome::StillUncertain,
                observation => {
                    let candidate = recovery
                        .candidate
                        .context("proved reconciliation has no checked candidate")?;
                    let status = recovery
                        .status
                        .context("proved reconciliation has no checked status")?;
                    let handle = Uuid::new_v4();
                    state.candidates.insert(handle, candidate);
                    match observation {
                        Observation::Committed { .. } => {
                            CandidateReconciliationOutcome::Committed { handle, status }
                        }
                        Observation::NotCommitted => {
                            CandidateReconciliationOutcome::NotCommitted { handle, status }
                        }
                        Observation::StillUncertain => unreachable!(),
                    }
                }
            }
        }
        Ok(Err(_)) | Err(_) => CandidateReconciliationOutcome::StillUncertain,
    };
    Ok(ServiceValue::CandidateReconciliationOutcome(result))
}

impl OutcomeProbe<'_> {
    /// `Ok(true)` only for the exact committed receipt. A receipt conflict
    /// is the fault the guarded read reports too; a miss, any other error or
    /// the probe's own timeout is discarded.
    async fn committed(&self, deadline: Instant) -> Result<bool> {
        let budget = PROBE_BUDGET.min(deadline.saturating_duration_since(Instant::now()));
        let probed = match self {
            Self::None => return Ok(false),
            Self::MainReceipt { store, query } => {
                tokio::time::timeout(
                    budget,
                    store.probe_logical_receipt(query.id, query.method, query.argument_digest),
                )
                .await
            }
            Self::UsageProof { store, proof } => {
                tokio::time::timeout(budget, async {
                    store.usage_ledger()?.inspect_proof_unguarded(proof).await
                })
                .await
            }
        };
        match probed {
            Ok(Ok(found)) => Ok(found),
            Ok(Err(error)) if error.downcast_ref::<LogicalReceiptConflict>().is_some() => {
                Err(error)
            }
            Ok(Err(_)) | Err(_) => Ok(false),
        }
    }
}

/// How an outcome handler continues once its original request's progress is
/// known.
enum OutcomeGate {
    /// The lock-free probe found the exact committed receipt.
    Committed,
    /// The original request was still registered when the wait ended.
    InFlight,
    /// Too little of the handler budget remains for a guarded read.
    StillUncertain,
    /// Run the guarded read by `deadline`, judging a miss by `sample`.
    Read {
        sample: ReceiptProgressState,
        deadline: Instant,
    },
}

/// Sample the original request's progress; while it runs, probe for positive
/// evidence, wait for settlement and probe again.
///
/// A negative or open answer comes only from a guarded read that started
/// after a `Completed` sample or under a changed generation. A `Running`
/// sample never carries into a negative answer; after a wait the status is
/// sampled again.
async fn gate_outcome<S: AsyncRead + Unpin>(
    progress: &ReceiptProgress,
    key: &ReceiptKey,
    probe: OutcomeProbe<'_>,
    client: &mut S,
) -> Result<OutcomeGate> {
    let deadline = Instant::now() + HANDLER_BUDGET;
    let mut sample = progress.status(key);
    if matches!(sample, ReceiptProgressState::Running) {
        if probe.committed(deadline).await? {
            return Ok(OutcomeGate::Committed);
        }
        // Leave room for one more probe and a guarded read.
        let wait_end = (Instant::now() + progress.settlement_wait_limit())
            .min(deadline - REPLY_MARGIN - PROBE_BUDGET);
        sample = match progress.await_settlement(key, wait_end, client).await {
            SettlementWait::Settled(sample) => sample,
            SettlementWait::Exhausted => return Ok(OutcomeGate::InFlight),
            SettlementWait::ClientGone => return Err(OutcomeClientLeft::Gone.into()),
            SettlementWait::ClientProtocolViolation => {
                return Err(OutcomeClientLeft::ProtocolViolation.into());
            }
        };
        // Answer committed evidence before queueing behind later writers.
        if probe.committed(deadline).await? {
            return Ok(OutcomeGate::Committed);
        }
    }
    if deadline.saturating_duration_since(Instant::now()) < REPLY_MARGIN {
        return Ok(match sample {
            ReceiptProgressState::Running => OutcomeGate::InFlight,
            ReceiptProgressState::Completed | ReceiptProgressState::Unknown => {
                OutcomeGate::StillUncertain
            }
        });
    }
    Ok(OutcomeGate::Read { sample, deadline })
}

async fn ledger_outcome<S: AsyncRead + Unpin>(
    store: &MemoryStore,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    id: Uuid,
    original_generation: &str,
    proof: &UsageProof,
    client: &mut S,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(original_generation)?.to_string() == original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: "kuru_usage_v1".to_owned(),
        id,
    };
    let probe = OutcomeProbe::UsageProof { store, proof };
    let (state, deadline) = match gate_outcome(progress, &key, probe, client).await? {
        OutcomeGate::Committed => return Ok(ServiceValue::Outcome(OutcomeStatus::Committed)),
        OutcomeGate::InFlight => return Ok(ServiceValue::Outcome(OutcomeStatus::InFlight)),
        OutcomeGate::StillUncertain => {
            return Ok(ServiceValue::Outcome(OutcomeStatus::StillUncertain));
        }
        OutcomeGate::Read { sample, deadline } => (sample, deadline),
    };
    let ledger = store.usage_ledger()?;
    let queried = tokio::time::timeout_at(deadline, ledger.inspect_proof(proof)).await;
    let status = match queried {
        Ok(Ok(true)) => OutcomeStatus::Committed,
        Ok(Ok(false))
            if matches!(state, ReceiptProgressState::Completed)
                || original_generation != authority.service_generation =>
        {
            OutcomeStatus::Absent
        }
        Ok(Ok(false)) | Err(_) => OutcomeStatus::StillUncertain,
        Ok(Err(error)) if error.downcast_ref::<LogicalReceiptConflict>().is_some() => {
            return Err(error);
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "usage outcome inspection is uncertain");
            OutcomeStatus::StillUncertain
        }
    };
    Ok(ServiceValue::Outcome(status))
}

struct TransitionQuery<'a> {
    id: Uuid,
    original_generation: &'a str,
    kind: CandidateTransitionKind,
    branch: &'a str,
    base: &'a str,
    target: &'a str,
    /// Selected abandonment of an inspected ref, not a handle's transition.
    selected: bool,
}

async fn candidate_transition_outcome<S: AsyncRead + Unpin>(
    store: &MemoryStore,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    query: TransitionQuery<'_>,
    client: &mut S,
) -> Result<ServiceValue> {
    use crate::store::CandidateTransitionObservation as Observation;

    let TransitionQuery {
        id,
        original_generation,
        kind,
        branch,
        base,
        target,
        selected,
    } = query;
    ensure!(
        Uuid::parse_str(original_generation)?.to_string() == original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: if selected {
            selected_abandon_progress_view(branch, base, target)
        } else {
            branch.to_owned()
        },
        id,
    };
    let (sample, deadline) = match gate_outcome(progress, &key, OutcomeProbe::None, client).await? {
        OutcomeGate::Committed => unreachable!("transition outcomes have no lock-free probe"),
        OutcomeGate::InFlight => {
            return Ok(ServiceValue::CandidateTransitionOutcome(
                CandidateTransitionResult::InFlight,
            ));
        }
        OutcomeGate::StillUncertain => {
            return Ok(ServiceValue::CandidateTransitionOutcome(
                CandidateTransitionResult::StillUncertain,
            ));
        }
        OutcomeGate::Read { sample, deadline } => (sample, deadline),
    };
    let settled = matches!(sample, ReceiptProgressState::Completed)
        || original_generation != authority.service_generation;
    let observed = tokio::time::timeout_at(
        deadline,
        store.candidate_transition_observation(branch, base, target),
    )
    .await;
    let result = match observed {
        Ok(Ok(Observation::Promoted)) => match kind {
            CandidateTransitionKind::Promote => CandidateTransitionResult::Promoted {
                revision: target.to_owned(),
            },
            CandidateTransitionKind::Abandon => CandidateTransitionResult::PreservedConflict,
        },
        Ok(Ok(Observation::Abandoned | Observation::AbandonedReclaimed)) if settled => match kind {
            CandidateTransitionKind::Abandon => CandidateTransitionResult::Abandoned,
            CandidateTransitionKind::Promote => CandidateTransitionResult::PreservedConflict,
        },
        Ok(Ok(Observation::Abandoned)) => match kind {
            CandidateTransitionKind::Abandon => CandidateTransitionResult::Abandoned,
            CandidateTransitionKind::Promote => CandidateTransitionResult::PreservedConflict,
        },
        Ok(Ok(Observation::AbandonedReclaimed)) => CandidateTransitionResult::StillUncertain,
        Ok(Ok(Observation::OpenUnchanged)) if settled => CandidateTransitionResult::OpenUnchanged,
        Ok(Ok(Observation::OpenConflict)) if settled => CandidateTransitionResult::OpenConflict,
        Ok(Ok(
            Observation::OpenUnchanged | Observation::OpenConflict | Observation::Indeterminate,
        ))
        | Err(_) => CandidateTransitionResult::StillUncertain,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "candidate transition inspection is uncertain");
            CandidateTransitionResult::StillUncertain
        }
    };
    Ok(ServiceValue::CandidateTransitionOutcome(result))
}

async fn candidate_outcome<S: AsyncRead + Unpin>(
    store: &MemoryStore,
    state: &mut AttachmentState,
    progress: &ReceiptProgress,
    id: Uuid,
    original_generation: &str,
    client: &mut S,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(original_generation)?.to_string() == original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: store.candidate_branch_for_id(id),
        id,
    };
    let deadline = match gate_outcome(progress, &key, OutcomeProbe::None, client).await? {
        OutcomeGate::Committed => unreachable!("candidate creation has no lock-free probe"),
        OutcomeGate::InFlight => {
            return Ok(ServiceValue::CandidateOutcome(
                CandidateCreationOutcome::InFlight,
            ));
        }
        OutcomeGate::StillUncertain => {
            return Ok(ServiceValue::CandidateOutcome(
                CandidateCreationOutcome::StillUncertain,
            ));
        }
        OutcomeGate::Read { deadline, .. } => deadline,
    };
    let result = tokio::time::timeout_at(deadline, store.candidate_for_id(id)).await;
    let outcome = match result {
        Ok(Ok(CandidateLookup::Open(candidate))) => {
            ensure!(
                state.candidates.len() < 8,
                "too many candidates on this attachment"
            );
            let base = candidate.base().to_owned();
            let branch = candidate.view().pinned_view().to_owned();
            let handle = Uuid::new_v4();
            state.candidates.insert(handle, *candidate);
            CandidateCreationOutcome::Open {
                handle,
                base,
                branch,
            }
        }
        Ok(Ok(CandidateLookup::Resolved)) => CandidateCreationOutcome::Resolved,
        // A ref can also be missing after explicit resolution and cleanup;
        // without a durable creation tombstone that is not noncommit proof.
        Ok(Ok(CandidateLookup::Missing)) | Err(_) => CandidateCreationOutcome::StillUncertain,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "candidate creation outcome inspection is uncertain");
            CandidateCreationOutcome::StillUncertain
        }
    };
    Ok(ServiceValue::CandidateOutcome(outcome))
}

struct OutcomeQuery<'a> {
    id: Uuid,
    original_generation: &'a str,
    view: &'a str,
    method: &'a str,
    argument_digest: &'a str,
}

async fn reconcile_outcome<S: AsyncRead + Unpin>(
    store: &MemoryStore,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    query: OutcomeQuery<'_>,
    client: &mut S,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(query.original_generation)?.to_string() == query.original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: query.view.to_owned(),
        id: query.id,
    };
    let probe = if query.view == "main" {
        OutcomeProbe::MainReceipt {
            store,
            query: &query,
        }
    } else {
        OutcomeProbe::None
    };
    let (state, deadline) = match gate_outcome(progress, &key, probe, client).await? {
        OutcomeGate::Committed => return Ok(ServiceValue::Outcome(OutcomeStatus::Committed)),
        OutcomeGate::InFlight => return Ok(ServiceValue::Outcome(OutcomeStatus::InFlight)),
        OutcomeGate::StillUncertain => {
            return Ok(ServiceValue::Outcome(OutcomeStatus::StillUncertain));
        }
        OutcomeGate::Read { sample, deadline } => (sample, deadline),
    };
    let queried = tokio::time::timeout_at(
        deadline,
        store.indexed_logical_outcome(query.view, query.id, query.method, query.argument_digest),
    )
    .await;
    let status = match queried {
        Ok(Ok(Some(true))) => OutcomeStatus::Committed,
        Ok(Ok(Some(false)))
            if matches!(state, ReceiptProgressState::Completed)
                || query.original_generation != authority.service_generation =>
        {
            OutcomeStatus::Absent
        }
        Ok(Ok(Some(false) | None)) | Err(_) => OutcomeStatus::StillUncertain,
        Ok(Err(error))
            if error
                .downcast_ref::<crate::store::LogicalReceiptConflict>()
                .is_some() =>
        {
            return Err(error);
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "memory service outcome inspection is uncertain");
            OutcomeStatus::StillUncertain
        }
    };
    Ok(ServiceValue::Outcome(status))
}

#[cfg(test)]
pub async fn request_one<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    call: ServiceCall,
) -> Result<ServiceValue> {
    super::connect_handshake(stream, authority).await?;
    request_attached(stream, authority, call).await
}

#[cfg(test)]
pub(super) async fn request_attached<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    call: ServiceCall,
) -> Result<ServiceValue> {
    resolve_response(exchange_attached(stream, authority, call).await?)
}

/// A complete, authenticated reply is a definite outcome even when it rejects
/// an operation. Only an incomplete exchange invalidates the connection.
#[cfg(test)]
pub(super) async fn exchange_attached<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    call: ServiceCall,
) -> Result<ServiceResponse> {
    exchange_attached_with_id(stream, authority, Uuid::new_v4(), call).await
}

#[cfg(test)]
pub(super) async fn exchange_attached_with_id<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    id: Uuid,
    call: ServiceCall,
) -> Result<ServiceResponse> {
    exchange_attached_for_driver(stream, authority, id, call, None).await
}

pub(super) async fn exchange_attached_for_driver<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    id: Uuid,
    call: ServiceCall,
    driver: Option<crate::SessionDriverProof>,
) -> Result<ServiceResponse> {
    let reply_deadline = call.contract().reply.deadline();
    let mut request = ServiceRequest::with_id(&authority.service_generation, id, call);
    request.driver = driver;
    write_frame(stream, &request, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    let reply: ServiceReply = read_frame(stream, OPERATION_FRAME_LIMIT, reply_deadline).await?;
    ensure!(reply.id == request.id, "memory service reply ID changed");
    ensure!(
        reply.generation == authority.service_generation,
        "memory service reply generation changed"
    );
    Ok(reply.response)
}

/// A per-attachment test barrier after a complete request frame, before the
/// call returns its reply. Dropping the call future then exercises the real
/// facade cancellation path without changing owner dispatch or persistence.
///
/// The client reads the owner's reply frame while paused and holds it until
/// `release`. A test cancels after a sibling has seen the effect; the owner
/// answers the following outcome query definitely from that evidence, so no
/// owner-side reply event is needed for that.
///
/// `replied` is a different event: a reply frame has arrived and is held. The
/// owner settles the request's receipt before it writes that frame and then
/// only waits for the next request, so a test that must cancel while the
/// owner has no request in hand (the owner-retirement cancellation tests,
/// whose serve events are then ordered by client actions alone) awaits it.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub(crate) struct ReplyPause {
    pub sent: tokio::sync::Notify,
    pub replied: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
    pub promotion_sent: AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
pub(super) async fn exchange_attached_with_id_paused<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    id: Uuid,
    call: ServiceCall,
    pause: &ReplyPause,
    driver: Option<crate::SessionDriverProof>,
) -> Result<ServiceResponse> {
    pause.promotion_sent.store(
        matches!(&call, ServiceCall::PromoteCandidate { .. }),
        Ordering::Release,
    );
    let reply_deadline = call.contract().reply.deadline();
    let mut request = ServiceRequest::with_id(&authority.service_generation, id, call);
    request.driver = driver;
    write_frame(stream, &request, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    pause.sent.notify_one();
    // One reply deadline bounds the whole pause, including the frame read. A
    // failed read is held like a reply, so the call still ends only at release.
    let reply = tokio::time::timeout(reply_deadline, async {
        let reply: Result<ServiceReply> =
            read_frame(stream, OPERATION_FRAME_LIMIT, reply_deadline).await;
        if reply.is_ok() {
            pause.replied.notify_one();
        }
        pause.release.notified().await;
        reply
    })
    .await
    .context("test client reply pause exceeded operation deadline")??;
    ensure!(reply.id == request.id, "memory service reply ID changed");
    ensure!(
        reply.generation == authority.service_generation,
        "memory service reply generation changed"
    );
    Ok(reply.response)
}

pub(super) fn resolve_response(response: ServiceResponse) -> Result<ServiceValue> {
    match response {
        ServiceResponse::Rejected(ServiceFault::SessionDriverRejected(reason)) => {
            Err(crate::SessionDriverRejected(reason).into())
        }
        ServiceResponse::Success(value) => Ok(*value),
        ServiceResponse::Rejected(ServiceFault::GenerationChanged) => {
            bail!("memory service generation changed during the operation")
        }
        ServiceResponse::Rejected(ServiceFault::StorageFailed) => {
            bail!("memory service storage operation failed")
        }
        ServiceResponse::Rejected(ServiceFault::CandidateConflict) => {
            Err(crate::store::CandidateConflict.into())
        }
        ServiceResponse::Rejected(ServiceFault::ContextSummaryStale) => {
            Err(crate::store::ContextSummaryStale.into())
        }
        ServiceResponse::Rejected(ServiceFault::StateStale(stale)) => Err(stale.into()),
        ServiceResponse::Rejected(ServiceFault::ReasoningSummaryConflict) => {
            Err(crate::store::ReasoningSummaryConflict.into())
        }
        ServiceResponse::Rejected(ServiceFault::ReceiptConflict) => {
            bail!("logical mutation ID conflicts with a different operation on this memory view")
        }
        ServiceResponse::Rejected(ServiceFault::CandidateRefRejected(reason)) => {
            Err(CandidateRefRejected(reason).into())
        }
        ServiceResponse::Rejected(ServiceFault::SessionLifecycleRejected(reason)) => {
            Err(crate::SessionLifecycleRejected(reason).into())
        }
        ServiceResponse::Rejected(ServiceFault::SessionTurnRejected(reason)) => {
            Err(crate::SessionTurnRejected(reason).into())
        }
    }
}

async fn dispatch(
    store: &MemoryStore,
    state: &mut AttachmentState,
    retirement: Option<&Retirement>,
    request_id: Uuid,
    call: ServiceCall,
) -> Result<ServiceValue> {
    let unit_receipt_view = if call.unit_receipt_method().is_some() {
        unit_receipt_view(&call, state)?
    } else {
        "main".to_owned()
    };
    let unit_receipt = call.unit_receipt_bytes(&unit_receipt_view)?;
    let value = match call {
        ServiceCall::SelectSessionDriver { selection } => {
            ensure!(
                !state.holds_handles() && state.dream_lease.is_none(),
                "session selection requires a dedicated presence attachment"
            );
            let handle = store
                .select_session_driver(
                    state
                        .client
                        .context("session selection requires an authenticated client")?,
                    state.connection,
                    &state.generation,
                    request_id,
                    &selection,
                )
                .await?;
            let proof = handle.proof().clone();
            state.driver = Some(handle);
            ServiceValue::SessionDriver(proof)
        }
        ServiceCall::ReattachSessionDriver {
            original_id,
            original_generation,
            selection,
        } => {
            ensure!(
                !state.holds_resources(),
                "presence reattachment requires an empty dedicated attachment"
            );
            ensure!(
                original_generation == state.generation,
                "session presence generation changed"
            );
            let handle = store
                .reattach_session_driver(
                    state
                        .client
                        .context("session presence requires an authenticated client")?,
                    state.connection,
                    &original_generation,
                    original_id,
                    &selection,
                )
                .await?;
            let proof = handle.proof().clone();
            state.driver = Some(handle);
            ServiceValue::SessionDriver(proof)
        }
        ServiceCall::LiveSessionDrivers => {
            ServiceValue::LiveSessionDrivers(store.live_session_drivers()?)
        }
        ServiceCall::SessionDriverOutcome { .. } => {
            unreachable!("driver outcomes are handled before dispatch")
        }
        ServiceCall::RetireIfIdle => ServiceValue::Retirement {
            accepted: !state.holds_resources()
                && retirement.is_some_and(Retirement::request_if_idle),
        },
        ServiceCall::TryAcquireDreamLease => {
            if state.dream_lease.is_none() {
                ensure!(
                    !state.holds_resources(),
                    "dream lease acquisition requires a dedicated main-view attachment"
                );
                state.dream_lease = store.try_acquire_dream_lease();
            }
            ServiceValue::DreamLease {
                acquired: state.dream_lease.is_some(),
            }
        }
        ServiceCall::AppendMessage { namespace, message } => {
            let (method, encoded) = unit_receipt.as_ref().context("missing append receipt")?;
            store
                .with_logical_receipt(request_id, method, encoded)
                .append_message(&namespace, &message)
                .await?;
            ServiceValue::Unit
        }
        ServiceCall::HistoryWindow { namespace, limit } => {
            ServiceValue::HistoryWindow(store.history_window(&namespace, limit).await?)
        }
        ServiceCall::Notes { namespace, limit } => {
            ServiceValue::Notes(store.notes(&namespace, limit).await?)
        }
        ServiceCall::PutMany { values } => {
            let (method, encoded) = unit_receipt.as_ref().context("missing state receipt")?;
            store
                .with_logical_receipt(request_id, method, encoded)
                .put_many(&values)
                .await?;
            ServiceValue::Unit
        }
        ServiceCall::PutReasoningSummaries { records } => {
            let (method, encoded) = unit_receipt
                .as_ref()
                .context("missing private reasoning summary receipt")?;
            store
                .with_logical_receipt(request_id, method, encoded)
                .put_reasoning_summaries(&records)
                .await?;
            ServiceValue::Unit
        }
        ServiceCall::Get { key } => ServiceValue::StoredValue(store.get(&key).await?),
        ServiceCall::Reconcile => ServiceValue::Reconciled(store.reconcile().await?),
        ServiceCall::Revision => ServiceValue::Revision(store.revision().await?),
        ServiceCall::Outcome { .. } => unreachable!("outcome queries are handled before dispatch"),
        ServiceCall::CandidateOutcome { .. } => {
            unreachable!("candidate outcome queries are handled before dispatch")
        }
        ServiceCall::LedgerOutcome { .. } => {
            unreachable!("usage outcome queries are handled before dispatch")
        }
        ServiceCall::CandidateTransitionOutcome { .. } => {
            unreachable!("candidate transition outcome queries are handled before dispatch")
        }
        ServiceCall::SelectedAbandonOutcome { .. } => {
            unreachable!("selected abandonment outcome queries are handled before dispatch")
        }
        ServiceCall::CandidateReconciliationOutcome { .. } => {
            unreachable!("candidate reconciliation outcomes are handled before dispatch")
        }
        ServiceCall::CandidateInventory { after, limit } => ServiceValue::CandidateInventory(
            store.candidate_inventory(after.as_deref(), limit).await?,
        ),
        ServiceCall::CandidateRefStatus { branch } => {
            ServiceValue::CandidateRefStatus(store.candidate_ref_status(&branch).await?)
        }
        ServiceCall::AbandonCandidateRef {
            branch,
            base,
            target,
        } => {
            // Handles only: a dream-lease holder is not refused here today.
            if state.holds_handles() {
                return Err(CandidateRefRejected(CandidateRefRefusal::Active).into());
            }
            let _reservation = retirement
                .context("selected candidate abandonment requires a managed owner")?
                .reserve_candidate_resolution()?;
            store.abandon_candidate_ref(&branch, &base, &target).await?;
            ServiceValue::Unit
        }
        ServiceCall::View {
            candidate,
            operation,
        } => {
            let view = match candidate {
                Some(handle) => state
                    .candidates
                    .get(&handle)
                    .context("candidate does not belong to this attachment")?
                    .view(),
                None => store.clone(),
            };
            let view = view.with_request_caller(store);
            let view = if let Some((method, encoded)) = &unit_receipt {
                view.with_logical_receipt(request_id, method, encoded)
            } else {
                view
            };
            dispatch_view(&view, *operation).await?
        }
        ServiceCall::BeginCandidate { label } => {
            ensure!(
                state.candidates.len() < 8,
                "too many candidates on this attachment"
            );
            let candidate = store.begin_candidate_with_id(&label, request_id).await?;
            let base = candidate.base().to_owned();
            let branch = candidate.view().pinned_view().to_owned();
            let handle = Uuid::new_v4();
            state.candidates.insert(handle, candidate);
            ServiceValue::CandidateStarted {
                handle,
                base,
                branch,
            }
        }
        ServiceCall::PromoteCandidate {
            handle,
            branch,
            base,
            target,
        } => {
            let revision = {
                let candidate = state
                    .candidates
                    .get(&handle)
                    .context("candidate does not belong to this attachment")?;
                ensure!(
                    candidate.view().pinned_view() == branch && candidate.base() == base,
                    "candidate transition identity changed"
                );
                candidate.promote_exact(&target).await?
            };
            state.candidates.remove(&handle);
            ServiceValue::Revision(revision)
        }
        ServiceCall::AbandonCandidate {
            handle,
            branch,
            base,
            target,
        } => {
            let candidate = state
                .candidates
                .get(&handle)
                .context("candidate does not belong to this attachment")?;
            ensure!(
                candidate.view().pinned_view() == branch && candidate.base() == base,
                "candidate transition identity changed"
            );
            candidate.abandon_exact(&target).await?;
            state.candidates.remove(&handle);
            ServiceValue::Unit
        }
        ServiceCall::ReconcileCandidate {
            handle,
            branch,
            from,
            live,
        } => {
            let candidate = state
                .candidates
                .get(&handle)
                .context("candidate does not belong to this attachment")?;
            ensure!(
                candidate.branch() == branch,
                "candidate reconciliation identity changed"
            );
            let (result, fresh) = candidate.reconcile_with_live(&from, &live).await?;
            let handle = if let Some(fresh) = fresh {
                state.candidates.remove(&handle);
                let handle = Uuid::new_v4();
                state.candidates.insert(handle, fresh);
                Some(handle)
            } else {
                None
            };
            ServiceValue::CandidateReconciled { result, handle }
        }
        ServiceCall::Ledger { operation } => dispatch_ledger(store, *operation).await?,
        ServiceCall::BeginExport => {
            ensure!(
                state.exports.len() < 8,
                "too many exports on this attachment"
            );
            let snapshot = store.begin_active_export().await?;
            let provenance = snapshot.provenance().clone();
            let handle = Uuid::new_v4();
            state.exports.insert(handle, snapshot);
            ServiceValue::ExportStarted { handle, provenance }
        }
        ServiceCall::ExportPage { handle, cursor } => {
            let snapshot = state
                .exports
                .get(&handle)
                .context("export does not belong to this attachment")?;
            ServiceValue::ExportPage(snapshot.page(cursor).await?)
        }
        ServiceCall::BeginStateReadCut { candidate } => {
            ensure!(
                state.state_cuts.len() < 8,
                "too many state cuts on this attachment"
            );
            let cut = match candidate {
                Some(selected) => {
                    store
                        .candidate_state_read_cut(&selected.branch, selected.revision)
                        .await?
                }
                None => store.begin_state_read_cut().await?,
            };
            let provenance = cut.provenance().clone();
            let handle = Uuid::new_v4();
            state.state_cuts.insert(handle, cut);
            ServiceValue::StateReadCutStarted { handle, provenance }
        }
        ServiceCall::StateReadCutGet { handle, key } => {
            let cut = state
                .state_cuts
                .get(&handle)
                .context("state cut does not belong to this attachment")?;
            ServiceValue::VersionedValue(cut.get_versioned(&key).await?)
        }
        ServiceCall::StateReadCutPage {
            handle,
            prefix,
            cursor,
        } => {
            let cut = state
                .state_cuts
                .get(&handle)
                .context("state cut does not belong to this attachment")?;
            ServiceValue::StateReadPage(cut.page(&prefix, cursor).await?)
        }
        ServiceCall::CloseStateReadCut { handle } => {
            let cut = state
                .state_cuts
                .remove(&handle)
                .context("state cut does not belong to this attachment")?;
            cut.close().await?;
            ServiceValue::Unit
        }
    };
    Ok(value)
}

async fn dispatch_view(store: &MemoryStore, operation: ViewOperation) -> Result<ServiceValue> {
    Ok(match operation {
        ViewOperation::Append {
            namespace,
            role,
            content,
        } => {
            store.append(&namespace, &role, &content).await?;
            ServiceValue::Unit
        }
        ViewOperation::AppendMessage { namespace, message } => {
            store.append_message(&namespace, &message).await?;
            ServiceValue::Unit
        }
        ViewOperation::AppendSessionMessage {
            namespace,
            session_id,
            message,
        } => {
            store
                .append_session_message(&namespace, &session_id, &message)
                .await?;
            ServiceValue::Unit
        }
        ViewOperation::Checkpoint {
            namespace,
            messages,
            values,
        } => {
            store.checkpoint(&namespace, &messages, &values).await?;
            ServiceValue::Unit
        }
        ViewOperation::CheckpointSession {
            namespace,
            session_id,
            messages,
            values,
            public_turn,
            mode,
        } => {
            if let Some(mode) = mode {
                ensure!(
                    public_turn.is_none() && messages.is_empty(),
                    "mode checkpoint cannot carry a public turn or transcript entries"
                );
                store
                    .checkpoint_session_mode(&namespace, &session_id, &values, &mode)
                    .await?;
            } else if let Some(public_turn) = public_turn {
                store
                    .checkpoint_session_turn(
                        &namespace,
                        &session_id,
                        &messages,
                        &values,
                        &public_turn,
                    )
                    .await?;
            } else {
                store
                    .checkpoint_session(&namespace, &session_id, &messages, &values)
                    .await?;
            }
            ServiceValue::Unit
        }
        ViewOperation::History { namespace, limit } => {
            ServiceValue::Messages(store.history(&namespace, limit).await?)
        }
        ViewOperation::HistoryWindow { namespace, limit } => {
            ServiceValue::HistoryWindow(store.history_window(&namespace, limit).await?)
        }
        ViewOperation::SessionHistoryWindow {
            namespace,
            session_id,
            limit,
        } => ServiceValue::HistoryWindow(
            store
                .session_history_window(&namespace, &session_id, limit)
                .await?,
        ),
        ViewOperation::SessionHistoryWindowAfter {
            namespace,
            session_id,
            after_exclusive,
            limit,
        } => ServiceValue::SessionHistoryWindowAfter(
            store
                .session_history_window_after(&namespace, &session_id, after_exclusive, limit)
                .await?,
        ),
        ViewOperation::SessionCatalogPage {
            lifecycle_state,
            cursor,
            expected_revision,
            limit,
        } => ServiceValue::SessionCatalogPage(
            store
                .session_catalog_page(
                    lifecycle_state,
                    cursor.as_ref(),
                    expected_revision.as_deref(),
                    limit,
                )
                .await?,
        ),
        ViewOperation::SessionCatalogRecord { session_id } => {
            ServiceValue::SessionCatalogRecord(store.session_catalog_record(&session_id).await?)
        }
        ViewOperation::CreateSession {
            session_id,
            mode,
            label,
        } => ServiceValue::SessionLifecycleOutcome(
            store
                .create_session_catalog(&session_id, mode, &label)
                .await?,
        ),
        ViewOperation::RenameSession {
            session_id,
            expected_generation,
            label,
        } => ServiceValue::SessionLifecycleOutcome(
            store
                .rename_session_catalog(&session_id, expected_generation, &label)
                .await?,
        ),
        ViewOperation::RemoveSession {
            session_id,
            expected_generation,
        } => ServiceValue::SessionLifecycleOutcome(
            store
                .remove_session_catalog(&session_id, expected_generation)
                .await?,
        ),
        ViewOperation::RestoreSession {
            session_id,
            expected_generation,
        } => ServiceValue::SessionLifecycleOutcome(
            store
                .restore_session_catalog(&session_id, expected_generation)
                .await?,
        ),
        ViewOperation::ForkSession {
            source_session_id,
            expected_source_generation,
            source_node_id,
            child_session_id,
            label,
        } => ServiceValue::SessionLifecycleOutcome(
            store
                .fork_session_catalog(
                    &source_session_id,
                    expected_source_generation,
                    &source_node_id,
                    &child_session_id,
                    &label,
                )
                .await?,
        ),
        ViewOperation::PublicTranscriptPage {
            session_id,
            cursor,
            limit,
        } => ServiceValue::PublicTranscriptPage(
            store
                .public_transcript_page(&session_id, cursor.as_ref(), limit)
                .await?,
        ),
        ViewOperation::SessionSourceSnapshot {
            actor_namespace,
            session_id,
            source_namespace,
            after_exclusive,
            limit,
        } => ServiceValue::SessionSourceSnapshot(
            store
                .session_source_snapshot(
                    &actor_namespace,
                    &session_id,
                    &source_namespace,
                    after_exclusive,
                    limit,
                )
                .await?,
        ),
        ViewOperation::CheckpointContextSummary {
            record,
            private_reasoning,
        } => {
            let checkpoint = crate::ContextSummaryCheckpoint {
                record,
                private_reasoning,
            };
            validate_context_summary_checkpoint_request(&checkpoint)?;
            store.checkpoint_context_summary(&checkpoint).await?;
            ServiceValue::Unit
        }
        ViewOperation::ContextSummaryConfirmation { summary_id } => {
            ServiceValue::ContextSummaryConfirmation(
                store.context_summary_confirmation(&summary_id).await?,
            )
        }
        ViewOperation::ContextSummaryCursor {
            actor_namespace,
            session_id,
            source_namespace,
        } => ServiceValue::ContextSummaryCursor(
            store
                .context_summary_cursor(&actor_namespace, &session_id, &source_namespace)
                .await?,
        ),
        ViewOperation::ContextSummaryWindow {
            actor_namespace,
            summary_namespace,
            session_id,
            source_namespace,
            limit,
        } => ServiceValue::ContextSummaryWindow(
            store
                .context_summary_window(
                    &actor_namespace,
                    &summary_namespace,
                    session_id.as_deref(),
                    source_namespace.as_deref(),
                    limit,
                )
                .await?,
        ),
        ViewOperation::Notes { namespace, limit } => {
            ServiceValue::Notes(store.notes(&namespace, limit).await?)
        }
        ViewOperation::ForgetNote {
            namespace,
            sequence,
        } => {
            store.forget_note(&namespace, sequence).await?;
            ServiceValue::Unit
        }
        ViewOperation::PutMany { values } => {
            store.put_many(&values).await?;
            ServiceValue::Unit
        }
        ViewOperation::PutManyConditional { expected, values } => {
            store.put_many_conditional(&expected, &values).await?;
            ServiceValue::Unit
        }
        ViewOperation::GetVersioned { key } => {
            ServiceValue::VersionedValue(store.get_versioned(&key).await?)
        }
        ViewOperation::GetMany { keys } => ServiceValue::StoredValues(store.get_many(&keys).await?),
        ViewOperation::GetManyVersioned { keys } => {
            ServiceValue::VersionedValues(store.get_many_versioned(&keys).await?)
        }
        ViewOperation::Get { key } => ServiceValue::StoredValue(store.get(&key).await?),
        ViewOperation::Clear { namespace } => {
            store.clear(&namespace).await?;
            ServiceValue::Unit
        }
        ViewOperation::Reconcile => ServiceValue::Reconciled(store.reconcile().await?),
        ViewOperation::Revision => ServiceValue::Revision(store.revision().await?),
        ViewOperation::Revisions { limit } => {
            ServiceValue::Revisions(store.revisions(limit).await?)
        }
        ViewOperation::Status => {
            let status = store.status().await?;
            ServiceValue::Status(ServiceStatus {
                project: status.project,
                directory: status.directory,
                branch: status.branch,
                revision: status.revision,
                read_only: status.read_only,
            })
        }
    })
}

async fn dispatch_ledger(store: &MemoryStore, operation: LedgerOperation) -> Result<ServiceValue> {
    let ledger = store.usage_ledger()?;
    Ok(match operation {
        LedgerOperation::MarkNewSession { session_id } => {
            ledger.mark_new_session(&session_id).await?;
            ServiceValue::Unit
        }
        LedgerOperation::Admit { start } => {
            ledger.admit(*start).await?;
            ServiceValue::Unit
        }
        LedgerOperation::Observe {
            invocation_id,
            observation,
        } => {
            ledger.observe(&invocation_id, observation).await?;
            ServiceValue::Unit
        }
        LedgerOperation::Settle {
            invocation_id,
            outcome,
        } => {
            ledger.settle(&invocation_id, outcome).await?;
            ServiceValue::Unit
        }
        LedgerOperation::Session { session_id } => {
            ServiceValue::SessionUsage(ledger.session(&session_id).await?)
        }
    })
}

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContextSummaryCheckpoint, ContextSummaryRecord, ReasoningSummaryRecord};
    use tokio::io::{AsyncWriteExt, duplex};

    #[test]
    fn worst_escaped_reasoning_summary_request_fits_the_rpc_frame() -> Result<()> {
        let count = 1024usize;
        let identity_bytes_per_record = 4usize;
        let text_bytes = crate::store::MAX_REASONING_SUMMARY_BATCH_STRING_BYTES
            .checked_sub(identity_bytes_per_record * count)
            .context("summary identity bytes exceed the aggregate limit")?;
        let per_record = text_bytes / count;
        let remainder = text_bytes % count;
        let records = (0..count)
            .map(|index| ReasoningSummaryRecord {
                session_id: "s".into(),
                turn_id: Some("t".into()),
                operation_id: None,
                actor_id: "a".into(),
                invocation_id: "i".into(),
                item_id: None,
                output_index: None,
                summary_index: u64::try_from(index).expect("summary index fits u64"),
                text: "\u{0001}".repeat(per_record + usize::from(index < remainder)),
            })
            .collect::<Vec<_>>();
        crate::store::validate_reasoning_summaries(&records)?;
        let request = ServiceRequest::with_id(
            "generation",
            Uuid::nil(),
            ServiceCall::PutReasoningSummaries { records },
        );
        ensure!(
            serde_json::to_vec(&request)?.len() <= OPERATION_FRAME_LIMIT,
            "worst escaped private reasoning summary request exceeds the RPC frame"
        );
        Ok(())
    }

    #[test]
    fn compact_checkpoint_enforces_exact_serialized_bound_below_rpc_frame() -> Result<()> {
        let mut checkpoint = ContextSummaryCheckpoint {
            record: ContextSummaryRecord {
                actor_namespace: "actor namespace".into(),
                session_id: "session".into(),
                source_namespace: "source namespace".into(),
                summary_namespace: "summary namespace".into(),
                source_view: "main".into(),
                source_revision: "a".repeat(64),
                after_sequence: 0,
                through_sequence: 1,
                turn_id: None,
                operation_id: Some("operation".into()),
                producer_actor_id: Some("producer".into()),
                invocation_id: "invocation".into(),
                summary: String::new(),
            },
            private_reasoning: Vec::new(),
        };
        let empty_len = serde_json::to_vec(&checkpoint)?.len();
        let mut exact = None;
        for trim in 1..=5 {
            let text_len = 16 * 1024 * 1024 - trim;
            let escaped = crate::store::MAX_CONTEXT_SUMMARY_CHECKPOINT_BYTES
                .checked_sub(empty_len + text_len)
                .context("checkpoint fixed fields exceed their serialized bound")?;
            if escaped % 5 == 0 && escaped / 5 <= text_len {
                exact = Some((text_len, escaped / 5));
                break;
            }
        }
        let (text_len, escaped) =
            exact.context("fixture could not reach exact checkpoint bound")?;
        checkpoint.record.summary = format!(
            "{}{}",
            "\u{0001}".repeat(escaped),
            "x".repeat(text_len - escaped)
        );
        assert_eq!(
            serde_json::to_vec(&checkpoint)?.len(),
            crate::store::MAX_CONTEXT_SUMMARY_CHECKPOINT_BYTES
        );
        validate_context_summary_checkpoint_request(&checkpoint)?;
        let request = ServiceRequest::with_id(
            "00000000-0000-0000-0000-000000000000",
            Uuid::nil(),
            ServiceCall::View {
                candidate: Some(Uuid::nil()),
                operation: Box::new(ViewOperation::CheckpointContextSummary {
                    record: checkpoint.record.clone(),
                    private_reasoning: Vec::new(),
                }),
            },
        );
        assert!(serde_json::to_vec(&request)?.len() < OPERATION_FRAME_LIMIT);

        checkpoint.record.summary.push('x');
        let error = validate_context_summary_checkpoint_request(&checkpoint)
            .expect_err("checkpoint accepted one serialized byte over its bound");
        assert!(
            error
                .to_string()
                .contains("context summary checkpoint exceeds 64 MiB")
        );
        Ok(())
    }

    #[test]
    fn selected_candidate_resolution_temporarily_excludes_new_attachments() -> Result<()> {
        let retirement = Arc::new(Retirement::default());
        let requester = retirement.attached().context("requester attachment")?;
        let second = retirement.attached().context("second attachment")?;
        assert!(retirement.reserve_candidate_resolution().is_err());
        drop(second);
        let reserved = retirement.reserve_candidate_resolution()?;
        assert!(retirement.attached().is_none());
        assert!(!retirement.request_if_idle());
        drop(reserved);
        let later = retirement.attached().context("later attachment")?;
        drop(later);
        drop(requester);
        Ok(())
    }

    #[test]
    fn candidate_receipt_fingerprint_uses_pinned_view_not_attachment_handle() -> Result<()> {
        let operation = || ViewOperation::Append {
            namespace: "dream".into(),
            role: "assistant".into(),
            content: "private".into(),
        };
        let first = ServiceCall::View {
            candidate: Some(Uuid::new_v4()),
            operation: Box::new(operation()),
        };
        let reattached = ServiceCall::View {
            candidate: Some(Uuid::new_v4()),
            operation: Box::new(operation()),
        };
        let view = format!("candidate_{}", Uuid::new_v4().simple());
        assert_eq!(
            first.unit_receipt_fingerprint(&view)?,
            reattached.unit_receipt_fingerprint(&view)?
        );
        assert_ne!(
            first.unit_receipt_fingerprint(&view)?,
            reattached.unit_receipt_fingerprint("main")?
        );
        Ok(())
    }

    #[test]
    fn session_cursor_history_is_read_only_and_has_no_unit_receipt() -> Result<()> {
        let call = ServiceCall::View {
            candidate: None,
            operation: Box::new(ViewOperation::SessionHistoryWindowAfter {
                namespace: "actor".into(),
                session_id: "session".into(),
                after_exclusive: 7,
                limit: 16,
            }),
        };
        assert!(!call.may_mutate());
        assert!(call.unit_receipt_bytes("main")?.is_none());
        Ok(())
    }

    #[test]
    fn optional_public_turn_checkpoint_preserves_legacy_wire_and_receipt_shape() -> Result<()> {
        let operation = ViewOperation::CheckpointSession {
            namespace: "transcript".into(),
            session_id: "session".into(),
            messages: vec![Message::text("user", "hello")],
            values: vec![("journal".into(), Value::String("started".into()))],
            public_turn: None,
            mode: None,
        };
        let encoded = serde_json::to_value(&operation)?;
        ensure!(
            encoded.get("public_turn").is_none(),
            "legacy checkpoint wire unexpectedly gained a public-turn field"
        );
        ensure!(
            encoded.get("mode").is_none(),
            "legacy checkpoint wire unexpectedly gained a mode field"
        );
        let decoded: ViewOperation = serde_json::from_value(encoded.clone())?;
        ensure!(
            matches!(
                decoded,
                ViewOperation::CheckpointSession {
                    public_turn: None,
                    mode: None,
                    ..
                }
            ),
            "legacy checkpoint wire did not decode without public-turn metadata"
        );
        let legacy = ServiceCall::View {
            candidate: None,
            operation: Box::new(operation),
        };
        let public = ServiceCall::View {
            candidate: None,
            operation: Box::new(ViewOperation::CheckpointSession {
                namespace: "transcript".into(),
                session_id: "session".into(),
                messages: vec![Message::text("user", "hello")],
                values: vec![("journal".into(), Value::String("started".into()))],
                public_turn: Some(crate::SessionTurnCheckpoint::Admit {
                    expected_generation: 0,
                    turn_id: "turn".into(),
                    label: None,
                    expected_transcript_rows: None,
                }),
                mode: None,
            }),
        };
        assert_ne!(
            legacy.unit_receipt_fingerprint("main")?,
            public.unit_receipt_fingerprint("main")?
        );
        let changed_mode = ServiceCall::View {
            candidate: None,
            operation: Box::new(ViewOperation::CheckpointSession {
                namespace: "project/transcript/session".into(),
                session_id: "session".into(),
                messages: vec![],
                values: vec![(
                    "project/session/session".into(),
                    serde_json::json!({"id":"session", "mode":"jungian", "lifecycle_generation":0}),
                )],
                public_turn: None,
                mode: Some(crate::SessionModeCheckpoint {
                    expected_generation: 0,
                    expected_mode: Mode::Ifs,
                    mode: Mode::Jungian,
                }),
            }),
        };
        assert_ne!(
            legacy.unit_receipt_fingerprint("main")?,
            changed_mode.unit_receipt_fingerprint("main")?
        );
        Ok(())
    }

    fn receipt_key() -> ReceiptKey {
        ReceiptKey {
            view: "main".into(),
            id: Uuid::new_v4(),
        }
    }

    fn is_unknown(progress: &ReceiptProgress, key: &ReceiptKey) -> bool {
        matches!(progress.status(key), ReceiptProgressState::Unknown)
    }

    /// T14 (a): a later same-ID request whose handler was dropped makes the
    /// key unknown for the rest of the generation, whatever settles later.
    #[test]
    fn unsettled_drop_after_a_settled_request_keeps_the_key_unknown() {
        let progress = Arc::new(ReceiptProgress::default());
        let key = receipt_key();
        progress.begin(key.clone()).settle();
        assert!(matches!(
            progress.status(&key),
            ReceiptProgressState::Completed
        ));
        drop(progress.begin(key.clone()));
        assert!(is_unknown(&progress, &key));
        progress.begin(key.clone()).settle();
        assert!(is_unknown(&progress, &key));
    }

    /// T14 (b): with two concurrent same-key requests, one dropped unsettled
    /// keeps the key unknown in either completion order.
    #[test]
    fn concurrent_unsettled_drop_keeps_the_key_unknown_in_either_order() {
        let progress = Arc::new(ReceiptProgress::default());
        let key = receipt_key();
        let settled = progress.begin(key.clone());
        let dropped = progress.begin(key.clone());
        drop(dropped);
        assert!(matches!(
            progress.status(&key),
            ReceiptProgressState::Running
        ));
        settled.settle();
        assert!(is_unknown(&progress, &key));

        let key = receipt_key();
        let settled = progress.begin(key.clone());
        let dropped = progress.begin(key.clone());
        settled.settle();
        drop(dropped);
        assert!(is_unknown(&progress, &key));
    }

    /// T14 (c): overflowing the bounded unsettled marks makes every key that
    /// is not running unknown, never completed, for the generation.
    #[test]
    fn unsettled_overflow_makes_every_idle_key_unknown() {
        let progress = Arc::new(ReceiptProgress::default());
        let earlier = receipt_key();
        progress.begin(earlier.clone()).settle();
        for _ in 0..=COMPLETED_RECEIPT_WINDOW {
            drop(progress.begin(receipt_key()));
        }
        assert!(is_unknown(&progress, &earlier));
        let later = receipt_key();
        let running = progress.begin(later.clone());
        assert!(matches!(
            progress.status(&later),
            ReceiptProgressState::Running
        ));
        running.settle();
        assert!(is_unknown(&progress, &later));
    }

    /// T14 (d): every request end, settled or not, wakes settlement waiters.
    #[test]
    fn every_request_end_wakes_settlement_waiters() {
        use futures::FutureExt;

        let progress = Arc::new(ReceiptProgress::default());
        for settle in [true, false] {
            let running = progress.begin(receipt_key());
            let mut notified = std::pin::pin!(progress.settled.notified());
            notified.as_mut().enable();
            if settle {
                running.settle();
            } else {
                drop(running);
            }
            assert!(
                notified.now_or_never().is_some(),
                "request end (settled: {settle}) did not wake a waiter"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn partial_request_header_expires_without_waiting_for_peer_eof() {
        let (mut client, mut server) = duplex(64);
        client.write_all(&[0]).await.unwrap();
        let error = read_next(&mut server, Arc::new(Semaphore::new(FRAME_BUDGET_MIB)))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("deadline exceeded"));
    }
}
