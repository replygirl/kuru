//! Public storage handles. SQL, branch pools and Dolt lifetime stay private to
//! the local owner; a managed caller holds only generation-bound attachments.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{InvocationOutcome, InvocationStart, Message, SessionUsage, UsageObservation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use uuid::Uuid;

use crate::service::rpc::{CandidateTransitionKind, LedgerOperation, ViewOperation};
use crate::{
    ExportProvenance, HistoryWindow, MemoryOpenProgress, MemoryOpenStage, MemoryStatus,
    OpenOptions, Revision, StorageRecord, StoredNote,
    progress::ProgressReporter,
    service::{self, AttachmentFactory, ServiceAttachment, ServiceCall, ServiceValue},
    store,
};

#[derive(Clone)]
pub struct MemoryStore {
    backend: Backend,
}

pub type MemoryView = MemoryStore;

// Leave room under the owner's 32-attachment ceiling for other local clients,
// candidates and exports while allowing concurrent reads within one runtime.
const MAX_PARALLEL_CLIENT_CONNECTIONS: usize = 16;

#[derive(Clone)]
enum Backend {
    Local(store::MemoryStore),
    Remote(RemoteView),
}

#[derive(Clone)]
struct RemoteView {
    session: Arc<RemoteSession>,
    attachment: Arc<AsyncMutex<ServiceAttachment>>,
    candidate: Option<Uuid>,
    candidate_creation_id: Option<Uuid>,
    pinned_view: String,
    read_only: bool,
}

struct RemoteSession {
    closed: AtomicBool,
    uncertain_write: AtomicBool,
    pending_unit: Mutex<Option<PendingUnit>>,
    pending_candidate: Mutex<Option<PendingCandidate>>,
    pending_ledger: Mutex<Option<PendingLedger>>,
    pending_transition: Mutex<Option<PendingTransition>>,
    factory: AttachmentFactory,
    options: OpenOptions,
    project: PathBuf,
    executable: PathBuf,
    mutations: AsyncMutex<()>,
    extra_connections: Arc<Semaphore>,
    attachments: Mutex<Vec<Weak<AsyncMutex<ServiceAttachment>>>>,
}

#[derive(Clone)]
struct PendingUnit {
    id: Uuid,
    generation: String,
    view: String,
    candidate_creation_id: Option<Uuid>,
    method: &'static str,
    argument_digest: String,
}

#[derive(Clone)]
struct PendingCandidate {
    id: Uuid,
    generation: String,
}

#[derive(Clone)]
struct PendingLedger {
    id: Uuid,
    generation: String,
    proof: store::UsageProof,
}

#[derive(Clone)]
struct PendingTransition {
    id: Uuid,
    generation: String,
    kind: CandidateTransitionKind,
    branch: String,
    base: String,
    target: String,
    creation_id: Uuid,
}

/// Exact read-only result for a candidate transition whose service reply was
/// lost. `None` means this facade has no pending transition to inspect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateTransitionResolution {
    Promoted(String),
    Abandoned,
    OpenUnchanged,
    OpenConflict,
    PreservedConflict,
}

/// A settled transition and, when its exact ref is still open, a newly
/// authenticated candidate handle. The former attachment is never reused.
#[derive(Debug)]
pub struct CandidateTransitionRecovery {
    pub resolution: CandidateTransitionResolution,
    pub candidate: Option<Candidate>,
}

/// A settled candidate unit write and a newly checked handle to its exact
/// still-open ref. The old candidate attachment is never reused.
#[derive(Debug)]
pub struct CandidateUnitRecovery {
    pub committed: bool,
    pub candidate: Candidate,
}

/// Cancellation drops this guard before a reply can be inspected. It fences
/// every clone until the caller's retained receipt has a typed outcome; a
/// fresh connection by itself is not proof.
struct PendingMutation<'a> {
    uncertain: &'a AtomicBool,
    complete: bool,
}

impl PendingMutation<'_> {
    fn complete(&mut self) {
        self.complete = true;
    }
}

impl Drop for PendingMutation<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.uncertain.store(true, Ordering::Release);
        }
    }
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryStore")
            .field(
                "backend",
                &match self.backend {
                    Backend::Local(_) => "local",
                    Backend::Remote(_) => "service",
                },
            )
            .finish()
    }
}

impl RemoteSession {
    fn new_view(
        attachment: ServiceAttachment,
        options: OpenOptions,
        project: PathBuf,
        executable: PathBuf,
    ) -> Result<RemoteView> {
        let factory = attachment.factory()?;
        let read_only = options.read_only;
        let session = Arc::new(Self {
            closed: AtomicBool::new(false),
            uncertain_write: AtomicBool::new(false),
            pending_unit: Mutex::new(None),
            pending_candidate: Mutex::new(None),
            pending_ledger: Mutex::new(None),
            pending_transition: Mutex::new(None),
            factory,
            options,
            project,
            executable,
            mutations: AsyncMutex::new(()),
            extra_connections: Arc::new(Semaphore::new(MAX_PARALLEL_CLIENT_CONNECTIONS)),
            attachments: Mutex::new(Vec::new()),
        });
        let attachment = Arc::new(AsyncMutex::new(attachment));
        session.register(&attachment)?;
        Ok(RemoteView {
            session,
            attachment,
            candidate: None,
            candidate_creation_id: None,
            pinned_view: "main".to_owned(),
            read_only,
        })
    }

    fn register(&self, attachment: &Arc<AsyncMutex<ServiceAttachment>>) -> Result<()> {
        let mut attachments = self
            .attachments
            .lock()
            .map_err(|_| anyhow::anyhow!("memory attachment registry is poisoned"))?;
        ensure!(
            !self.closed.load(Ordering::Acquire),
            "memory store is closed"
        );
        attachments.retain(|attachment| attachment.strong_count() > 0);
        attachments.push(Arc::downgrade(attachment));
        Ok(())
    }

    fn ensure_open(&self) -> Result<()> {
        ensure!(
            !self.closed.load(Ordering::Acquire),
            "memory store is closed"
        );
        Ok(())
    }

    fn ensure_mutation_allowed(&self) -> Result<()> {
        self.ensure_open()?;
        ensure!(
            !self.uncertain_write.load(Ordering::Acquire),
            "memory service write outcome is uncertain; this client cannot issue another mutation"
        );
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        self.extra_connections.close();
        let attachments = std::mem::take(
            &mut *self
                .attachments
                .lock()
                .map_err(|_| anyhow::anyhow!("memory attachment registry is poisoned"))?,
        );
        for attachment in attachments {
            if let Some(attachment) = attachment.upgrade() {
                attachment.lock().await.close();
            }
        }
        Ok(())
    }

    async fn reconcile_pending_unit(&self) -> Result<Option<bool>> {
        let _mutation = self.mutations.lock().await;
        self.ensure_open()?;
        let pending = self
            .pending_unit
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending receipt is poisoned"))?
            .clone();
        let Some(pending) = pending else {
            bail!(
                "memory service write outcome is uncertain and has no unit receipt proof; recover a pending candidate or usage operation through its typed outcome"
            );
        };
        ensure!(
            pending.candidate_creation_id.is_none(),
            "candidate unit write requires typed candidate recovery before further mutation"
        );
        let mut attachment =
            service::attach_or_start(&self.options, &self.project, &self.executable)
                .await
                .context("connect for memory write outcome reconciliation")?;
        ensure!(
            attachment.store_instance() == self.factory.store_instance(),
            "memory store identity changed before write outcome reconciliation"
        );
        let current_generation = attachment.generation() == pending.generation;
        let status = attachment
            .call(ServiceCall::Outcome {
                original_id: pending.id,
                original_generation: pending.generation,
                view: pending.view,
                method: pending.method.to_owned(),
                argument_digest: pending.argument_digest,
            })
            .await?;
        let ServiceValue::Outcome(status) = status else {
            bail!("memory service returned the wrong outcome response")
        };
        match status {
            service::rpc::OutcomeStatus::Committed | service::rpc::OutcomeStatus::Absent => {
                *self
                    .pending_unit
                    .lock()
                    .map_err(|_| anyhow::anyhow!("memory pending receipt is poisoned"))? = None;
                if current_generation {
                    self.uncertain_write.store(false, Ordering::Release);
                } else {
                    // Any candidate, export or ledger handle still names the
                    // retired owner generation; require a fresh public open.
                    self.closed.store(true, Ordering::Release);
                    drop(_mutation);
                    self.close().await?;
                }
                Ok(Some(status == service::rpc::OutcomeStatus::Committed))
            }
            service::rpc::OutcomeStatus::InFlight | service::rpc::OutcomeStatus::StillUncertain => {
                bail!("memory service write outcome remains uncertain")
            }
        }
    }

