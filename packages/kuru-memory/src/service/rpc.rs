//! Typed storage operations carried only after an authenticated generation handshake.
//! A failed response or broken connection is never permission to replay a write.

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{InvocationOutcome, InvocationStart, Message, SessionUsage, UsageObservation};
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
use uuid::Uuid;

use super::{EndpointAuthority, read_frame, write_frame};
use crate::{
    ExportProvenance, HistoryWindow, Revision, StoredNote,
    store::{
        ActiveExportSnapshot, Candidate, CandidateLookup, ExportCursor, ExportPage, MemoryStore,
        UsageProof,
    },
};

// JSON can escape a valid 16 MiB typed message by up to six times. Keep the
// frame bounded while leaving the existing message limit representable.
const OPERATION_FRAME_LIMIT: usize = 100 * 1024 * 1024;
const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(35);
pub(super) const FRAME_BUDGET_MIB: usize = 128;
const MIB: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequest {
    pub id: Uuid,
    pub generation: String,
    pub call: ServiceCall,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceCall {
    RetireIfIdle,
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
        operation: ViewOperation,
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
    CandidateTransitionOutcome {
        original_id: Uuid,
        original_generation: String,
        transition: CandidateTransitionKind,
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
}

impl ServiceCall {
    /// A lost reply to one of these calls may conceal an accepted effect.
    /// Callers must not issue another mutation through a sibling attachment.
    pub(crate) fn may_mutate(&self) -> bool {
        match self {
            Self::RetireIfIdle
            | Self::AppendMessage { .. }
            | Self::PutMany { .. }
            | Self::BeginCandidate { .. }
            | Self::PromoteCandidate { .. }
            | Self::AbandonCandidate { .. } => true,
            Self::View { operation, .. } => matches!(
                operation,
                ViewOperation::Append { .. }
                    | ViewOperation::AppendMessage { .. }
                    | ViewOperation::Checkpoint { .. }
                    | ViewOperation::ForgetNote { .. }
                    | ViewOperation::PutMany { .. }
                    | ViewOperation::Clear { .. }
            ),
            Self::Ledger { operation } => !matches!(&**operation, LedgerOperation::Session { .. }),
            Self::HistoryWindow { .. }
            | Self::Notes { .. }
            | Self::Get { .. }
            | Self::Reconcile
            | Self::Revision
            | Self::Outcome { .. }
            | Self::CandidateOutcome { .. }
            | Self::CandidateTransitionOutcome { .. }
            | Self::LedgerOutcome { .. }
            | Self::BeginExport
            | Self::ExportPage { .. } => false,
        }
    }

    /// Only receipt-bearing unit writes use this path. Candidate transitions
    /// and usage records need their existing typed ref/natural-key outcomes.
    fn unit_receipt_method(&self) -> Option<&'static str> {
        match self {
            Self::AppendMessage { .. } => Some("append_message"),
            Self::PutMany { .. } => Some("put_many"),
            Self::View { operation, .. } => match operation {
                ViewOperation::Append { .. } => Some("view.append"),
                ViewOperation::AppendMessage { .. } => Some("view.append_message"),
                ViewOperation::Checkpoint { .. } => Some("view.checkpoint"),
                ViewOperation::ForgetNote { .. } => Some("view.forget_note"),
                ViewOperation::PutMany { .. } => Some("view.put_many"),
                ViewOperation::Clear { .. } => Some("view.clear"),
                _ => None,
            },
            _ => None,
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
    Checkpoint {
        namespace: String,
        messages: Vec<Message>,
        values: Vec<(String, Value)>,
    },
    History {
        namespace: String,
        limit: usize,
    },
    HistoryWindow {
        namespace: String,
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
    Unit,
    Retirement {
        accepted: bool,
    },
    Messages(Vec<Message>),
    HistoryWindow(HistoryWindow),
    Notes(Vec<StoredNote>),
    StoredValue(Option<Value>),
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
    ExportStarted {
        handle: Uuid,
        provenance: ExportProvenance,
    },
    ExportPage(ExportPage),
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceFault {
    GenerationChanged,
    StorageFailed,
    CandidateConflict,
    ReceiptConflict,
}

#[derive(Default)]
struct AttachmentState {
    candidates: HashMap<Uuid, Candidate>,
    exports: HashMap<Uuid, ActiveExportSnapshot>,
}

const COMPLETED_RECEIPT_WINDOW: usize = 4096;

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
}

/// In-process proof for a same-generation outcome query. Eviction can only
/// turn a definitive absence into "still uncertain", never the reverse.
#[derive(Default)]
pub(super) struct ReceiptProgress(
    StdMutex<ProgressState>,
    #[cfg(test)] StdMutex<Option<Arc<RegisteredPause>>>,
);

/// One owner-local test barrier after a mutating request has registered its
/// receipt key and before it can reach Dolt. Outcome requests remain unpaused.
#[cfg(test)]
#[derive(Default)]
pub(super) struct RegisteredPause {
    pub entered: Notify,
    pub release: Notify,
}

#[derive(Clone, Copy)]
enum ReceiptProgressState {
    Running,
    Completed,
    Unknown,
}

struct RunningReceipt {
    progress: Arc<ReceiptProgress>,
    key: ReceiptKey,
}

impl ReceiptProgress {
    fn begin(self: &Arc<Self>, key: ReceiptKey) -> RunningReceipt {
        let mut state = self.0.lock().expect("receipt progress lock");
        *state.running.entry(key.clone()).or_default() += 1;
        RunningReceipt {
            progress: self.clone(),
            key,
        }
    }

    fn status(&self, key: &ReceiptKey) -> ReceiptProgressState {
        let state = self.0.lock().expect("receipt progress lock");
        if state.running.contains_key(key) {
            ReceiptProgressState::Running
        } else if state.completed.contains(key) {
            ReceiptProgressState::Completed
        } else {
            ReceiptProgressState::Unknown
        }
    }

    #[cfg(test)]
    pub(super) fn pause_next(&self, pause: Arc<RegisteredPause>) {
        *self.1.lock().expect("receipt pause lock") = Some(pause);
    }

    #[cfg(test)]
    async fn pause_after_registration(&self) -> Result<()> {
        let pause = self.1.lock().expect("receipt pause lock").take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(10), pause.release.notified())
                .await
                .context("registered receipt test pause exceeded 10 seconds")?;
        }
        Ok(())
    }
}

impl Drop for RunningReceipt {
    fn drop(&mut self) {
        let mut state = self.progress.0.lock().expect("receipt progress lock");
        let remaining = state
            .running
            .get_mut(&self.key)
            .expect("registered receipt request");
        *remaining -= 1;
        if *remaining == 0 {
            state.running.remove(&self.key);
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

#[derive(Default)]
pub(super) struct Retirement {
    active: AtomicUsize,
    requested: AtomicBool,
    notify: Notify,
}

impl Retirement {
    pub(super) fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    pub(super) fn attached(self: &Arc<Self>) -> Option<RetainedAttachment> {
        if self.requested() {
            return None;
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        Some(RetainedAttachment(self.clone()))
    }

    pub(super) fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.notify.notified()
    }

    fn request_if_idle(&self) -> bool {
        if self.active.load(Ordering::Acquire) != 1 {
            return false;
        }
        if self.requested.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.notify.notify_one();
        true
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
        }
    }
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
    ensure!(
        super::accept_handshake(stream, authority).await?.is_ok(),
        "memory service handshake rejected"
    );
    let mut state = AttachmentState::default();
    let result = async {
        while let Some((request, _bytes)) = read_next(stream, budget.clone()).await? {
            respond(
                stream,
                authority,
                store,
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

async fn respond<S: AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    store: &MemoryStore,
    state: &mut AttachmentState,
    retirement: Option<&Retirement>,
    progress: &Arc<ReceiptProgress>,
    request: ServiceRequest,
) -> Result<()> {
    let response = if request.generation == authority.service_generation {
        let processed = async {
            let key = receipt_progress_key(&request.call, request.id, state, store)?;
            let _running = key.map(|key| progress.begin(key));
            #[cfg(test)]
            if _running.is_some() {
                progress.pause_after_registration().await?;
            }
            match request.call {
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
                    )
                    .await
                }
                ServiceCall::CandidateOutcome {
                    original_id,
                    original_generation,
                } => {
                    candidate_outcome(store, state, progress, original_id, &original_generation)
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
                        original_id,
                        &original_generation,
                        transition,
                        &branch,
                        &base,
                        &target,
                    )
                    .await
                }
                call => dispatch(store, state, retirement, request.id, call).await,
            }
        }
        .await;
        match processed {
            Ok(value) => ServiceResponse::Success(Box::new(value)),
            Err(error) => {
                tracing::warn!(error = %error, "memory service operation failed");
                ServiceResponse::Rejected(
                    if error
                        .downcast_ref::<crate::store::CandidateConflict>()
                        .is_some()
                    {
                        ServiceFault::CandidateConflict
                    } else if error
                        .downcast_ref::<crate::store::LogicalReceiptConflict>()
                        .is_some()
                    {
                        ServiceFault::ReceiptConflict
                    } else {
                        ServiceFault::StorageFailed
                    },
                )
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

fn receipt_progress_key(
    call: &ServiceCall,
    id: Uuid,
    state: &AttachmentState,
    store: &MemoryStore,
) -> Result<Option<ReceiptKey>> {
    if matches!(call, ServiceCall::BeginCandidate { .. }) {
        return Ok(Some(ReceiptKey {
            view: store.candidate_branch_for_id(id),
            id,
        }));
    }
    if let ServiceCall::Ledger { operation } = call
        && operation.proof()?.is_some()
    {
        return Ok(Some(ReceiptKey {
            view: "kuru_usage_v1".to_owned(),
            id,
        }));
    }
    if let ServiceCall::PromoteCandidate { handle, branch, .. }
    | ServiceCall::AbandonCandidate { handle, branch, .. } = call
    {
        let candidate = state
            .candidates
            .get(handle)
            .context("candidate does not belong to this attachment")?;
        ensure!(
            candidate.view().pinned_view() == branch,
            "candidate transition names the wrong pinned view"
        );
        return Ok(Some(ReceiptKey {
            view: branch.clone(),
            id,
        }));
    }
    if call.unit_receipt_method().is_none() {
        return Ok(None);
    }
    let view = match call {
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
        _ => "main".to_owned(),
    };
    Ok(Some(ReceiptKey { view, id }))
}

async fn ledger_outcome(
    store: &MemoryStore,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    id: Uuid,
    original_generation: &str,
    proof: &UsageProof,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(original_generation)?.to_string() == original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: "kuru_usage_v1".to_owned(),
        id,
    };
    let state = progress.status(&key);
    if matches!(state, ReceiptProgressState::Running) {
        return Ok(ServiceValue::Outcome(OutcomeStatus::InFlight));
    }
    let ledger = store.usage_ledger()?;
    let queried = tokio::time::timeout(OPERATION_TIMEOUT, ledger.inspect_proof(proof)).await;
    let status = match queried {
        Ok(Ok(true)) => OutcomeStatus::Committed,
        Ok(Ok(false))
            if matches!(state, ReceiptProgressState::Completed)
                || original_generation != authority.service_generation =>
        {
            OutcomeStatus::Absent
        }
        Ok(Ok(false)) | Err(_) => OutcomeStatus::StillUncertain,
        Ok(Err(error))
            if error
                .downcast_ref::<crate::store::LogicalReceiptConflict>()
                .is_some() =>
        {
            return Err(error);
        }
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "usage outcome inspection is uncertain");
            OutcomeStatus::StillUncertain
        }
    };
    Ok(ServiceValue::Outcome(status))
}

#[allow(clippy::too_many_arguments)]
async fn candidate_transition_outcome(
    store: &MemoryStore,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    id: Uuid,
    original_generation: &str,
    kind: CandidateTransitionKind,
    branch: &str,
    base: &str,
    target: &str,
) -> Result<ServiceValue> {
    use crate::store::CandidateTransitionObservation as Observation;

    ensure!(
        Uuid::parse_str(original_generation)?.to_string() == original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: branch.to_owned(),
        id,
    };
    let progress = progress.status(&key);
    if matches!(progress, ReceiptProgressState::Running) {
        return Ok(ServiceValue::CandidateTransitionOutcome(
            CandidateTransitionResult::InFlight,
        ));
    }
    let settled = matches!(progress, ReceiptProgressState::Completed)
        || original_generation != authority.service_generation;
    let observed = tokio::time::timeout(
        OPERATION_TIMEOUT,
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
        Ok(Ok(Observation::Abandoned)) => match kind {
            CandidateTransitionKind::Abandon => CandidateTransitionResult::Abandoned,
            CandidateTransitionKind::Promote => CandidateTransitionResult::PreservedConflict,
        },
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

async fn candidate_outcome(
    store: &MemoryStore,
    state: &mut AttachmentState,
    progress: &ReceiptProgress,
    id: Uuid,
    original_generation: &str,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(original_generation)?.to_string() == original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: store.candidate_branch_for_id(id),
        id,
    };
    if matches!(progress.status(&key), ReceiptProgressState::Running) {
        return Ok(ServiceValue::CandidateOutcome(
            CandidateCreationOutcome::InFlight,
        ));
    }
    let result = tokio::time::timeout(OPERATION_TIMEOUT, store.candidate_for_id(id)).await;
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

async fn reconcile_outcome(
    store: &MemoryStore,
    authority: &EndpointAuthority,
    progress: &ReceiptProgress,
    query: OutcomeQuery<'_>,
) -> Result<ServiceValue> {
    ensure!(
        Uuid::parse_str(query.original_generation)?.to_string() == query.original_generation,
        "invalid original service generation"
    );
    let key = ReceiptKey {
        view: query.view.to_owned(),
        id: query.id,
    };
    let state = progress.status(&key);
    if matches!(state, ReceiptProgressState::Running) {
        return Ok(ServiceValue::Outcome(OutcomeStatus::InFlight));
    }
    let queried = tokio::time::timeout(
        OPERATION_TIMEOUT,
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

pub(super) async fn exchange_attached_with_id<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    id: Uuid,
    call: ServiceCall,
) -> Result<ServiceResponse> {
    let request = ServiceRequest::with_id(&authority.service_generation, id, call);
    write_frame(stream, &request, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    let reply: ServiceReply = read_frame(stream, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    ensure!(reply.id == request.id, "memory service reply ID changed");
    ensure!(
        reply.generation == authority.service_generation,
        "memory service reply generation changed"
    );
    Ok(reply.response)
}

/// A per-attachment test barrier after a complete request frame, before the
/// client reads the reply. Dropping the call future then exercises the real
/// facade cancellation path without changing owner dispatch or persistence.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct ReplyPause {
    pub sent: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}

#[cfg(test)]
pub(super) async fn exchange_attached_with_id_paused<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    authority: &EndpointAuthority,
    id: Uuid,
    call: ServiceCall,
    pause: &ReplyPause,
) -> Result<ServiceResponse> {
    let request = ServiceRequest::with_id(&authority.service_generation, id, call);
    write_frame(stream, &request, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    pause.sent.notify_one();
    tokio::time::timeout(OPERATION_TIMEOUT, pause.release.notified())
        .await
        .context("test client reply pause exceeded operation deadline")?;
    let reply: ServiceReply = read_frame(stream, OPERATION_FRAME_LIMIT, OPERATION_TIMEOUT).await?;
    ensure!(reply.id == request.id, "memory service reply ID changed");
    ensure!(
        reply.generation == authority.service_generation,
        "memory service reply generation changed"
    );
    Ok(reply.response)
}

pub(super) fn resolve_response(response: ServiceResponse) -> Result<ServiceValue> {
    match response {
        ServiceResponse::Success(value) => Ok(*value),
        ServiceResponse::Rejected(ServiceFault::GenerationChanged) => {
            bail!("memory service generation changed during the operation")
        }
        ServiceResponse::Rejected(ServiceFault::StorageFailed) => {
            bail!("memory service storage operation failed")
        }
        ServiceResponse::Rejected(ServiceFault::CandidateConflict) => {
            bail!("dream candidate is stale: live memory changed since its base")
        }
        ServiceResponse::Rejected(ServiceFault::ReceiptConflict) => {
            bail!("logical mutation ID conflicts with a different operation on this memory view")
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
    let unit_receipt_view = match &call {
        ServiceCall::View {
            candidate: Some(handle),
            ..
        } if call.unit_receipt_method().is_some() => state
            .candidates
            .get(handle)
            .context("candidate does not belong to this attachment")?
            .view()
            .pinned_view()
            .to_owned(),
        _ => "main".to_owned(),
    };
    let unit_receipt = call.unit_receipt_bytes(&unit_receipt_view)?;
    let value = match call {
        ServiceCall::RetireIfIdle => ServiceValue::Retirement {
            accepted: state.candidates.is_empty()
                && state.exports.is_empty()
                && retirement.is_some_and(Retirement::request_if_idle),
        },
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
            let view = if let Some((method, encoded)) = &unit_receipt {
                view.with_logical_receipt(request_id, method, encoded)
            } else {
                view
            };
            dispatch_view(&view, operation).await?
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
            let candidate = state
                .candidates
                .get(&handle)
                .context("candidate does not belong to this attachment")?;
            ensure!(
                candidate.view().pinned_view() == branch && candidate.base() == base,
                "candidate transition identity changed"
            );
            ServiceValue::Revision(candidate.promote_exact(&target).await?)
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
        ViewOperation::Checkpoint {
            namespace,
            messages,
            values,
        } => {
            store.checkpoint(&namespace, &messages, &values).await?;
            ServiceValue::Unit
        }
        ViewOperation::History { namespace, limit } => {
            ServiceValue::Messages(store.history(&namespace, limit).await?)
        }
        ViewOperation::HistoryWindow { namespace, limit } => {
            ServiceValue::HistoryWindow(store.history_window(&namespace, limit).await?)
        }
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
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, duplex};

    #[test]
    fn candidate_receipt_fingerprint_uses_pinned_view_not_attachment_handle() -> Result<()> {
        let operation = || ViewOperation::Append {
            namespace: "dream".into(),
            role: "assistant".into(),
            content: "private".into(),
        };
        let first = ServiceCall::View {
            candidate: Some(Uuid::new_v4()),
            operation: operation(),
        };
        let reattached = ServiceCall::View {
            candidate: Some(Uuid::new_v4()),
            operation: operation(),
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