    async fn recover_candidate_unit(self: &Arc<Self>) -> Result<Option<CandidateUnitRecovery>> {
        let _mutation = self.mutations.lock().await;
        self.ensure_open()?;
        let pending = self
            .pending_unit
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending receipt is poisoned"))?
            .clone();
        let Some(pending) = pending else {
            return Ok(None);
        };
        let creation_id = pending
            .candidate_creation_id
            .context("pending unit receipt is not a candidate write")?;
        let mut attachment =
            service::attach_or_start(&self.options, &self.project, &self.executable)
                .await
                .context("connect for candidate unit outcome")?;
        ensure!(
            attachment.store_instance() == self.factory.store_instance(),
            "memory store identity changed before candidate unit outcome"
        );
        let current_generation = attachment.generation() == pending.generation;
        let response = attachment
            .call(ServiceCall::Outcome {
                original_id: pending.id,
                original_generation: pending.generation.clone(),
                view: pending.view.clone(),
                method: pending.method.to_owned(),
                argument_digest: pending.argument_digest,
            })
            .await?;
        let ServiceValue::Outcome(status) = response else {
            bail!("memory service returned the wrong candidate unit outcome response")
        };
        let committed = match status {
            service::rpc::OutcomeStatus::Committed => true,
            service::rpc::OutcomeStatus::Absent => false,
            service::rpc::OutcomeStatus::InFlight | service::rpc::OutcomeStatus::StillUncertain => {
                bail!("memory service candidate unit outcome remains uncertain")
            }
        };
        let response = attachment
            .call(ServiceCall::CandidateOutcome {
                original_id: creation_id,
                original_generation: pending.generation.clone(),
            })
            .await?;
        let ServiceValue::CandidateOutcome(service::rpc::CandidateCreationOutcome::Open {
            handle,
            base,
            branch,
        }) = response
        else {
            bail!("candidate ref could not be reattached after unit outcome: {response:?}")
        };
        ensure!(
            branch == pending.view,
            "candidate unit outcome named a different pinned view"
        );
        let view = if current_generation {
            let attachment = Arc::new(AsyncMutex::new(attachment));
            self.register(&attachment)?;
            RemoteView {
                session: self.clone(),
                attachment,
                candidate: Some(handle),
                candidate_creation_id: Some(creation_id),
                pinned_view: branch,
                read_only: false,
            }
        } else {
            let mut view = RemoteSession::new_view(
                attachment,
                self.options.clone(),
                self.project.clone(),
                self.executable.clone(),
            )?;
            view.candidate = Some(handle);
            view.candidate_creation_id = Some(creation_id);
            view.pinned_view = branch;
            view
        };
        *self
            .pending_unit
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending receipt is poisoned"))? = None;
        if current_generation {
            self.uncertain_write.store(false, Ordering::Release);
        } else {
            self.closed.store(true, Ordering::Release);
            drop(_mutation);
            self.close().await?;
        }
        Ok(Some(CandidateUnitRecovery {
            committed,
            candidate: Candidate {
                backend: CandidateBackend::Remote(RemoteCandidate { view, handle, base }),
            },
        }))
    }

    async fn reconcile_pending_ledger(&self) -> Result<Option<bool>> {
        let _mutation = self.mutations.lock().await;
        self.ensure_open()?;
        let pending = self
            .pending_ledger
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending usage proof is poisoned"))?
            .clone()
            .context("memory service has no pending usage proof")?;
        let mut attachment =
            service::attach_or_start(&self.options, &self.project, &self.executable)
                .await
                .context("connect for usage outcome reconciliation")?;
        ensure!(
            attachment.store_instance() == self.factory.store_instance(),
            "memory store identity changed before usage outcome reconciliation"
        );
        let current_generation = attachment.generation() == pending.generation;
        let response = attachment
            .call(ServiceCall::LedgerOutcome {
                original_id: pending.id,
                original_generation: pending.generation,
                proof: pending.proof,
            })
            .await?;
        let ServiceValue::Outcome(status) = response else {
            bail!("memory service returned the wrong usage outcome response")
        };
        match status {
            service::rpc::OutcomeStatus::Committed | service::rpc::OutcomeStatus::Absent => {
                *self
                    .pending_ledger
                    .lock()
                    .map_err(|_| anyhow::anyhow!("memory pending usage proof is poisoned"))? = None;
                if current_generation {
                    self.uncertain_write.store(false, Ordering::Release);
                } else {
                    self.closed.store(true, Ordering::Release);
                    drop(_mutation);
                    self.close().await?;
                }
                Ok(Some(status == service::rpc::OutcomeStatus::Committed))
            }
            service::rpc::OutcomeStatus::InFlight | service::rpc::OutcomeStatus::StillUncertain => {
                bail!("memory service usage outcome remains uncertain")
            }
        }
    }

    async fn recover_candidate_transition(
        self: &Arc<Self>,
    ) -> Result<Option<CandidateTransitionRecovery>> {
        let _mutation = self.mutations.lock().await;
        self.ensure_open()?;
        let pending = self
            .pending_transition
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending candidate transition is poisoned"))?
            .clone();
        let Some(pending) = pending else {
            return Ok(None);
        };
        let mut attachment =
            service::attach_or_start(&self.options, &self.project, &self.executable)
                .await
                .context("connect for candidate transition outcome")?;
        ensure!(
            attachment.store_instance() == self.factory.store_instance(),
            "memory store identity changed before candidate transition outcome"
        );
        let current_generation = attachment.generation() == pending.generation;
        let response = attachment
            .call(ServiceCall::CandidateTransitionOutcome {
                original_id: pending.id,
                original_generation: pending.generation.clone(),
                transition: pending.kind,
                branch: pending.branch.clone(),
                base: pending.base.clone(),
                target: pending.target.clone(),
            })
            .await?;
        let ServiceValue::CandidateTransitionOutcome(result) = response else {
            bail!("memory service returned the wrong candidate transition response")
        };
        let resolved = match result {
            service::rpc::CandidateTransitionResult::Promoted { revision } => {
                CandidateTransitionResolution::Promoted(revision)
            }
            service::rpc::CandidateTransitionResult::Abandoned => {
                CandidateTransitionResolution::Abandoned
            }
            service::rpc::CandidateTransitionResult::OpenUnchanged => {
                CandidateTransitionResolution::OpenUnchanged
            }
            service::rpc::CandidateTransitionResult::OpenConflict => {
                CandidateTransitionResolution::OpenConflict
            }
            service::rpc::CandidateTransitionResult::PreservedConflict => {
                CandidateTransitionResolution::PreservedConflict
            }
            service::rpc::CandidateTransitionResult::InFlight
            | service::rpc::CandidateTransitionResult::StillUncertain => {
                bail!("memory service candidate transition remains uncertain")
            }
        };
        // An open or conflicted ref is only useful through a fresh handle on
        // the attachment which inspected it. The original stream may have
        // been lost, and its handle is never adopted into this generation.
        let open = matches!(
            resolved,
            CandidateTransitionResolution::OpenUnchanged
                | CandidateTransitionResolution::OpenConflict
        );
        let reattached = if open {
            match attachment
                .call(ServiceCall::CandidateOutcome {
                    original_id: pending.creation_id,
                    original_generation: pending.generation.clone(),
                })
                .await?
            {
                ServiceValue::CandidateOutcome(service::rpc::CandidateCreationOutcome::Open {
                    handle,
                    base,
                    branch,
                }) if base == pending.base && branch == pending.branch => {
                    Some((handle, base, branch))
                }
                ServiceValue::CandidateOutcome(other) => {
                    bail!("candidate ref could not be reattached after transition: {other:?}")
                }
                _ => bail!("memory service returned the wrong candidate reattachment response"),
            }
        } else {
            None
        };
        ensure!(
            !open || reattached.is_some(),
            "open candidate ref lost its checked handle"
        );
        let candidate = if let Some((handle, base, branch)) = reattached {
            let view = if current_generation {
                let attachment = Arc::new(AsyncMutex::new(attachment));
                self.register(&attachment)?;
                RemoteView {
                    session: self.clone(),
                    attachment,
                    candidate: Some(handle),
                    candidate_creation_id: Some(pending.creation_id),
                    pinned_view: branch,
                    read_only: false,
                }
            } else {
                let mut view = RemoteSession::new_view(
                    attachment,
                    self.options.clone(),
                    self.project.clone(),
                    self.executable.clone(),
                )?;
                view.candidate = Some(handle);
                view.candidate_creation_id = Some(pending.creation_id);
                view.pinned_view = branch;
                view
            };
            Some(Candidate {
                backend: CandidateBackend::Remote(RemoteCandidate { view, handle, base }),
            })
        } else {
            None
        };
        *self
            .pending_transition
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending candidate transition is poisoned"))? =
            None;
        if current_generation {
            self.uncertain_write.store(false, Ordering::Release);
        } else {
            self.closed.store(true, Ordering::Release);
            drop(_mutation);
            self.close().await?;
        }
        Ok(Some(CandidateTransitionRecovery {
            resolution: resolved,
            candidate,
        }))
    }

    async fn recover_candidate_begin(self: &Arc<Self>) -> Result<Option<Candidate>> {
        let _mutation = self.mutations.lock().await;
        self.ensure_open()?;
        let pending = self
            .pending_candidate
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending candidate is poisoned"))?
            .clone();
        let Some(pending) = pending else {
            return Ok(None);
        };
        let mut attachment =
            service::attach_or_start(&self.options, &self.project, &self.executable)
                .await
                .context("connect for candidate creation outcome")?;
        ensure!(
            attachment.store_instance() == self.factory.store_instance(),
            "memory store identity changed before candidate creation outcome"
        );
        let current_generation = attachment.generation() == pending.generation;
        let response = attachment
            .call(ServiceCall::CandidateOutcome {
                original_id: pending.id,
                original_generation: pending.generation,
            })
            .await?;
        let ServiceValue::CandidateOutcome(outcome) = response else {
            bail!("memory service returned the wrong candidate outcome response")
        };
        let service::rpc::CandidateCreationOutcome::Open {
            handle,
            base,
            branch,
        } = outcome
        else {
            bail!("candidate creation outcome remains unresolved: {outcome:?}")
        };
        let view = if current_generation {
            let attachment = Arc::new(AsyncMutex::new(attachment));
            self.register(&attachment)?;
            RemoteView {
                session: self.clone(),
                attachment,
                candidate: Some(handle),
                candidate_creation_id: Some(pending.id),
                pinned_view: branch,
                read_only: false,
            }
        } else {
            // Old candidate/export/ledger handles cannot be reused in the
            // successor generation. The proven branch receives a new session.
            let mut view = RemoteSession::new_view(
                attachment,
                self.options.clone(),
                self.project.clone(),
                self.executable.clone(),
            )?;
            view.candidate = Some(handle);
            view.candidate_creation_id = Some(pending.id);
            view.pinned_view = branch;
            view
        };
        *self
            .pending_candidate
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending candidate is poisoned"))? = None;
        if current_generation {
            self.uncertain_write.store(false, Ordering::Release);
        } else {
            self.closed.store(true, Ordering::Release);
            drop(_mutation);
            self.close().await?;
        }
        Ok(Some(Candidate {
            backend: CandidateBackend::Remote(RemoteCandidate { view, handle, base }),
        }))
    }
}

impl RemoteView {
    fn ensure_writable(&self) -> Result<()> {
        self.session.ensure_mutation_allowed()?;
        ensure!(!self.read_only, "this memory view is read-only");
        Ok(())
    }

    async fn checked_call(
        &self,
        attachment: &mut ServiceAttachment,
        call: ServiceCall,
    ) -> Result<ServiceValue> {
        let mutating = call.may_mutate();
        let _mutation = if mutating {
            Some(self.session.mutations.lock().await)
        } else {
            None
        };
        self.checked_call_locked(attachment, call).await
    }

    /// Candidate transitions hold the shared lock across target capture and
    /// dispatch; ordinary calls acquire it in `checked_call`.
    async fn checked_call_locked(
        &self,
        attachment: &mut ServiceAttachment,
        call: ServiceCall,
    ) -> Result<ServiceValue> {
        self.checked_call_locked_with_id(attachment, call, Uuid::new_v4())
            .await
    }

    async fn checked_call_locked_with_id(
        &self,
        attachment: &mut ServiceAttachment,
        call: ServiceCall,
        request_id: Uuid,
    ) -> Result<ServiceValue> {
        let mutating = call.may_mutate();
        let receipt = call.unit_receipt_fingerprint(&self.pinned_view)?;
        let ledger_proof = match &call {
            ServiceCall::Ledger { operation } => operation.proof()?,
            _ => None,
        };
        let transition = match &call {
            ServiceCall::PromoteCandidate {
                branch,
                base,
                target,
                ..
            } => Some((CandidateTransitionKind::Promote, branch, base, target)),
            ServiceCall::AbandonCandidate {
                branch,
                base,
                target,
                ..
            } => Some((CandidateTransitionKind::Abandon, branch, base, target)),
            _ => None,
        };
        if mutating {
            self.session.ensure_mutation_allowed()?;
        } else {
            self.session.ensure_open()?;
        }
        if !attachment.has_complete_exchange() {
            ensure!(
                self.candidate.is_none(),
                "candidate attachment was lost; inspect the durable candidate ref before continuing"
            );
            *attachment = self.session.factory.connect().await?;
        }
        let candidate_begin = matches!(&call, ServiceCall::BeginCandidate { .. });
        if let Some((method, argument_digest)) = receipt {
            *self
                .session
                .pending_unit
                .lock()
                .map_err(|_| anyhow::anyhow!("memory pending receipt is poisoned"))? =
                Some(PendingUnit {
                    id: request_id,
                    generation: attachment.generation().to_owned(),
                    view: self.pinned_view.clone(),
                    candidate_creation_id: self.candidate_creation_id,
                    method,
                    argument_digest,
                });
        }
        if candidate_begin {
            *self
                .session
                .pending_candidate
                .lock()
                .map_err(|_| anyhow::anyhow!("memory pending candidate is poisoned"))? =
                Some(PendingCandidate {
                    id: request_id,
                    generation: attachment.generation().to_owned(),
                });
        }
        if let Some(proof) = ledger_proof {
            *self
                .session
                .pending_ledger
                .lock()
                .map_err(|_| anyhow::anyhow!("memory pending usage proof is poisoned"))? =
                Some(PendingLedger {
                    id: request_id,
                    generation: attachment.generation().to_owned(),
                    proof,
                });
        }
        if let Some((kind, branch, base, target)) = transition {
            ensure!(
                self.candidate.is_some() && branch == &self.pinned_view,
                "candidate transition names the wrong pinned view"
            );
            *self.session.pending_transition.lock().map_err(|_| {
                anyhow::anyhow!("memory pending candidate transition is poisoned")
            })? = Some(PendingTransition {
                id: request_id,
                generation: attachment.generation().to_owned(),
                kind,
                branch: branch.clone(),
                base: base.clone(),
                target: target.clone(),
                creation_id: self
                    .candidate_creation_id
                    .context("candidate transition is missing its original creation identity")?,
            });
        }
        let mut pending = mutating.then(|| PendingMutation {
            uncertain: &self.session.uncertain_write,
            complete: false,
        });
        let result = attachment.call_with_id(request_id, call).await;
        if attachment.has_definite_mutation_reply() {
            if let Some(pending) = &mut pending {
                pending.complete();
            }
            if mutating {
                *self
                    .session
                    .pending_unit
                    .lock()
                    .map_err(|_| anyhow::anyhow!("memory pending receipt is poisoned"))? = None;
                if candidate_begin {
                    *self
                        .session
                        .pending_candidate
                        .lock()
                        .map_err(|_| anyhow::anyhow!("memory pending candidate is poisoned"))? =
                        None;
                }
                *self
                    .session
                    .pending_ledger
                    .lock()
                    .map_err(|_| anyhow::anyhow!("memory pending usage proof is poisoned"))? = None;
                *self.session.pending_transition.lock().map_err(|_| {
                    anyhow::anyhow!("memory pending candidate transition is poisoned")
                })? = None;
            }
        }
        if mutating && !attachment.has_definite_mutation_reply() {
            result.context(
                "memory service write outcome is uncertain; further client mutations are blocked",
            )
        } else {
            result
        }
    }

    async fn call(&self, operation: ViewOperation) -> Result<ServiceValue> {
        self.call_raw(ServiceCall::View {
            candidate: self.candidate,
            operation,
        })
        .await
    }

    async fn call_raw(&self, call: ServiceCall) -> Result<ServiceValue> {
        self.session.ensure_open()?;
        if self.candidate.is_none() {
            if let Ok(mut attachment) = self.attachment.try_lock() {
                self.session.ensure_open()?;
                return self.checked_call(&mut attachment, call).await;
            }
            let _permit = self
                .session
                .extra_connections
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| anyhow::anyhow!("memory store is closed"))?;
            self.session.ensure_open()?;
            let attachment = Arc::new(AsyncMutex::new(self.session.factory.connect().await?));
            self.session.register(&attachment)?;
            let mut attachment = attachment.lock().await;
            self.session.ensure_open()?;
            return self.checked_call(&mut attachment, call).await;
        }
        let mut attachment = self.attachment.lock().await;
        self.session.ensure_open()?;
        self.checked_call(&mut attachment, call).await
    }

    async fn fork(&self) -> Result<Self> {
        self.session.ensure_open()?;
        let attachment = self.session.factory.connect().await?;
        let attachment = Arc::new(AsyncMutex::new(attachment));
        self.session.register(&attachment)?;
        Ok(Self {
            session: self.session.clone(),
            attachment,
            candidate: None,
            candidate_creation_id: None,
            pinned_view: "main".to_owned(),
            read_only: self.read_only,
        })
    }
}

fn unit(value: ServiceValue) -> Result<()> {
    ensure!(
        matches!(value, ServiceValue::Unit),
        "memory service returned the wrong response"
    );
    Ok(())
}

impl MemoryStore {
    pub fn exists(data_dir: &Path, project_scope: &str) -> Result<bool> {
        store::MemoryStore::exists(data_dir, project_scope)
    }

    /// Existing direct local opens remain usable by isolated fixtures and
    /// explicit maintenance. The project store lock excludes a live owner.
    pub async fn open(options: OpenOptions) -> Result<Self> {
        Ok(Self {
            backend: Backend::Local(store::MemoryStore::open(options).await?),
        })
    }

    pub fn open_observed(
        options: OpenOptions,
    ) -> (
        MemoryOpenProgress,
        impl std::future::Future<Output = Result<Self>> + Send + 'static,
    ) {
        let (progress, opening) = store::MemoryStore::open_observed(options);
        (progress, async move {
            Ok(Self {
                backend: Backend::Local(opening.await?),
            })
        })
    }

    /// Ordinary CLI opens attach to the checked per-project owner. The CLI
    /// still holds its existing conversation-driver lease in this phase.
    pub fn open_managed_observed(
        options: OpenOptions,
        project: std::path::PathBuf,
        executable: std::path::PathBuf,
    ) -> (
        MemoryOpenProgress,
        impl std::future::Future<Output = Result<Self>> + Send + 'static,
    ) {
        let (progress, mut reporter) = ProgressReporter::observed();
        let opening = async move {
            reporter.report(MemoryOpenStage::WaitingForProjectOwnership);
            let attachment = if options.read_only {
                match service::attach_existing(&options, &project).await? {
                    Some(attachment) => attachment,
                    None => {
                        let local = store::MemoryStore::open(options).await?;
                        reporter.report(MemoryOpenStage::Ready);
                        return Ok(Self {
                            backend: Backend::Local(local),
                        });
                    }
                }
            } else {
                service::attach_or_start(&options, &project, &executable).await?
            };
            let remote = RemoteSession::new_view(attachment, options, project, executable)?;
            reporter.report(MemoryOpenStage::Ready);
            Ok(Self {
                backend: Backend::Remote(remote),
            })
        };
        (progress, opening)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn temporary() -> Result<Self> {
        Ok(Self {
            backend: Backend::Local(store::MemoryStore::temporary().await?),
        })
    }

    pub fn usage_ledger(&self) -> Result<UsageLedger> {
        match &self.backend {
            Backend::Local(store) => Ok(UsageLedger {
                backend: LedgerBackend::Local(store.usage_ledger()?),
            }),
            Backend::Remote(remote) => {
                remote.session.ensure_open()?;
                Ok(UsageLedger {
                    backend: LedgerBackend::Remote(remote.clone()),
                })
            }
        }
    }

    pub async fn append(&self, namespace: &str, role: &str, content: &str) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.append(namespace, role, content).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::Append {
                            namespace: namespace.into(),
                            role: role.into(),
                            content: content.into(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn append_message(&self, namespace: &str, message: &Message) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.append_message(namespace, message).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::AppendMessage {
                            namespace: namespace.into(),
                            message: message.clone(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn checkpoint(
        &self,
        namespace: &str,
        messages: &[Message],
        values: &[(String, Value)],
    ) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.checkpoint(namespace, messages, values).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::Checkpoint {
                            namespace: namespace.into(),
                            messages: messages.to_vec(),
                            values: values.to_vec(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn history(&self, namespace: &str, limit: usize) -> Result<Vec<Message>> {
        match &self.backend {
            Backend::Local(store) => store.history(namespace, limit).await,
            Backend::Remote(remote) => match remote
                .call(ViewOperation::History {
                    namespace: namespace.into(),
                    limit,
                })
                .await?
            {
                ServiceValue::Messages(messages) => Ok(messages),
                _ => bail!("memory service returned the wrong history response"),
            },
        }
    }

    pub async fn history_window(&self, namespace: &str, limit: usize) -> Result<HistoryWindow> {
        match &self.backend {
            Backend::Local(store) => store.history_window(namespace, limit).await,
            Backend::Remote(remote) => match remote
                .call(ViewOperation::HistoryWindow {
                    namespace: namespace.into(),
                    limit,
                })
                .await?
            {
                ServiceValue::HistoryWindow(window) => Ok(window),
                _ => bail!("memory service returned the wrong history-window response"),
            },
        }
    }

    pub async fn notes(&self, namespace: &str, limit: usize) -> Result<Vec<StoredNote>> {
        match &self.backend {
            Backend::Local(store) => store.notes(namespace, limit).await,
            Backend::Remote(remote) => match remote
                .call(ViewOperation::Notes {
                    namespace: namespace.into(),
                    limit,
                })
                .await?
            {
                ServiceValue::Notes(notes) => Ok(notes),
                _ => bail!("memory service returned the wrong notes response"),
            },
        }
    }

    pub async fn forget_note(&self, namespace: &str, sequence: i64) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.forget_note(namespace, sequence).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::ForgetNote {
                            namespace: namespace.into(),
                            sequence,
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn put(&self, key: &str, value: &Value) -> Result<()> {
        self.put_many(&[(key.to_owned(), value.clone())]).await
    }

    pub async fn put_many(&self, values: &[(String, Value)]) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.put_many(values).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::PutMany {
                            values: values.to_vec(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn get(&self, key: &str) -> Result<Option<Value>> {
        match &self.backend {
            Backend::Local(store) => store.get(key).await,
            Backend::Remote(remote) => {
                match remote.call(ViewOperation::Get { key: key.into() }).await? {
                    ServiceValue::StoredValue(value) => Ok(value),
                    _ => bail!("memory service returned the wrong state response"),
                }
            }
        }
    }

    pub async fn clear(&self, namespace: &str) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.clear(namespace).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::Clear {
                            namespace: namespace.into(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn reconcile(&self) -> Result<Option<bool>> {
        match &self.backend {
            Backend::Local(store) => store.reconcile().await,
            Backend::Remote(remote) => {
                if remote.session.uncertain_write.load(Ordering::Acquire) {
                    let has_ledger_proof = remote
                        .session
                        .pending_ledger
                        .lock()
                        .map_err(|_| anyhow::anyhow!("memory pending usage proof is poisoned"))?
                        .is_some();
                    if has_ledger_proof {
                        return remote.session.reconcile_pending_ledger().await;
                    }
                    return remote.session.reconcile_pending_unit().await;
                }
                match remote.call(ViewOperation::Reconcile).await? {
                    ServiceValue::Reconciled(value) => Ok(value),
                    _ => bail!("memory service returned the wrong reconciliation response"),
                }
            }
        }
    }

    /// Resolve a lost candidate unit-write reply from its indexed receipt,
    /// then reattach the exact still-open ref without replaying that write.
    /// A generic `reconcile` cannot release this fence because it cannot
    /// return a usable generation-bound candidate handle.
    pub async fn recover_candidate_unit(&self) -> Result<Option<CandidateUnitRecovery>> {
        match &self.backend {
            Backend::Local(_) => Ok(None),
            Backend::Remote(remote) => remote.session.recover_candidate_unit().await,
        }
    }

    pub async fn begin_candidate(&self, label: &str) -> Result<Candidate> {
        match &self.backend {
            Backend::Local(store) => Ok(Candidate {
                backend: CandidateBackend::Local(store.begin_candidate(label).await?),
            }),
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                let dedicated = remote.fork().await?;
                let creation_id = Uuid::new_v4();
                let ServiceValue::CandidateStarted {
                    handle,
                    base,
                    branch,
                } = ({
                    let mut attachment = dedicated.attachment.lock().await;
                    let _mutation = dedicated.session.mutations.lock().await;
                    dedicated
                        .checked_call_locked_with_id(
                            &mut attachment,
                            ServiceCall::BeginCandidate {
                                label: label.into(),
                            },
                            creation_id,
                        )
                        .await?
                })
                else {
                    bail!("memory service returned the wrong candidate response")
                };
                let view = RemoteView {
                    candidate: Some(handle),
                    candidate_creation_id: Some(creation_id),
                    pinned_view: branch,
                    ..dedicated
                };
                Ok(Candidate {
                    backend: CandidateBackend::Remote(RemoteCandidate { view, handle, base }),
                })
            }
        }
    }

    /// Resolve a previously sent candidate Begin without issuing it again.
    /// A missing/resolved exact ref leaves the shared mutation fence in place.
    pub async fn recover_candidate_begin(&self) -> Result<Option<Candidate>> {
        match &self.backend {
            Backend::Local(_) => Ok(None),
            Backend::Remote(remote) => remote.session.recover_candidate_begin().await,
        }
    }

    /// Inspect the exact ref and captured base/target after a lost candidate
    /// promotion or abandonment reply. This never repeats the transition.
    pub async fn recover_candidate_transition(
        &self,
    ) -> Result<Option<CandidateTransitionRecovery>> {
        match &self.backend {
            Backend::Local(_) => Ok(None),
            Backend::Remote(remote) => remote.session.recover_candidate_transition().await,
        }
    }

    pub async fn revision(&self) -> Result<String> {
        match &self.backend {
            Backend::Local(store) => store.revision().await,
            Backend::Remote(remote) => match remote.call(ViewOperation::Revision).await? {
                ServiceValue::Revision(revision) => Ok(revision),
                _ => bail!("memory service returned the wrong revision response"),
            },
        }
    }

    pub async fn revisions(&self, limit: usize) -> Result<Vec<Revision>> {
        match &self.backend {
            Backend::Local(store) => store.revisions(limit).await,
            Backend::Remote(remote) => match remote.call(ViewOperation::Revisions { limit }).await?
            {
                ServiceValue::Revisions(revisions) => Ok(revisions),
                _ => bail!("memory service returned the wrong revisions response"),
            },
        }
    }

    pub async fn status(&self) -> Result<MemoryStatus> {
        match &self.backend {
            Backend::Local(store) => store.status().await,
            Backend::Remote(remote) => match remote.call(ViewOperation::Status).await? {
                ServiceValue::Status(status) => Ok(MemoryStatus {
                    engine: "dolt",
                    engine_version: crate::provision::DOLT_VERSION,
                    project: status.project,
                    directory: status.directory,
                    branch: status.branch,
                    revision: status.revision,
                    read_only: status.read_only || remote.read_only,
                }),
                _ => bail!("memory service returned the wrong status response"),
            },
        }
    }

    pub async fn begin_active_export(&self) -> Result<ActiveExportSnapshot> {
        match &self.backend {
            Backend::Local(store) => Ok(ActiveExportSnapshot {
                backend: ExportBackend::Local(store.begin_active_export().await?),
            }),
            Backend::Remote(remote) => {
                let dedicated = remote.fork().await?;
                let ServiceValue::ExportStarted { handle, provenance } =
                    dedicated.call_raw(ServiceCall::BeginExport).await?
                else {
                    bail!("memory service returned the wrong export response")
                };
                Ok(ActiveExportSnapshot {
                    backend: ExportBackend::Remote(RemoteExport {
                        view: dedicated,
                        handle,
                        provenance,
                    }),
                })
            }
        }
    }

    pub async fn purge(options: OpenOptions) -> Result<crate::PurgeOutcome> {
        let _permit = service::acquire_maintenance_permit(&options).await?;
        store::MemoryStore::purge(options).await
    }

    pub async fn close(self) -> Result<()> {
        match self.backend {
            Backend::Local(store) => store.close().await,
            Backend::Remote(remote) => remote.session.close().await,
        }
    }

    pub(crate) async fn fixture_commit_malformed_state(&self, key: &str) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.fixture_commit_malformed_state(key).await,
            Backend::Remote(_) => bail!("fixture mutation requires a local memory store"),
        }
    }
}

#[derive(Debug)]
pub struct Candidate {
    backend: CandidateBackend,
}

#[derive(Debug)]
enum CandidateBackend {
    Local(store::Candidate),
    Remote(RemoteCandidate),
}

struct RemoteCandidate {
    view: RemoteView,
    handle: Uuid,
    base: String,
}

impl std::fmt::Debug for RemoteCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteCandidate")
            .field("handle", &self.handle)
            .field("base", &self.base)
            .finish()
    }
}

impl Candidate {
    async fn remote_transition(
        candidate: &RemoteCandidate,
        kind: CandidateTransitionKind,
    ) -> Result<ServiceValue> {
        let session = &candidate.view.session;
        // Ordinary candidate calls take this attachment before the shared
        // mutation lock. Keep that order while capturing and sending one
        // exact transition, or a concurrent candidate write could deadlock.
        let mut attachment = candidate.view.attachment.lock().await;
        let _mutation = session.mutations.lock().await;
        candidate.view.ensure_writable()?;
        let target = match candidate
            .view
            .checked_call_locked(
                &mut attachment,
                ServiceCall::View {
                    candidate: Some(candidate.handle),
                    operation: ViewOperation::Revision,
                },
            )
            .await?
        {
            ServiceValue::Revision(revision) => revision,
            _ => bail!("memory service returned the wrong candidate revision response"),
        };
        let branch = candidate.view.pinned_view.clone();
        let base = candidate.base.clone();
        let call = match kind {
            CandidateTransitionKind::Promote => ServiceCall::PromoteCandidate {
                handle: candidate.handle,
                branch,
                base,
                target,
            },
            CandidateTransitionKind::Abandon => ServiceCall::AbandonCandidate {
                handle: candidate.handle,
                branch,
                base,
                target,
            },
        };
        candidate
            .view
            .checked_call_locked(&mut attachment, call)
            .await
    }

    pub fn view(&self) -> MemoryStore {
        match &self.backend {
            CandidateBackend::Local(candidate) => MemoryStore {
                backend: Backend::Local(candidate.view()),
            },
            CandidateBackend::Remote(candidate) => MemoryStore {
                backend: Backend::Remote(candidate.view.clone()),
            },
        }
    }

    pub fn base(&self) -> &str {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.base(),
            CandidateBackend::Remote(candidate) => &candidate.base,
        }
    }

    pub async fn promote(&self) -> Result<String> {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.promote().await,
            CandidateBackend::Remote(candidate) => {
                match Self::remote_transition(candidate, CandidateTransitionKind::Promote).await? {
                    ServiceValue::Revision(revision) => Ok(revision),
                    _ => bail!("memory service returned the wrong promotion response"),
                }
            }
        }
    }

    pub async fn abandon(&self) -> Result<()> {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.abandon().await,
            CandidateBackend::Remote(candidate) => {
                unit(Self::remote_transition(candidate, CandidateTransitionKind::Abandon).await?)
            }
        }
    }
}

#[derive(Clone)]
pub struct UsageLedger {
    backend: LedgerBackend,
}

#[derive(Clone)]
enum LedgerBackend {
    Local(store::UsageLedger),
    Remote(RemoteView),
}

impl std::fmt::Debug for UsageLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UsageLedger")
            .field(
                "backend",
                &match self.backend {
                    LedgerBackend::Local(_) => "local",
                    LedgerBackend::Remote(_) => "service",
                },
            )
            .finish()
    }
}

impl UsageLedger {
    async fn call(&self, operation: LedgerOperation, writable: bool) -> Result<ServiceValue> {
        let LedgerBackend::Remote(remote) = &self.backend else {
            bail!("local usage ledger call was routed remotely")
        };
        if writable {
            remote.ensure_writable()?;
        }
        remote
            .call_raw(ServiceCall::Ledger {
                operation: Box::new(operation),
            })
            .await
    }

    pub async fn mark_new_session(&self, session_id: &str) -> Result<()> {
        match &self.backend {
            LedgerBackend::Local(ledger) => ledger.mark_new_session(session_id).await,
            LedgerBackend::Remote(_) => unit(
                self.call(
                    LedgerOperation::MarkNewSession {
                        session_id: session_id.into(),
                    },
                    true,
                )
                .await?,
            ),
        }
    }

    pub async fn admit(&self, start: InvocationStart) -> Result<()> {
        match &self.backend {
            LedgerBackend::Local(ledger) => ledger.admit(start).await,
            LedgerBackend::Remote(_) => unit(
                self.call(
                    LedgerOperation::Admit {
                        start: Box::new(start),
                    },
                    true,
                )
                .await?,
            ),
        }
    }

    pub async fn observe(&self, invocation_id: &str, observation: UsageObservation) -> Result<()> {
        match &self.backend {
            LedgerBackend::Local(ledger) => ledger.observe(invocation_id, observation).await,
            LedgerBackend::Remote(_) => unit(
                self.call(
                    LedgerOperation::Observe {
                        invocation_id: invocation_id.into(),
                        observation,
                    },
                    true,
                )
                .await?,
            ),
        }
    }

    pub async fn settle(&self, invocation_id: &str, outcome: InvocationOutcome) -> Result<()> {
        match &self.backend {
            LedgerBackend::Local(ledger) => ledger.settle(invocation_id, outcome).await,
            LedgerBackend::Remote(_) => unit(
                self.call(
                    LedgerOperation::Settle {
                        invocation_id: invocation_id.into(),
                        outcome,
                    },
                    true,
                )
                .await?,
            ),
        }
    }

    pub async fn session(&self, session_id: &str) -> Result<SessionUsage> {
        match &self.backend {
            LedgerBackend::Local(ledger) => ledger.session(session_id).await,
            LedgerBackend::Remote(_) => match self
                .call(
                    LedgerOperation::Session {
                        session_id: session_id.into(),
                    },
                    false,
                )
                .await?
            {
                ServiceValue::SessionUsage(usage) => Ok(usage),
                _ => bail!("memory service returned the wrong usage response"),
            },
        }
    }
}

#[derive(Clone)]
pub struct ActiveExportSnapshot {
    backend: ExportBackend,
}

#[derive(Clone)]
enum ExportBackend {
    Local(store::ActiveExportSnapshot),
    Remote(RemoteExport),
}

#[derive(Clone)]
struct RemoteExport {
    view: RemoteView,
    handle: Uuid,
    provenance: ExportProvenance,
}

impl std::fmt::Debug for ActiveExportSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActiveExportSnapshot")
            .field("provenance", self.provenance())
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExportCursor {
    inner: CursorBackend,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum CursorBackend {
    Local(store::ExportCursor),
    Remote {
        handle: Uuid,
        cursor: store::ExportCursor,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ExportPage {
    pub records: Vec<StorageRecord>,
    pub next: Option<ExportCursor>,
}

impl ActiveExportSnapshot {
    pub fn provenance(&self) -> &ExportProvenance {
        match &self.backend {
            ExportBackend::Local(snapshot) => snapshot.provenance(),
            ExportBackend::Remote(snapshot) => &snapshot.provenance,
        }
    }

    pub async fn page(&self, cursor: Option<ExportCursor>) -> Result<ExportPage> {
        match &self.backend {
            ExportBackend::Local(snapshot) => {
                let cursor = match cursor {
                    None => None,
                    Some(ExportCursor {
                        inner: CursorBackend::Local(cursor),
                    }) => Some(cursor),
                    Some(ExportCursor {
                        inner: CursorBackend::Remote { .. },
                    }) => bail!("export cursor belongs to another backend"),
                };
                let page = snapshot.page(cursor).await?;
                Ok(ExportPage {
                    records: page.records,
                    next: page.next.map(|cursor| ExportCursor {
                        inner: CursorBackend::Local(cursor),
                    }),
                })
            }
            ExportBackend::Remote(snapshot) => {
                let cursor = match cursor {
                    None => None,
                    Some(ExportCursor {
                        inner: CursorBackend::Remote { handle, cursor },
                    }) if handle == snapshot.handle => Some(cursor),
                    Some(_) => bail!("export cursor belongs to a different snapshot"),
                };
                let page = match snapshot
                    .view
                    .call_raw(ServiceCall::ExportPage {
                        handle: snapshot.handle,
                        cursor,
                    })
                    .await?
                {
                    ServiceValue::ExportPage(page) => page,
                    _ => bail!("memory service returned the wrong export-page response"),
                };
                Ok(ExportPage {
                    records: page.records,
                    next: page.next.map(|cursor| ExportCursor {
                        inner: CursorBackend::Remote {
                            handle: snapshot.handle,
                            cursor,
                        },
                    }),
                })
            }
        }
    }

    pub fn verify_counts(&self, message_count: u64, state_count: u64) -> Result<()> {
        match &self.backend {
            ExportBackend::Local(snapshot) => snapshot.verify_counts(message_count, state_count),
            ExportBackend::Remote(snapshot) => {
                snapshot.view.session.ensure_open()?;
                ensure!(
                    message_count == snapshot.provenance.message_count
                        && state_count == snapshot.provenance.state_count,
                    "export records do not match captured committed counts"
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::time::Duration;

    struct AbortOnDrop(tokio::task::AbortHandle);

    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    #[tokio::test]
    async fn cancelling_after_accepted_unit_frame_fences_clones_until_indexed_proof() -> Result<()>
    {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
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
            let executable = std::env::current_exe()?;
            let open = || {
                MemoryStore::open_managed_observed(
                    options.clone(),
                    project.clone(),
                    executable.clone(),
                )
                .1
            };
            let memory = open().await?;
            let sibling = open().await?;
            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed cancellation fixture did not attach to the service")
            };
            let pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(pause.clone());
            let writer = tokio::spawn({
                let memory = memory.clone();
                async move { memory.put("accepted-lost-reply", &json!(1)).await }
            });
            let _writer_cleanup = AbortOnDrop(writer.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                .await
                .context("accepted write frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if sibling.get("accepted-lost-reply").await? == Some(json!(1)) {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit the paused write")??;
            writer.abort();
            let stopped = tokio::time::timeout(Duration::from_secs(5), writer)
                .await
                .context("cancelled write future did not end")?;
            ensure!(
                stopped.is_err_and(|error| error.is_cancelled()),
                "accepted write future completed instead of being cancelled"
            );
            ensure!(memory.clone().put("blocked", &json!(true)).await.is_err());
            sibling.put("later-sibling", &json!(2)).await?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    match memory.reconcile().await {
                        Ok(Some(true)) => break Ok::<(), anyhow::Error>(()),
                        Err(error) if error.to_string().contains("remains uncertain") => {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        other => {
                            bail!("accepted cancelled write had unexpected outcome: {other:?}")
                        }
                    }
                }
            })
            .await
            .context("cancelled write indexed-outcome deadline")??;
            ensure!(memory.get("accepted-lost-reply").await? == Some(json!(1)));
            memory.put("after-proof", &json!(3)).await?;
            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("cancelled write fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("accepted cancelled write fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_candidate_unit_reply_reattaches_before_read_write_and_promotion() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            for restart_owner in [false, true] {
                let root = crate::test_support::tempdir()?;
                let project = root.path().join("project");
                std::fs::create_dir(&project)?;
                let project = project.canonicalize()?;
                let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
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
                let mut served = tokio::spawn(owner.serve());
                let executable = std::env::current_exe()?;
                let memory = MemoryStore::open_managed_observed(
                    options.clone(),
                    project.clone(),
                    executable.clone(),
                )
                .1
                .await?;
                let candidate = memory.begin_candidate("accepted private write").await?;
                let private = candidate.view();
                let Backend::Remote(remote) = &private.backend else {
                    bail!("candidate unit fixture did not attach to the service")
                };
                let creation_id = remote
                    .candidate_creation_id
                    .context("candidate creation identity was not retained")?;
                let generation = remote.attachment.lock().await.generation().to_owned();
                let pause = Arc::new(service::rpc::ReplyPause::default());
                remote
                    .attachment
                    .lock()
                    .await
                    .pause_after_next_send(pause.clone());
                let writer = tokio::spawn({
                    let private = private.clone();
                    async move { private.put("accepted-private", &json!(1)).await }
                });
                let _writer_cleanup = AbortOnDrop(writer.abort_handle());
                tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                    .await
                    .context("candidate unit frame was not flushed")?;

                // A separate checked attachment proves the native owner committed
                // the private write before cancellation drops its actual reply.
                let mut witness = service::attach_or_start(&options, &project, &executable).await?;
                let ServiceValue::CandidateOutcome(service::rpc::CandidateCreationOutcome::Open {
                    handle,
                    branch,
                    ..
                }) = witness
                    .call(ServiceCall::CandidateOutcome {
                        original_id: creation_id,
                        original_generation: generation,
                    })
                    .await?
                else {
                    bail!("witness could not open the exact candidate ref")
                };
                ensure!(branch == remote.pinned_view);
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let value = witness
                            .call(ServiceCall::View {
                                candidate: Some(handle),
                                operation: ViewOperation::Get {
                                    key: "accepted-private".into(),
                                },
                            })
                            .await?;
                        if matches!(value, ServiceValue::StoredValue(Some(ref stored)) if stored == &json!(1)) {
                            break Ok::<(), anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .context("owner did not commit the paused candidate write")??;
                witness.close();
                writer.abort();
                let stopped = tokio::time::timeout(Duration::from_secs(5), writer)
                    .await
                    .context("cancelled candidate writer did not end")?;
                ensure!(stopped.is_err_and(|error| error.is_cancelled()));
                ensure!(memory.put("fenced", &json!(true)).await.is_err());
                ensure!(
                    private
                        .reconcile()
                        .await
                        .unwrap_err()
                        .to_string()
                        .contains("typed candidate recovery")
                );
                if restart_owner {
                    // The logical client keeps its pending receipt and fence, but
                    // releases old transports so the owner can reap before a
                    // successor performs the read-only proof and reattachment.
                    let Backend::Remote(main_remote) = &memory.backend else {
                        bail!("candidate fixture lost its managed main view")
                    };
                    main_remote.attachment.lock().await.close();
                    remote.attachment.lock().await.close();
                    let permit = tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            match service::acquire_maintenance_permit(&options).await {
                                Ok(permit) => break Ok::<_, anyhow::Error>(permit),
                                Err(error)
                                    if error
                                        .to_string()
                                        .contains("memory service has active clients") =>
                                {
                                    tokio::time::sleep(Duration::from_millis(20)).await;
                                }
                                Err(error) => break Err(error),
                            }
                        }
                    })
                    .await
                    .context("closed candidate transports did not drain before owner retirement")??;
                    tokio::time::timeout(Duration::from_secs(10), &mut served)
                        .await
                        .context("candidate unit old owner did not reap")???;
                    drop(permit);
                    let successor = service::ServiceOwner::open(options.clone(), &project).await?;
                    served = tokio::spawn(successor.serve());
                }
                let recovered = private
                    .recover_candidate_unit()
                    .await?
                    .context("settled private unit write had no candidate recovery")?;
                ensure!(recovered.committed);
                ensure!(private.get("accepted-private").await.is_err());
                let candidate = recovered.candidate;
                ensure!(candidate.view().get("accepted-private").await? == Some(json!(1)));
                candidate.view().put("after-proof", &json!(2)).await?;
                candidate.promote().await?;
                let observer = if restart_owner {
                    ensure!(memory.get("accepted-private").await.is_err());
                    MemoryStore::open_managed_observed(
                        options.clone(),
                        project.clone(),
                        executable.clone(),
                    )
                    .1
                    .await?
                } else {
                    memory.clone()
                };
                ensure!(observer.get("accepted-private").await? == Some(json!(1)));
                ensure!(observer.get("after-proof").await? == Some(json!(2)));
                observer.close().await?;
                if restart_owner {
                    // The recovered candidate owns a new logical session; the
                    // old main close below cannot release its attachment.
                    candidate.view().close().await?;
                }
                memory.close().await?;
                let permit = service::acquire_maintenance_permit(&options)
                    .await
                    .context("candidate unit fixture final owner remained busy")?;
                tokio::time::timeout(Duration::from_secs(10), served)
                    .await
                    .context("candidate unit fixture owner did not reap")???;
                drop(permit);
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("candidate unit recovery fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_promotion_recovery_uses_exact_target_and_releases_clone_fence() -> Result<()>
    {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
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
            let (_, opening) = MemoryStore::open_managed_observed(
                options.clone(),
                project,
                std::env::current_exe()?,
            );
            let memory = opening.await?;
            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed promotion fixture did not attach to the service")
            };
            let candidate = memory.begin_candidate("lost promotion reply").await?;
            let CandidateBackend::Remote(ref remote_candidate) = candidate.backend else {
                bail!("managed promotion fixture did not return a remote candidate")
            };
            candidate.view().put("private", &json!(1)).await?;
            let target = candidate.view().revision().await?;
            let branch = remote_candidate.view.pinned_view.clone();
            let base = candidate.base().to_owned();
            let generation = remote_candidate
                .view
                .attachment
                .lock()
                .await
                .generation()
                .to_owned();
            let creation_id = remote_candidate
                .view
                .candidate_creation_id
                .context("candidate fixture lost its creation ID")?;
            ensure!(candidate.promote().await? == target);
            memory.put("later-main", &json!(true)).await?;
            // The typed IPC fixture checks exact refs. Here the owner has
            // settled promotion, but the logical client models losing its
            // reply before it could release the shared mutation fence.
            *remote.session.pending_transition.lock().unwrap() = Some(PendingTransition {
                id: Uuid::new_v4(),
                generation: generation.clone(),
                kind: CandidateTransitionKind::Promote,
                branch: branch.clone(),
                base: base.clone(),
                target: target.clone(),
                creation_id,
            });
            remote
                .session
                .uncertain_write
                .store(true, Ordering::Release);
            ensure!(memory.clone().put("blocked", &json!(true)).await.is_err());
            ensure!(
                memory
                    .recover_candidate_transition()
                    .await?
                    .is_some_and(|recovery| {
                        recovery.resolution
                            == CandidateTransitionResolution::Promoted(target.clone())
                            && recovery.candidate.is_none()
                    })
            );
            // An inverse transition against this already-promoted ref is a
            // resolved conflict, not an open candidate requiring reattach.
            *remote.session.pending_transition.lock().unwrap() = Some(PendingTransition {
                id: Uuid::new_v4(),
                generation,
                kind: CandidateTransitionKind::Abandon,
                branch,
                base,
                target,
                creation_id,
            });
            remote
                .session
                .uncertain_write
                .store(true, Ordering::Release);
            let inverse = memory
                .recover_candidate_transition()
                .await?
                .context("missing inverse transition outcome")?;
            ensure!(inverse.resolution == CandidateTransitionResolution::PreservedConflict);
            ensure!(inverse.candidate.is_none());
            memory.put("after-proof", &json!(true)).await?;
            memory.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("promotion facade fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("candidate promotion facade fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn preserved_candidate_conflict_reattaches_before_releasing_mutation_fence() -> Result<()>
    {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
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
            let (_, opening) = MemoryStore::open_managed_observed(
                options.clone(),
                project,
                std::env::current_exe()?,
            );
            let memory = opening.await?;
            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed conflict fixture did not attach to the service")
            };
            let candidate = memory.begin_candidate("preserved conflict").await?;
            let CandidateBackend::Remote(ref original) = candidate.backend else {
                bail!("managed conflict fixture did not return a remote candidate")
            };
            candidate.view().put("private", &json!(1)).await?;
            let target = candidate.view().revision().await?;
            memory.put("sibling", &json!(true)).await?;
            let id = Uuid::new_v4();
            let mut attachment = original.view.attachment.lock().await;
            let generation = attachment.generation().to_owned();
            ensure!(
                attachment
                    .call_with_id(
                        id,
                        ServiceCall::PromoteCandidate {
                            handle: original.handle,
                            branch: original.view.pinned_view.clone(),
                            base: original.base.clone(),
                            target: target.clone(),
                        }
                    )
                    .await
                    .is_err()
            );
            // Model a lost rejection reply after the owner recorded that this
            // request completed; the original candidate connection is gone.
            attachment.close();
            drop(attachment);
            *remote.session.pending_transition.lock().unwrap() = Some(PendingTransition {
                id,
                generation,
                kind: CandidateTransitionKind::Promote,
                branch: original.view.pinned_view.clone(),
                base: original.base.clone(),
                target,
                creation_id: original
                    .view
                    .candidate_creation_id
                    .context("conflict fixture lost candidate creation identity")?,
            });
            remote
                .session
                .uncertain_write
                .store(true, Ordering::Release);
            ensure!(memory.clone().put("blocked", &json!(true)).await.is_err());
            let recovered = memory
                .recover_candidate_transition()
                .await?
                .context("missing pending candidate transition")?;
            ensure!(recovered.resolution == CandidateTransitionResolution::OpenConflict);
            let candidate = recovered
                .candidate
                .context("preserved ref had no checked handle")?;
            ensure!(candidate.view().get("private").await? == Some(json!(1)));
            candidate.abandon().await?;
            memory.put("after-proof", &json!(true)).await?;
            memory.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("candidate conflict fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("candidate conflict reattachment fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn usage_reply_recovery_fences_clones_until_natural_key_is_proven() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
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
            let (_, opening) = MemoryStore::open_managed_observed(
                options.clone(),
                project,
                std::env::current_exe()?,
            );
            let memory = opening.await?;
            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed usage fixture did not attach to the service")
            };
            let ledger = memory.usage_ledger()?;
            ledger.mark_new_session("lost-usage-session").await?;
            let generation = remote.attachment.lock().await.generation().to_owned();
            // Model an accepted ledger mutation whose reply was lost after the
            // owner committed it. The direct IPC fixture covers that transport
            // boundary; this one proves the shared facade fence and release.
            *remote.session.pending_ledger.lock().unwrap() = Some(PendingLedger {
                id: Uuid::new_v4(),
                generation,
                proof: store::UsageProof::new_session("lost-usage-session"),
            });
            remote
                .session
                .uncertain_write
                .store(true, Ordering::Release);
            ensure!(ledger.clone().mark_new_session("blocked").await.is_err());
            ensure!(memory.clone().put("blocked", &json!(true)).await.is_err());
            ensure!(memory.reconcile().await? == Some(true));
            ledger.mark_new_session("after-proof").await?;
            ensure!(ledger.session("after-proof").await?.historical_complete);
            memory.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("usage facade fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("usage facade recovery fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_begin_recovery_keeps_clones_fenced_until_exact_ref_reattaches() -> Result<()>
    {
        tokio::time::timeout(Duration::from_secs(90), async {
            for restart_owner in [false, true] {
                let root = crate::test_support::tempdir()?;
                let project = root.path().join("project");
                std::fs::create_dir(&project)?;
                let project = project.canonicalize()?;
                let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
                let scope = format!(
                    "project/{}",
                    digest
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                );
                let options =
                    crate::test_support::open_options(root.path().join("private"), scope)?;
                let _gate = crate::spawn_gate::spawning().await;
                let owner = service::ServiceOwner::open(options.clone(), &project).await?;
                let mut served = tokio::spawn(owner.serve());
                let (_, opening) = MemoryStore::open_managed_observed(
                    options.clone(),
                    project.clone(),
                    std::env::current_exe()?,
                );
                let memory = opening.await?;
                let Backend::Remote(remote) = &memory.backend else {
                    bail!("managed candidate fixture did not attach to the service")
                };
                let dedicated = remote.fork().await?;
                let id = Uuid::new_v4();
                let generation;
                {
                    let mut attachment = dedicated.attachment.lock().await;
                    generation = attachment.generation().to_owned();
                    ensure!(matches!(
                        attachment
                            .call_with_id(
                                id,
                                ServiceCall::BeginCandidate {
                                    label: "unread Begin reply".into(),
                                },
                            )
                            .await?,
                        ServiceValue::CandidateStarted { .. }
                    ));
                    attachment.close();
                }
                drop(dedicated);
                // The native service accepted Begin, but this logical caller lost
                // the reply before it could retain the generation-local handle.
                *remote.session.pending_candidate.lock().unwrap() =
                    Some(PendingCandidate { id, generation });
                remote
                    .session
                    .uncertain_write
                    .store(true, Ordering::Release);
                ensure!(
                    memory
                        .clone()
                        .put("blocked", &json!(true))
                        .await
                        .unwrap_err()
                        .to_string()
                        .contains("outcome is uncertain")
                );
                ensure!(memory.reconcile().await.is_err());
                if restart_owner {
                    // Keep the caller's pending UUID and fence, but release its
                    // old transport so the owner can retire and reap Dolt. The
                    // successor must reattach the ref without replaying Begin.
                    remote.attachment.lock().await.close();
                    let permit = service::acquire_maintenance_permit(&options).await?;
                    tokio::time::timeout(Duration::from_secs(10), &mut served)
                        .await
                        .context("candidate fixture old owner did not reap")???;
                    drop(permit);
                    let successor = service::ServiceOwner::open(options.clone(), &project).await?;
                    served = tokio::spawn(successor.serve());
                }
                let candidate = memory
                    .recover_candidate_begin()
                    .await
                    .with_context(|| {
                        format!(
                            "recover exact candidate ref (restart_owner={restart_owner})"
                        )
                    })?
                    .with_context(|| {
                        format!(
                            "exact candidate ref was not recovered (restart_owner={restart_owner})"
                        )
                    })?;
                candidate
                    .view()
                    .put("private", &json!("retained"))
                    .await
                    .with_context(|| {
                        format!("write recovered candidate (restart_owner={restart_owner})")
                    })?;
                ensure!(
                    candidate
                        .view()
                        .get("private")
                        .await
                        .with_context(|| {
                            format!("read recovered candidate (restart_owner={restart_owner})")
                        })?
                        == Some(json!("retained")),
                    "recovered candidate returned the wrong value (restart_owner={restart_owner})"
                );
                if restart_owner {
                    ensure!(
                        memory
                            .clone()
                            .put("retired", &json!(true))
                            .await
                            .unwrap_err()
                            .to_string()
                            .contains("closed"),
                        "a retired-generation clone remained writable"
                    );
                } else {
                    memory
                        .put("unblocked", &json!(true))
                        .await
                        .with_context(|| {
                            format!(
                                "write main after candidate recovery (restart_owner={restart_owner})"
                            )
                        })?;
                }
                candidate
                    .abandon()
                    .await
                    .with_context(|| {
                        format!("abandon recovered candidate (restart_owner={restart_owner})")
                    })?;
                candidate
                    .view()
                    .close()
                    .await
                    .with_context(|| {
                        format!("close recovered candidate view (restart_owner={restart_owner})")
                    })?;
                memory
                    .close()
                    .await
                    .with_context(|| format!("close main view (restart_owner={restart_owner})"))?;
                let permit = service::acquire_maintenance_permit(&options).await?;
                tokio::time::timeout(Duration::from_secs(10), served)
                    .await
                    .context("candidate facade fixture owner did not reap")???;
                drop(permit);
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("candidate facade recovery fixture exceeded 90 seconds")??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_facade_preserves_views_ledger_export_and_independent_clients() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(90), async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
            let scope = format!(
                "project/{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let data = root.path().join("private");
            let mut options = OpenOptions::new(data, scope);
            options.config.cache_dir = Some(store::test_cache());
            options.config.offline = true;
            options.supervisor = Some(store::test_supervisor()?);
            let _gate = crate::spawn_gate::spawning().await;
            let owner = service::ServiceOwner::open(options.clone(), &project).await?;
            let served = tokio::spawn(owner.serve());
            let executable = std::env::current_exe()?;
            let (_, opening) = MemoryStore::open_managed_observed(
                options.clone(),
                project.clone(),
                executable.clone(),
            );
            let first = opening.await?;
            let Backend::Remote(remote) = &first.backend else {
                bail!("managed facade unexpectedly opened local storage")
            };
            let owner_generation = remote.attachment.lock().await.generation().to_owned();
            let mut competing_local = options.clone();
            competing_local.config.startup_timeout_secs = 1;
            let denied =
                tokio::time::timeout(Duration::from_secs(5), MemoryStore::open(competing_local))
                    .await
                    .context("direct local open did not resolve against the live owner")?;
            ensure!(
                denied.is_err(),
                "direct local open bypassed the owner's exclusive store lease"
            );
            ensure!(
                service::EndpointRecord::read(&options.data_dir, &options.project_scope)?
                    .is_some_and(|record| record.authority.service_generation == owner_generation),
                "losing local open changed the live owner publication"
            );
            let held = remote.attachment.lock().await;
            let concurrent_revision =
                tokio::time::timeout(Duration::from_secs(5), first.revision())
                    .await
                    .context("a busy client connection serialized an independent read")??;
            ensure!(!concurrent_revision.is_empty());
            drop(held);

            first.append("private/actor", "user", "first").await?;
            first.put_many(&[("shared".into(), json!(1))]).await?;
            let ledger = first.usage_ledger()?;
            ledger.mark_new_session("session-1").await?;
            ledger.session("session-1").await?;
            let (_, opening) =
                MemoryStore::open_managed_observed(options.clone(), project.clone(), executable);
            let second = opening.await?;
            ensure!(
                second.history("private/actor", 10).await?.len() == 1,
                "second client did not read the first client's committed history"
            );
            let candidate = second.begin_candidate("private-dream").await?;
            candidate.view().put("shared", &json!(2)).await?;
            ensure!(second.get("shared").await? == Some(json!(1)));
            candidate.promote().await?;
            ensure!(second.get("shared").await? == Some(json!(2)));

            let stale = second.begin_candidate("stale-dream").await?;
            stale.view().put("shared", &json!(3)).await?;
            second
                .append("private/actor", "assistant", "moved live base")
                .await?;
            let conflict = stale.promote().await.unwrap_err();
            ensure!(
                format!("{conflict:#}").contains("stale"),
                "candidate conflict lost its typed meaning: {conflict:#}"
            );
            ensure!(stale.view().get("shared").await? == Some(json!(3)));
            stale.abandon().await?;

            let export = second.begin_active_export().await?;
            let mut cursor = None;
            let mut messages = 0;
            let mut state = 0;
            loop {
                let page = export.page(cursor).await?;
                for record in page.records {
                    match record {
                        StorageRecord::Message { .. } => messages += 1,
                        StorageRecord::State { .. } => state += 1,
                    }
                }
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            export.verify_counts(messages, state)?;
            drop(export);
            // A connection explicitly closed before transmission is a known
            // pre-write loss. Reattach and send normally; only an incomplete
            // request/reply after dispatch must fence the logical session.
            remote.attachment.lock().await.close();
            first
                .append("private/actor", "user", "after safe reconnect")
                .await?;
            ensure!(first.history("private/actor", 10).await?.len() == 3);
            first.clone().close().await?;
            ensure!(
                first
                    .revision()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("closed"),
                "closing one client's clone left its original handle usable"
            );
            ensure!(second.history("private/actor", 10).await?.len() == 3);
            second.close().await?;
            ensure!(
                candidate
                    .view()
                    .revision()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("closed"),
                "closing the second client left its candidate handle usable"
            );
            let permit = tokio::time::timeout(
                Duration::from_secs(20),
                service::acquire_maintenance_permit(&options),
            )
            .await
            .context("managed owner did not retire after clients closed")??;
            tokio::time::timeout(Duration::from_secs(5), served)
                .await
                .context("managed owner did not finish reaping")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("managed facade fixture exceeded 90 seconds")??;
        Ok(())
    }
}
