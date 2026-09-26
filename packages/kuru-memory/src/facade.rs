//! Public storage handles. SQL, branch pools and Dolt lifetime stay private to
//! the local owner; a managed caller holds only generation-bound attachments.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{
    InvocationOutcome, InvocationStart, Message, Mode, SessionUsage, UsageObservation,
};
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
    #[cfg(any(test, feature = "test-support"))]
    reject_next_state_write: Arc<AtomicBool>,
}

pub type MemoryView = MemoryStore;

// Leave room under the owner's 32-attachment ceiling for other local clients,
// candidates and exports while allowing concurrent reads within one runtime.
const MAX_PARALLEL_CLIENT_CONNECTIONS: usize = 16;
const DREAM_LEASE_WAIT: Duration = Duration::from_secs(660);
const DREAM_LEASE_POLL: Duration = Duration::from_millis(100);

#[derive(Clone)]
enum Backend {
    Local(store::MemoryStore),
    Remote(RemoteView),
}

/// Exclusive project dream ownership. This guard holds no SQL transaction,
/// pool connection or ordinary mutation lock while provider work runs.
pub struct DreamLease {
    _backend: DreamLeaseBackend,
}

enum DreamLeaseBackend {
    Local {
        _guard: tokio::sync::OwnedMutexGuard<()>,
    },
    Remote {
        _view: RemoteView,
    },
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
    checked_successor_rebind: AtomicBool,
    uncertain_write: AtomicBool,
    pending_unit: Mutex<Option<PendingUnit>>,
    pending_candidate: Mutex<Option<PendingCandidate>>,
    pending_ledger: Mutex<Option<PendingLedger>>,
    pending_transition: Mutex<Option<PendingTransition>>,
    pending_selected_abandon: Mutex<Option<PendingSelectedAbandon>>,
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

#[derive(Clone)]
struct PendingSelectedAbandon {
    id: Uuid,
    generation: String,
    branch: String,
    base: String,
    target: String,
}

/// Typed result of an explicit exact-ref abandonment whose reply was lost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectedAbandonResolution {
    Abandoned,
    OpenUnchanged,
    OpenConflict,
    PreservedConflict,
}

/// The exact selected abandonment remains in flight or cannot yet be proved.
/// Callers may poll the retained request identity, but must never resend it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedAbandonUncertain;

impl std::fmt::Display for SelectedAbandonUncertain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("selected candidate abandonment remains uncertain")
    }
}

impl std::error::Error for SelectedAbandonUncertain {}

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
            checked_successor_rebind: AtomicBool::new(false),
            uncertain_write: AtomicBool::new(false),
            pending_unit: Mutex::new(None),
            pending_candidate: Mutex::new(None),
            pending_ledger: Mutex::new(None),
            pending_transition: Mutex::new(None),
            pending_selected_abandon: Mutex::new(None),
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

    fn retire_after_checked_recovery(&self) {
        // Mark closed while the mutation guard is held so a queued call
        // cannot slip through before the old attachments are drained. The
        // terminal typed result is already known; retaining this capability
        // lets cancellation retry cleanup and main-view reattachment.
        self.checked_successor_rebind.store(true, Ordering::Release);
        self.closed.store(true, Ordering::Release);
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
        // A cancelled close must leave every attachment discoverable for a
        // later drain. Weak entries are pruned by register or session drop.
        let attachments = self
            .attachments
            .lock()
            .map_err(|_| anyhow::anyhow!("memory attachment registry is poisoned"))?
            .clone();
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
                    // retired owner generation; close the old session and
                    // require a checked fresh main view before continuation.
                    self.retire_after_checked_recovery();
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
        let Some(creation_id) = pending.candidate_creation_id else {
            return Ok(None);
        };
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
            self.retire_after_checked_recovery();
        }
        Ok(Some(CandidateUnitRecovery {
            committed,
            candidate: Candidate {
                backend: CandidateBackend::Remote(RemoteCandidate { view, handle, base }),
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: Arc::new(AtomicBool::new(false)),
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
                    self.retire_after_checked_recovery();
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
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: Arc::new(AtomicBool::new(false)),
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
            self.retire_after_checked_recovery();
        }
        Ok(Some(CandidateTransitionRecovery {
            resolution: resolved,
            candidate,
        }))
    }

    async fn recover_selected_candidate_abandon(
        &self,
    ) -> Result<Option<SelectedAbandonResolution>> {
        let _mutation = self.mutations.lock().await;
        self.ensure_open()?;
        let pending = self
            .pending_selected_abandon
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending selected abandonment is poisoned"))?
            .clone();
        let Some(pending) = pending else {
            return Ok(None);
        };
        let mut attachment =
            service::attach_or_start(&self.options, &self.project, &self.executable)
                .await
                .context("connect for selected candidate abandonment outcome")?;
        ensure!(
            attachment.store_instance() == self.factory.store_instance(),
            "memory store identity changed before selected abandonment outcome"
        );
        let current_generation = attachment.generation() == pending.generation;
        let response = attachment
            .call(ServiceCall::SelectedAbandonOutcome {
                original_id: pending.id,
                original_generation: pending.generation,
                branch: pending.branch,
                base: pending.base,
                target: pending.target,
            })
            .await?;
        let ServiceValue::CandidateTransitionOutcome(result) = response else {
            bail!("memory service returned the wrong selected abandonment outcome")
        };
        let resolution = match result {
            service::rpc::CandidateTransitionResult::Abandoned => {
                SelectedAbandonResolution::Abandoned
            }
            service::rpc::CandidateTransitionResult::OpenUnchanged => {
                SelectedAbandonResolution::OpenUnchanged
            }
            service::rpc::CandidateTransitionResult::OpenConflict => {
                SelectedAbandonResolution::OpenConflict
            }
            service::rpc::CandidateTransitionResult::PreservedConflict
            | service::rpc::CandidateTransitionResult::Promoted { .. } => {
                SelectedAbandonResolution::PreservedConflict
            }
            service::rpc::CandidateTransitionResult::InFlight
            | service::rpc::CandidateTransitionResult::StillUncertain => {
                return Err(SelectedAbandonUncertain.into());
            }
        };
        *self
            .pending_selected_abandon
            .lock()
            .map_err(|_| anyhow::anyhow!("memory pending selected abandonment is poisoned"))? =
            None;
        if current_generation {
            self.uncertain_write.store(false, Ordering::Release);
        } else {
            self.retire_after_checked_recovery();
        }
        Ok(Some(resolution))
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
            self.retire_after_checked_recovery();
        }
        Ok(Some(Candidate {
            backend: CandidateBackend::Remote(RemoteCandidate { view, handle, base }),
            #[cfg(any(test, feature = "test-support"))]
            reject_next_state_write: Arc::new(AtomicBool::new(false)),
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
        let selected_abandon = match &call {
            ServiceCall::AbandonCandidateRef {
                branch,
                base,
                target,
            } => Some((branch, base, target)),
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
        if let Some((branch, base, target)) = selected_abandon {
            ensure!(
                self.candidate.is_none(),
                "selected abandonment requires the main view"
            );
            *self.session.pending_selected_abandon.lock().map_err(|_| {
                anyhow::anyhow!("memory pending selected abandonment is poisoned")
            })? = Some(PendingSelectedAbandon {
                id: request_id,
                generation: attachment.generation().to_owned(),
                branch: branch.clone(),
                base: base.clone(),
                target: target.clone(),
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
                *self.session.pending_selected_abandon.lock().map_err(|_| {
                    anyhow::anyhow!("memory pending selected abandonment is poisoned")
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
            operation: Box::new(operation),
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

fn session_lifecycle_outcome(value: ServiceValue) -> Result<store::SessionLifecycleOutcome> {
    let ServiceValue::SessionLifecycleOutcome(outcome) = value else {
        bail!("memory service returned the wrong session lifecycle response")
    };
    store::validate_session_lifecycle_outcome(&outcome)?;
    Ok(outcome)
}

impl MemoryStore {
    /// One instance-scoped definite pre-send state-save refusal for fixtures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reject_next_state_write_for_test(&self) {
        self.reject_next_state_write.store(true, Ordering::Release);
    }

    #[cfg(any(test, feature = "test-support"))]
    fn check_state_write_fixture(&self) -> Result<()> {
        ensure!(
            !self.reject_next_state_write.swap(false, Ordering::AcqRel),
            "injected state-save refusal before request send"
        );
        Ok(())
    }

    pub fn ensure_project_scope(&self, scope: &str) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.ensure_project_scope(scope),
            Backend::Remote(remote) => {
                ensure!(
                    remote.session.options.project_scope == scope,
                    "memory view belongs to a different canonical project"
                );
                Ok(())
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn fixture_pause_next_service_reply(
        &self,
        barrier: &crate::test_support::ReplyBarrier,
    ) -> Result<()> {
        let Backend::Remote(remote) = &self.backend else {
            bail!("reply pause requires a managed memory view")
        };
        remote
            .attachment
            .lock()
            .await
            .pause_after_next_send(barrier.inner.clone());
        Ok(())
    }

    pub fn exists(data_dir: &Path, project_scope: &str) -> Result<bool> {
        store::MemoryStore::exists(data_dir, project_scope)
    }

    /// Existing direct local opens remain usable by isolated fixtures and
    /// explicit maintenance. The project store lock excludes a live owner.
    pub async fn open(options: OpenOptions) -> Result<Self> {
        Ok(Self {
            backend: Backend::Local(store::MemoryStore::open(options).await?),
            #[cfg(any(test, feature = "test-support"))]
            reject_next_state_write: Arc::new(AtomicBool::new(false)),
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
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: Arc::new(AtomicBool::new(false)),
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
                            #[cfg(any(test, feature = "test-support"))]
                            reject_next_state_write: Arc::new(AtomicBool::new(false)),
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
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: Arc::new(AtomicBool::new(false)),
            })
        };
        (progress, opening)
    }

    /// Build a fresh main view only after this exact session settled an
    /// operation against a verified successor owner. Repeated calls are safe:
    /// old clones remain closed and cannot consume another caller's recovery.
    /// Transport loss or ordinary close cannot grant this rebind.
    pub async fn reopen_after_checked_recovery(&self) -> Result<Option<Self>> {
        let Backend::Remote(remote) = &self.backend else {
            return Ok(None);
        };
        ensure!(
            remote.candidate.is_none() && remote.pinned_view == "main",
            "only a main memory view can reattach after successor recovery"
        );
        let session = &remote.session;
        if !session.closed.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(
            session.checked_successor_rebind.load(Ordering::Acquire),
            "closed memory session has no settled successor recovery"
        );
        ensure!(
            !session.options.read_only,
            "read-only memory view cannot resume writable recovery"
        );
        // The close registry survives cancellation, so a later caller can
        // retry the drain before it attaches to the verified successor.
        session.close().await?;
        let attachment =
            service::attach_or_start(&session.options, &session.project, &session.executable)
                .await?;
        ensure!(
            attachment.store_instance() == session.factory.store_instance(),
            "memory store identity changed before successor reattachment"
        );
        let view = RemoteSession::new_view(
            attachment,
            session.options.clone(),
            session.project.clone(),
            session.executable.clone(),
        )?;
        Ok(Some(Self {
            backend: Backend::Remote(view),
            #[cfg(any(test, feature = "test-support"))]
            reject_next_state_write: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// Release one fixture transport while retaining its logical receipt and
    /// candidate identity for a successor-owner outcome query.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn close_transport_for_test(&self) -> Result<()> {
        let Backend::Remote(remote) = &self.backend else {
            bail!("transport fixture requires managed memory")
        };
        remote.attachment.lock().await.close();
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn temporary() -> Result<Self> {
        Ok(Self {
            backend: Backend::Local(store::MemoryStore::temporary().await?),
            reject_next_state_write: Arc::new(AtomicBool::new(false)),
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

    pub async fn append_session_message(
        &self,
        namespace: &str,
        session_id: &str,
        message: &Message,
    ) -> Result<()> {
        store::validate_session_message(namespace, session_id, message)?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .append_session_message(namespace, session_id, message)
                    .await
            }
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::AppendSessionMessage {
                            namespace: namespace.into(),
                            session_id: session_id.into(),
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

    pub async fn checkpoint_session_turn(
        &self,
        namespace: &str,
        session_id: &str,
        messages: &[Message],
        values: &[(String, Value)],
        public_turn: &store::SessionTurnCheckpoint,
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
        mode: &store::SessionModeCheckpoint,
    ) -> Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        self.check_state_write_fixture()?;
        self.checkpoint_session_inner(namespace, session_id, &[], values, None, Some(mode))
            .await
    }

    async fn checkpoint_session_inner(
        &self,
        namespace: &str,
        session_id: &str,
        messages: &[Message],
        values: &[(String, Value)],
        public_turn: Option<&store::SessionTurnCheckpoint>,
        mode: Option<&store::SessionModeCheckpoint>,
    ) -> Result<()> {
        store::validate_session_checkpoint(namespace, session_id, messages, values)?;
        if let Some(public_turn) = public_turn {
            store::validate_session_turn_checkpoint(session_id, messages, public_turn)?;
        }
        if let Some(mode) = mode {
            store::validate_session_mode_checkpoint(namespace, session_id, messages, values, mode)?;
        }
        ensure!(
            public_turn.is_none() || mode.is_none(),
            "session checkpoint cannot change mode and public turn together"
        );
        match &self.backend {
            Backend::Local(store) => match (public_turn, mode) {
                (Some(public_turn), None) => {
                    store
                        .checkpoint_session_turn(
                            namespace,
                            session_id,
                            messages,
                            values,
                            public_turn,
                        )
                        .await
                }
                (None, Some(mode)) => {
                    store
                        .checkpoint_session_mode(namespace, session_id, values, mode)
                        .await
                }
                (None, None) => {
                    store
                        .checkpoint_session(namespace, session_id, messages, values)
                        .await
                }
                (Some(_), Some(_)) => unreachable!("checked above"),
            },
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::CheckpointSession {
                            namespace: namespace.into(),
                            session_id: session_id.into(),
                            messages: messages.to_vec(),
                            values: values.to_vec(),
                            public_turn: public_turn.cloned(),
                            mode: mode.cloned(),
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

    pub async fn session_history_window(
        &self,
        namespace: &str,
        session_id: &str,
        limit: usize,
    ) -> Result<HistoryWindow> {
        store::validate_session_history_request(namespace, session_id, limit)?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .session_history_window(namespace, session_id, limit)
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::SessionHistoryWindow {
                    namespace: namespace.into(),
                    session_id: session_id.into(),
                    limit,
                })
                .await?
            {
                ServiceValue::HistoryWindow(window) => Ok(window),
                _ => bail!("memory service returned the wrong session-history response"),
            },
        }
    }

    pub async fn session_history_window_after(
        &self,
        namespace: &str,
        session_id: &str,
        after_exclusive: i64,
        limit: usize,
    ) -> Result<store::SessionHistoryWindowAfter> {
        store::validate_session_history_after_request(
            namespace,
            session_id,
            after_exclusive,
            limit,
        )?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .session_history_window_after(namespace, session_id, after_exclusive, limit)
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::SessionHistoryWindowAfter {
                    namespace: namespace.into(),
                    session_id: session_id.into(),
                    after_exclusive,
                    limit,
                })
                .await?
            {
                ServiceValue::SessionHistoryWindowAfter(window) => Ok(window),
                _ => bail!("memory service returned the wrong session cursor history response"),
            },
        }
    }

    pub async fn session_catalog_page(
        &self,
        lifecycle_state: Option<store::SessionLifecycleState>,
        cursor: Option<&store::SessionCatalogCursor>,
        expected_revision: Option<&str>,
        limit: usize,
    ) -> Result<store::SessionCatalogPage> {
        ensure!(
            limit <= store::MAX_SESSION_SOURCE_ROWS,
            "session catalog limit cannot exceed {}",
            store::MAX_SESSION_SOURCE_ROWS
        );
        if let Some(cursor) = cursor {
            store::validate_session_catalog_cursor(cursor)?;
        }
        if let Some(revision) = expected_revision {
            store::validate_revision_identity("session catalog expected revision", revision)?;
            ensure!(cursor.is_some(), "catalog revision requires a continuation");
        }
        match &self.backend {
            Backend::Local(store) => {
                store
                    .session_catalog_page(lifecycle_state, cursor, expected_revision, limit)
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::SessionCatalogPage {
                    lifecycle_state,
                    cursor: cursor.cloned(),
                    expected_revision: expected_revision.map(str::to_owned),
                    limit,
                })
                .await?
            {
                ServiceValue::SessionCatalogPage(page) => Ok(page),
                _ => bail!("memory service returned the wrong session catalog response"),
            },
        }
    }

    pub async fn session_catalog_record(
        &self,
        session_id: &str,
    ) -> Result<Option<store::SessionCatalogRecord>> {
        store::session_identity("session identity", session_id, 128)?;
        match &self.backend {
            Backend::Local(store) => store.session_catalog_record(session_id).await,
            Backend::Remote(remote) => match remote
                .call(ViewOperation::SessionCatalogRecord {
                    session_id: session_id.into(),
                })
                .await?
            {
                ServiceValue::SessionCatalogRecord(record) => {
                    if let Some(record) = &record {
                        store::validate_session_catalog(record)?;
                        ensure!(
                            record.session_id == session_id,
                            "memory service changed the requested session identity"
                        );
                    }
                    Ok(record)
                }
                _ => bail!("memory service returned the wrong session-catalog response"),
            },
        }
    }

    pub async fn create_session(
        &self,
        session_id: &str,
        mode: Mode,
        label: &str,
    ) -> Result<store::SessionLifecycleOutcome> {
        store::validate_session_lifecycle_input(session_id, None, Some(label))?;
        match &self.backend {
            Backend::Local(store) => store.create_session_catalog(session_id, mode, label).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                session_lifecycle_outcome(
                    remote
                        .call(ViewOperation::CreateSession {
                            session_id: session_id.into(),
                            mode,
                            label: label.into(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn rename_session(
        &self,
        session_id: &str,
        expected_generation: u64,
        label: &str,
    ) -> Result<store::SessionLifecycleOutcome> {
        store::validate_session_lifecycle_input(
            session_id,
            Some(expected_generation),
            Some(label),
        )?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .rename_session_catalog(session_id, expected_generation, label)
                    .await
            }
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                session_lifecycle_outcome(
                    remote
                        .call(ViewOperation::RenameSession {
                            session_id: session_id.into(),
                            expected_generation,
                            label: label.into(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn remove_session(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<store::SessionLifecycleOutcome> {
        store::validate_session_lifecycle_input(session_id, Some(expected_generation), None)?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .remove_session_catalog(session_id, expected_generation)
                    .await
            }
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                session_lifecycle_outcome(
                    remote
                        .call(ViewOperation::RemoveSession {
                            session_id: session_id.into(),
                            expected_generation,
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn restore_session(
        &self,
        session_id: &str,
        expected_generation: u64,
    ) -> Result<store::SessionLifecycleOutcome> {
        store::validate_session_lifecycle_input(session_id, Some(expected_generation), None)?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .restore_session_catalog(session_id, expected_generation)
                    .await
            }
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                session_lifecycle_outcome(
                    remote
                        .call(ViewOperation::RestoreSession {
                            session_id: session_id.into(),
                            expected_generation,
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn fork_session(
        &self,
        source_session_id: &str,
        expected_source_generation: u64,
        source_node_id: &str,
        child_session_id: &str,
        label: &str,
    ) -> Result<store::SessionLifecycleOutcome> {
        store::validate_session_fork_input(
            source_session_id,
            expected_source_generation,
            source_node_id,
            child_session_id,
            label,
        )?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .fork_session_catalog(
                        source_session_id,
                        expected_source_generation,
                        source_node_id,
                        child_session_id,
                        label,
                    )
                    .await
            }
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                session_lifecycle_outcome(
                    remote
                        .call(ViewOperation::ForkSession {
                            source_session_id: source_session_id.into(),
                            expected_source_generation,
                            source_node_id: source_node_id.into(),
                            child_session_id: child_session_id.into(),
                            label: label.into(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn public_transcript_page(
        &self,
        session_id: &str,
        cursor: Option<&store::PublicTranscriptCursor>,
        limit: usize,
    ) -> Result<store::PublicTranscriptPage> {
        store::session_identity("public transcript session", session_id, 128)?;
        ensure!(
            limit <= store::MAX_SESSION_SOURCE_ROWS,
            "public transcript limit cannot exceed {}",
            store::MAX_SESSION_SOURCE_ROWS
        );
        if let Some(cursor) = cursor {
            store::validate_public_transcript_cursor(cursor)?;
            ensure!(
                cursor.session_id == session_id,
                "public transcript continuation changed session"
            );
        }
        match &self.backend {
            Backend::Local(store) => {
                store
                    .public_transcript_page(session_id, cursor, limit)
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::PublicTranscriptPage {
                    session_id: session_id.into(),
                    cursor: cursor.cloned(),
                    limit,
                })
                .await?
            {
                ServiceValue::PublicTranscriptPage(page) => Ok(page),
                _ => bail!("memory service returned the wrong public transcript response"),
            },
        }
    }

    pub async fn acquire_dream_lease(&self) -> Result<DreamLease> {
        match &self.backend {
            Backend::Local(store) => {
                let guard = tokio::time::timeout(DREAM_LEASE_WAIT, store.acquire_dream_lease())
                    .await
                    .context("dream lease acquisition deadline exceeded")?;
                Ok(DreamLease {
                    _backend: DreamLeaseBackend::Local { _guard: guard },
                })
            }
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                tokio::time::timeout(DREAM_LEASE_WAIT, async {
                    let dedicated = remote.fork().await?;
                    loop {
                        match dedicated
                            .call_raw(ServiceCall::TryAcquireDreamLease)
                            .await?
                        {
                            ServiceValue::DreamLease { acquired: true } => {
                                return Ok(DreamLease {
                                    _backend: DreamLeaseBackend::Remote { _view: dedicated },
                                });
                            }
                            ServiceValue::DreamLease { acquired: false } => {
                                tokio::time::sleep(DREAM_LEASE_POLL).await;
                            }
                            _ => bail!("memory service returned the wrong dream-lease response"),
                        }
                    }
                })
                .await
                .context("dream lease acquisition deadline exceeded")?
            }
        }
    }

    pub async fn session_source_snapshot(
        &self,
        actor_namespace: &str,
        session_id: &str,
        source_namespace: &str,
        after_exclusive: i64,
        limit: usize,
    ) -> Result<store::SessionSourceSnapshot> {
        store::validate_session_source_request(
            actor_namespace,
            session_id,
            source_namespace,
            after_exclusive,
            limit,
        )?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .session_source_snapshot(
                        actor_namespace,
                        session_id,
                        source_namespace,
                        after_exclusive,
                        limit,
                    )
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::SessionSourceSnapshot {
                    actor_namespace: actor_namespace.into(),
                    session_id: session_id.into(),
                    source_namespace: source_namespace.into(),
                    after_exclusive,
                    limit,
                })
                .await?
            {
                ServiceValue::SessionSourceSnapshot(snapshot) => Ok(snapshot),
                _ => bail!("memory service returned the wrong session snapshot response"),
            },
        }
    }

    pub async fn checkpoint_context_summary(
        &self,
        checkpoint: &store::ContextSummaryCheckpoint,
    ) -> Result<()> {
        service::rpc::validate_context_summary_checkpoint_request(checkpoint)?;
        match &self.backend {
            Backend::Local(store) => store.checkpoint_context_summary(checkpoint).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call(ViewOperation::CheckpointContextSummary {
                            record: checkpoint.record.clone(),
                            private_reasoning: checkpoint.private_reasoning.clone(),
                        })
                        .await?,
                )
            }
        }
    }

    pub async fn context_summary_cursor(
        &self,
        actor_namespace: &str,
        session_id: &str,
        source_namespace: &str,
    ) -> Result<Option<store::ContextSummaryCursor>> {
        store::identifier("actor namespace", actor_namespace, 1024)?;
        store::identifier("session identity", session_id, 128)?;
        store::identifier("source namespace", source_namespace, 1024)?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .context_summary_cursor(actor_namespace, session_id, source_namespace)
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::ContextSummaryCursor {
                    actor_namespace: actor_namespace.into(),
                    session_id: session_id.into(),
                    source_namespace: source_namespace.into(),
                })
                .await?
            {
                ServiceValue::ContextSummaryCursor(cursor) => Ok(cursor),
                _ => bail!("memory service returned the wrong context cursor response"),
            },
        }
    }

    pub async fn context_summary_window(
        &self,
        actor_namespace: &str,
        summary_namespace: &str,
        session_id: Option<&str>,
        source_namespace: Option<&str>,
        limit: usize,
    ) -> Result<store::ContextSummaryWindow> {
        store::validate_context_summary_window_request(
            actor_namespace,
            summary_namespace,
            session_id,
            source_namespace,
            limit,
        )?;
        match &self.backend {
            Backend::Local(store) => {
                store
                    .context_summary_window(
                        actor_namespace,
                        summary_namespace,
                        session_id,
                        source_namespace,
                        limit,
                    )
                    .await
            }
            Backend::Remote(remote) => match remote
                .call(ViewOperation::ContextSummaryWindow {
                    actor_namespace: actor_namespace.into(),
                    summary_namespace: summary_namespace.into(),
                    session_id: session_id.map(str::to_owned),
                    source_namespace: source_namespace.map(str::to_owned),
                    limit,
                })
                .await?
            {
                ServiceValue::ContextSummaryWindow(window) => Ok(window),
                _ => bail!("memory service returned the wrong context-summary response"),
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
        #[cfg(any(test, feature = "test-support"))]
        self.check_state_write_fixture()?;
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

    /// Persist one settled provider completion's private reasoning summaries
    /// in one idempotent mutation. A conflicting settled identity is a typed,
    /// definite no-effect response; a transport/storage failure remains fenced
    /// by the ordinary remote-write reconciliation path.
    pub async fn put_reasoning_summaries(
        &self,
        records: &[store::ReasoningSummaryRecord],
    ) -> Result<()> {
        store::validate_reasoning_summaries(records)?;
        match &self.backend {
            Backend::Local(store) => store.put_reasoning_summaries(records).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call_raw(ServiceCall::PutReasoningSummaries {
                            records: records.to_vec(),
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
                    if remote
                        .session
                        .pending_selected_abandon
                        .lock()
                        .map_err(|_| {
                            anyhow::anyhow!("memory pending selected abandonment is poisoned")
                        })?
                        .is_some()
                    {
                        bail!("selected candidate abandonment requires its typed outcome recovery");
                    }
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
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: Arc::new(AtomicBool::new(
                    self.reject_next_state_write.swap(false, Ordering::AcqRel),
                )),
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
                    #[cfg(any(test, feature = "test-support"))]
                    reject_next_state_write: Arc::new(AtomicBool::new(
                        self.reject_next_state_write.swap(false, Ordering::AcqRel),
                    )),
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

    /// Inspect exact retained candidate refs without opening or changing them.
    pub async fn candidate_inventory(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<store::CandidateInventoryPage> {
        match &self.backend {
            Backend::Local(store) => store.candidate_inventory(after, limit).await,
            Backend::Remote(remote) => match remote
                .call_raw(ServiceCall::CandidateInventory {
                    after: after.map(str::to_owned),
                    limit,
                })
                .await?
            {
                ServiceValue::CandidateInventory(page) => Ok(page),
                _ => bail!("memory service returned the wrong candidate inventory response"),
            },
        }
    }

    /// Recheck one validated candidate branch without dispatching a mutation.
    pub async fn candidate_ref_status(&self, branch: &str) -> Result<store::CandidateRefStatus> {
        match &self.backend {
            Backend::Local(store) => store.candidate_ref_status(branch).await,
            Backend::Remote(remote) => match remote
                .call_raw(ServiceCall::CandidateRefStatus {
                    branch: branch.to_owned(),
                })
                .await?
            {
                ServiceValue::CandidateRefStatus(status) => Ok(status),
                _ => bail!("memory service returned the wrong candidate status response"),
            },
        }
    }

    /// Explicitly abandon only the inspected branch/base/head. If the reply
    /// is lost, query `recover_selected_candidate_abandon`; never resend it.
    pub async fn abandon_candidate_ref(&self, branch: &str, base: &str, head: &str) -> Result<()> {
        match &self.backend {
            Backend::Local(store) => store.abandon_candidate_ref(branch, base, head).await,
            Backend::Remote(remote) => {
                remote.ensure_writable()?;
                unit(
                    remote
                        .call_raw(ServiceCall::AbandonCandidateRef {
                            branch: branch.to_owned(),
                            base: base.to_owned(),
                            target: head.to_owned(),
                        })
                        .await?,
                )
            }
        }
    }

    /// Resolve a lost selected-abandon reply from the exact typed ref result.
    /// An open or conflicted result requires a fresh inspection before action.
    pub async fn recover_selected_candidate_abandon(
        &self,
    ) -> Result<Option<SelectedAbandonResolution>> {
        match &self.backend {
            Backend::Local(_) => Ok(None),
            Backend::Remote(remote) => remote.session.recover_selected_candidate_abandon().await,
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

#[derive(Clone, Debug)]
pub struct Candidate {
    backend: CandidateBackend,
    /// One test-only pre-send refusal belongs to this exact new candidate.
    #[cfg(any(test, feature = "test-support"))]
    reject_next_state_write: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
enum CandidateBackend {
    Local(store::Candidate),
    Remote(RemoteCandidate),
}

#[derive(Clone)]
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
        expected_target: Option<&str>,
    ) -> Result<ServiceValue> {
        let session = &candidate.view.session;
        // Ordinary candidate calls take this attachment before the shared
        // mutation lock. Keep that order while capturing and sending one
        // exact transition, or a concurrent candidate write could deadlock.
        let mut attachment = candidate.view.attachment.lock().await;
        let _mutation = session.mutations.lock().await;
        candidate.view.ensure_writable()?;
        let target = if let Some(expected) = expected_target {
            expected.to_owned()
        } else {
            match candidate
                .view
                .checked_call_locked(
                    &mut attachment,
                    ServiceCall::View {
                        candidate: Some(candidate.handle),
                        operation: Box::new(ViewOperation::Revision),
                    },
                )
                .await?
            {
                ServiceValue::Revision(revision) => revision,
                _ => bail!("memory service returned the wrong candidate revision response"),
            }
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
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: self.reject_next_state_write.clone(),
            },
            CandidateBackend::Remote(candidate) => MemoryStore {
                backend: Backend::Remote(candidate.view.clone()),
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: self.reject_next_state_write.clone(),
            },
        }
    }

    pub fn base(&self) -> &str {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.base(),
            CandidateBackend::Remote(candidate) => &candidate.base,
        }
    }

    /// The exact durable branch pinned by this candidate handle.
    pub fn branch(&self) -> &str {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.branch(),
            CandidateBackend::Remote(candidate) => &candidate.view.pinned_view,
        }
    }

    pub async fn promote(&self) -> Result<String> {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.promote().await,
            CandidateBackend::Remote(candidate) => {
                match Self::remote_transition(candidate, CandidateTransitionKind::Promote, None)
                    .await?
                {
                    ServiceValue::Revision(revision) => Ok(revision),
                    _ => bail!("memory service returned the wrong promotion response"),
                }
            }
        }
    }

    /// Promote only the exact staged head already captured by the caller.
    /// The owner rechecks the branch, base and target before transition.
    pub async fn promote_exact(&self, expected_target: &str) -> Result<String> {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.promote_exact(expected_target).await,
            CandidateBackend::Remote(candidate) => {
                match Self::remote_transition(
                    candidate,
                    CandidateTransitionKind::Promote,
                    Some(expected_target),
                )
                .await?
                {
                    ServiceValue::Revision(revision) => Ok(revision),
                    _ => bail!("memory service returned the wrong promotion response"),
                }
            }
        }
    }

    pub async fn abandon(&self) -> Result<()> {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.abandon().await,
            CandidateBackend::Remote(candidate) => unit(
                Self::remote_transition(candidate, CandidateTransitionKind::Abandon, None).await?,
            ),
        }
    }

    /// Abandon an attached candidate only if it still has the exact head the
    /// user inspected. The owner rechecks this target under its write lock.
    pub async fn abandon_exact(&self, expected_target: &str) -> Result<()> {
        match &self.backend {
            CandidateBackend::Local(candidate) => candidate.abandon_exact(expected_target).await,
            CandidateBackend::Remote(candidate) => unit(
                Self::remote_transition(
                    candidate,
                    CandidateTransitionKind::Abandon,
                    Some(expected_target),
                )
                .await?,
            ),
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

    pub fn verify_counts(
        &self,
        message_count: u64,
        state_count: u64,
        context_summary_count: u64,
        context_cursor_count: u64,
        session_catalog_count: u64,
        public_turn_count: u64,
    ) -> Result<()> {
        match &self.backend {
            ExportBackend::Local(snapshot) => snapshot.verify_counts(
                message_count,
                state_count,
                context_summary_count,
                context_cursor_count,
                session_catalog_count,
                public_turn_count,
            ),
            ExportBackend::Remote(snapshot) => {
                snapshot.view.session.ensure_open()?;
                ensure!(
                    message_count == snapshot.provenance.message_count
                        && state_count == snapshot.provenance.state_count
                        && context_summary_count == snapshot.provenance.context_summary_count
                        && context_cursor_count == snapshot.provenance.context_cursor_count
                        && session_catalog_count == snapshot.provenance.session_catalog_count
                        && public_turn_count == snapshot.provenance.public_turn_count,
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

    /// Outer hang backstop for a facade fixture, derived from the product
    /// budgets it exercises so that a real stall reports its own product error
    /// before this bound expires.
    ///
    /// The fixture's first Dolt start may wait behind another fixture's cold
    /// install of the shared test runtime cache; the provisioner bounds that
    /// wait by `LOCK_TIMEOUT`. Each real service lifecycle (a
    /// `ServiceOwner::open` or a local `MemoryStore::open` that starts Dolt;
    /// managed attaches and checked rebinds start none) then gets the startup
    /// budget for its store open, `QUERY_TIMEOUT` for the fixture's operations,
    /// and the startup budget again for `acquire_maintenance_permit`, whose
    /// election and retirement deadline covers the owner's Dolt reap. Every
    /// fixture's options keep the `OpenOptions::new` budgets;
    /// `test_support::open_options` changes only cache, offline mode and
    /// supervisor.
    fn fixture_deadline(lifecycles: u32) -> Duration {
        let config = OpenOptions::new(PathBuf::new(), String::new()).config;
        let startup = Duration::from_secs(config.startup_timeout_secs);
        let lifecycle = startup
            .saturating_mul(2)
            .saturating_add(store::QUERY_TIMEOUT);
        crate::provision::LOCK_TIMEOUT.saturating_add(lifecycle.saturating_mul(lifecycles))
    }

    async fn cancel_before_session_acceptance<F>(remote: &RemoteView, operation: F) -> Result<()>
    where
        F: std::future::Future<Output = Result<store::SessionLifecycleOutcome>>,
    {
        // Force the real facade future to reach the shared mutation gate,
        // then cancel it before any request can be sent to the owner.
        let held = remote.session.mutations.lock().await;
        let mut operation = Box::pin(operation);
        ensure!(
            futures::poll!(operation.as_mut()).is_pending(),
            "session lifecycle operation finished before its admission gate"
        );
        drop(operation);
        drop(held);
        Ok(())
    }

    #[tokio::test]
    async fn managed_public_transcript_pages_preserve_main_and_candidate_views() -> Result<()> {
        // Real service lifecycles: a local seed open, then one service owner.
        let deadline = fixture_deadline(2);
        tokio::time::timeout(deadline, async {
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
            let mut turns = Vec::with_capacity(1025);
            let mut predecessor = None;
            for index in 0..1025 {
                let turn_id = format!("turn-{index:04}");
                let turn = store::PublicTurnRecord {
                    node_id: store::public_turn_node_id("managed-session", &turn_id)?,
                    origin_session_id: "managed-session".into(),
                    turn_id,
                    kind: store::PublicTurnKind::Primary,
                    continuation_of_node_id: None,
                    predecessor_node_id: predecessor,
                    settlement: store::PublicTurnSettlement::Completed,
                    user_entry: Some(Message::text(
                        "user",
                        format!("managed question {index:04}"),
                    )),
                    speaker_id: Some("managed-speaker".into()),
                    terminal_entries: vec![Message::text(
                        "assistant",
                        format!("managed answer {index:04}"),
                    )],
                    record_format: store::PUBLIC_TURN_RECORD_FORMAT.into(),
                };
                predecessor = Some(turn.node_id.clone());
                turns.push(turn);
            }
            let turn = turns.last().context("managed transcript is empty")?.clone();
            let catalog = store::SessionCatalogRecord {
                session_id: "managed-session".into(),
                mode: kuru_core::Mode::Ifs,
                label: "managed label".into(),
                created_order: 1,
                updated_order: 1,
                lifecycle_generation: 0,
                lifecycle_state: store::SessionLifecycleState::Active,
                head_node_id: Some(turn.node_id.clone()),
                pending_node_id: None,
                legacy_prefix: None,
                fork_provenance: None,
                record_format: store::SESSION_CATALOG_RECORD_FORMAT.into(),
            };
            let seed = store::MemoryStore::open(options.clone()).await?;
            seed.fixture_insert_public_session(&catalog, &turns)
                .await?;
            seed.close().await?;

            let _gate = crate::spawn_gate::spawning().await;
            let owner = service::ServiceOwner::open(options.clone(), &project).await?;
            let served = tokio::spawn(owner.serve());
            let executable = std::env::current_exe()?;
            let memory = MemoryStore::open_managed_observed(
                options.clone(),
                project.clone(),
                executable,
            )
            .1
            .await?;
            let catalog_page = memory.session_catalog_page(None, None, None, 16).await?;
            ensure!(
                catalog_page.view == "main"
                    && catalog_page.records.as_slice() == [catalog.clone()],
                "managed catalog projection changed its pinned main coordinates"
            );
            let main_page = memory
                .public_transcript_page("managed-session", None, 128)
                .await?;
            ensure!(
                main_page.view == "main"
                    && main_page.total_rows == 1025
                    && matches!(main_page.records.first(), Some(store::PublicTranscriptEntry::Turn { record }) if record == &turn),
                "managed main transcript projection changed its newest public turn"
            );
            let revision = main_page.revision.clone();
            let mut cursor = main_page.next.clone();
            let mut rows = main_page.records.len();
            while let Some(current) = cursor {
                let page = memory
                    .public_transcript_page("managed-session", Some(&current), 128)
                    .await?;
                ensure!(
                    page.view == "main"
                        && page.revision == revision
                        && page.total_rows == 1025,
                    "managed transcript continuation changed its pinned coordinates"
                );
                rows += page.records.len();
                cursor = page.next;
            }
            ensure!(rows == 1025, "managed transcript paging omitted or repeated rows");

            let candidate = memory.begin_candidate("public transcript candidate").await?;
            let candidate_page = candidate
                .view()
                .public_transcript_page("managed-session", None, 16)
                .await?;
            ensure!(
                candidate_page.view == candidate.branch()
                    && candidate_page.total_rows == 1025
                    && matches!(candidate_page.records.first(), Some(store::PublicTranscriptEntry::Turn { record }) if record == &turn),
                "candidate transcript projection lost its pinned view or newest inherited turn"
            );
            ensure!(
                memory
                    .public_transcript_page("managed-session", None, 16)
                    .await?
                    .view
                    == "main",
                "candidate transcript view leaked into main"
            );
            candidate.abandon().await?;
            memory.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("public transcript fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("managed public transcript fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_session_lifecycle_is_reversible_receipted_and_candidate_isolated() -> Result<()>
    {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
                bail!("managed lifecycle fixture did not attach to the service")
            };

            let before_create = sibling.revision().await?;
            cancel_before_session_acceptance(
                remote,
                memory.create_session("managed-lifecycle", Mode::Jungian, "first label"),
            )
            .await?;
            ensure!(
                sibling.revision().await? == before_create
                    && sibling
                        .session_catalog_record("managed-lifecycle")
                        .await?
                        .is_none(),
                "pre-acceptance create cancellation published a session"
            );

            let create_pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(create_pause.clone());
            let create = tokio::spawn({
                let memory = memory.clone();
                async move {
                    memory
                        .create_session("managed-lifecycle", Mode::Jungian, "first label")
                        .await
                }
            });
            let _create_cleanup = AbortOnDrop(create.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), create_pause.sent.notified())
                .await
                .context("accepted create frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if sibling
                        .session_catalog_record("managed-lifecycle")
                        .await?
                        .is_some()
                    {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit paused create")??;
            create.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), create)
                    .await
                    .context("cancelled create did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted create completed instead of being cancelled"
            );
            ensure!(memory.reconcile().await? == Some(true));
            let created = memory
                .session_catalog_record("managed-lifecycle")
                .await?
                .context("accepted create lost its catalog row")?;
            ensure!(
                created.lifecycle_generation == 0
                    && created.lifecycle_state == store::SessionLifecycleState::Active,
                "managed create returned the wrong lifecycle coordinates"
            );

            let before_rename = sibling.revision().await?;
            cancel_before_session_acceptance(
                remote,
                memory.rename_session("managed-lifecycle", 0, "renamed"),
            )
            .await?;
            ensure!(
                sibling.revision().await? == before_rename,
                "pre-acceptance rename cancellation changed the catalog"
            );
            let pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(pause.clone());
            let rename = tokio::spawn({
                let memory = memory.clone();
                async move {
                    memory
                        .rename_session("managed-lifecycle", 0, "renamed")
                        .await
                }
            });
            let _rename_cleanup = AbortOnDrop(rename.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                .await
                .context("accepted lifecycle frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let page = sibling.session_catalog_page(None, None, None, 16).await?;
                    if matches!(page.records.as_slice(), [record] if record.label == "renamed" && record.lifecycle_generation == 1)
                    {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit the paused lifecycle write")??;
            // A sibling's stale request races the first client's held reply.
            // It must receive a definite typed refusal without changing the
            // accepted rename or consuming its retained receipt.
            let before_stale = sibling.revision().await?;
            let stale = sibling
                .rename_session("managed-lifecycle", 0, "stale")
                .await
                .unwrap_err();
            ensure!(
                stale
                    .downcast_ref::<store::SessionLifecycleRejected>()
                    .is_some_and(|rejected| {
                        rejected.0 == store::SessionLifecycleRefusal::GenerationChanged
                    }),
                "managed stale generation lost its typed refusal"
            );
            ensure!(
                sibling.revision().await? == before_stale,
                "definite lifecycle refusal changed the view"
            );
            rename.abort();
            let stopped = tokio::time::timeout(Duration::from_secs(5), rename)
                .await
                .context("cancelled lifecycle future did not end")?;
            ensure!(
                stopped.is_err_and(|error| error.is_cancelled()),
                "accepted lifecycle future completed instead of being cancelled"
            );
            ensure!(
                memory
                    .remove_session("managed-lifecycle", 1)
                    .await
                    .is_err(),
                "uncertain lifecycle receipt failed to fence later mutation"
            );
            ensure!(memory.reconcile().await? == Some(true));

            let before_remove = sibling.revision().await?;
            cancel_before_session_acceptance(
                remote,
                memory.remove_session("managed-lifecycle", 1),
            )
            .await?;
            ensure!(
                sibling.revision().await? == before_remove,
                "pre-acceptance remove cancellation changed the catalog"
            );
            let remove_pause = Arc::new(service::rpc::ReplyPause::default());
            {
                let mut attachment = remote.attachment.lock().await;
                if !attachment.has_complete_exchange() {
                    *attachment = remote.session.factory.connect().await?;
                }
                attachment.pause_after_next_send(remove_pause.clone());
            }
            let mut remove = tokio::spawn({
                let memory = memory.clone();
                async move { memory.remove_session("managed-lifecycle", 1).await }
            });
            let _remove_cleanup = AbortOnDrop(remove.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    _ = remove_pause.sent.notified() => Ok(()),
                    outcome = &mut remove => bail!("remove finished before paused reply: {outcome:?}"),
                }
            })
            .await
            .context("accepted remove frame was not flushed")??;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let record = sibling.session_catalog_record("managed-lifecycle").await?;
                    if record.as_ref().is_some_and(|record| {
                        record.lifecycle_state == store::SessionLifecycleState::Removed
                            && record.lifecycle_generation == 2
                    }) {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit paused remove")??;
            remove.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), remove)
                    .await
                    .context("cancelled remove did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted remove completed instead of being cancelled"
            );
            ensure!(memory.reconcile().await? == Some(true));
            let removed = memory
                .session_catalog_record("managed-lifecycle")
                .await?
                .context("accepted remove lost its catalog row")?;
            ensure!(
                removed.lifecycle_state == store::SessionLifecycleState::Removed
                    && removed.lifecycle_generation == 2,
                "managed remove returned the wrong retained state"
            );
            ensure!(
                memory
                    .session_catalog_page(
                        Some(store::SessionLifecycleState::Active),
                        None,
                        None,
                        16,
                    )
                    .await?
                    .records
                    .is_empty(),
                "removed session remained in the active catalog"
            );
            let before_restore = sibling.revision().await?;
            cancel_before_session_acceptance(
                remote,
                memory.restore_session("managed-lifecycle", 2),
            )
            .await?;
            ensure!(
                sibling.revision().await? == before_restore,
                "pre-acceptance restore cancellation changed the catalog"
            );
            let restore_pause = Arc::new(service::rpc::ReplyPause::default());
            {
                let mut attachment = remote.attachment.lock().await;
                if !attachment.has_complete_exchange() {
                    *attachment = remote.session.factory.connect().await?;
                }
                attachment.pause_after_next_send(restore_pause.clone());
            }
            let mut restore = tokio::spawn({
                let memory = memory.clone();
                async move { memory.restore_session("managed-lifecycle", 2).await }
            });
            let _restore_cleanup = AbortOnDrop(restore.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    _ = restore_pause.sent.notified() => Ok(()),
                    outcome = &mut restore => bail!("restore finished before paused reply: {outcome:?}"),
                }
            })
            .await
            .context("accepted restore frame was not flushed")??;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let record = sibling.session_catalog_record("managed-lifecycle").await?;
                    if record.as_ref().is_some_and(|record| {
                        record.lifecycle_state == store::SessionLifecycleState::Active
                            && record.lifecycle_generation == 3
                    }) {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit paused restore")??;
            restore.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), restore)
                    .await
                    .context("cancelled restore did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted restore completed instead of being cancelled"
            );
            ensure!(memory.reconcile().await? == Some(true));
            let restored = memory
                .session_catalog_record("managed-lifecycle")
                .await?
                .context("accepted restore lost its catalog row")?;
            ensure!(
                restored.lifecycle_state == store::SessionLifecycleState::Active
                    && restored.lifecycle_generation == 3,
                "managed restore changed retained session metadata"
            );
            ensure!(
                matches!(
                    memory
                        .session_catalog_page(None, None, None, 16)
                        .await?
                        .records
                        .as_slice(),
                    [record] if record.label == "renamed" && record.updated_order == restored.updated_order
                ),
                "managed restore lost the retained session record"
            );

            let exact_id = Uuid::new_v4();
            let exact = {
                let _mutation = remote.session.mutations.lock().await;
                let mut attachment = remote.attachment.lock().await;
                session_lifecycle_outcome(
                    remote
                        .checked_call_locked_with_id(
                            &mut attachment,
                            ServiceCall::View {
                                candidate: remote.candidate,
                                operation: Box::new(ViewOperation::RenameSession {
                                    session_id: "managed-lifecycle".into(),
                                    expected_generation: 3,
                                    label: "exact receipt".into(),
                                }),
                            },
                            exact_id,
                        )
                        .await?,
                )?
            };
            ensure!(
                exact.lifecycle_generation == 4
                    && exact.lifecycle_state == store::SessionLifecycleState::Active,
                "managed exact receipt returned the wrong original outcome"
            );
            let later = sibling.remove_session("managed-lifecycle", 4).await?;
            ensure!(later.lifecycle_generation == 5);
            let later_revision = sibling.revision().await?;
            let replayed = {
                let _mutation = remote.session.mutations.lock().await;
                let mut attachment = remote.attachment.lock().await;
                session_lifecycle_outcome(
                    remote
                        .checked_call_locked_with_id(
                            &mut attachment,
                            ServiceCall::View {
                                candidate: remote.candidate,
                                operation: Box::new(ViewOperation::RenameSession {
                                    session_id: "managed-lifecycle".into(),
                                    expected_generation: 3,
                                    label: "exact receipt".into(),
                                }),
                            },
                            exact_id,
                        )
                        .await?,
                )?
            };
            ensure!(
                replayed == exact && sibling.revision().await? == later_revision,
                "managed exact retry changed or replaced its original receipt outcome"
            );
            let changed = {
                let _mutation = remote.session.mutations.lock().await;
                let mut attachment = remote.attachment.lock().await;
                remote
                    .checked_call_locked_with_id(
                        &mut attachment,
                        ServiceCall::View {
                            candidate: remote.candidate,
                            operation: Box::new(ViewOperation::RenameSession {
                                session_id: "managed-lifecycle".into(),
                                expected_generation: 3,
                                label: "changed receipt".into(),
                            }),
                        },
                        exact_id,
                    )
                    .await
                    .unwrap_err()
            };
            ensure!(
                changed.to_string().contains("different operation")
                    && sibling.revision().await? == later_revision,
                "managed changed-payload retry did not remain a no-effect receipt conflict"
            );

            let candidate = memory.begin_candidate("session lifecycle candidate").await?;
            let candidate_record = candidate
                .view()
                .create_session("candidate-only", Mode::Ifs, "candidate label")
                .await?;
            ensure!(
                candidate_record.session_id == "candidate-only"
                    && candidate
                        .view()
                        .session_catalog_page(None, None, None, 16)
                        .await?
                        .records
                        .iter()
                        .any(|record| record.session_id == "candidate-only"),
                "candidate lifecycle mutation did not remain readable on its branch"
            );
            ensure!(
                !memory
                    .session_catalog_page(None, None, None, 16)
                    .await?
                    .records
                    .iter()
                    .any(|record| record.session_id == "candidate-only"),
                "candidate lifecycle mutation leaked into main"
            );
            candidate.abandon().await?;

            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("managed lifecycle fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("managed lifecycle fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_fork_lost_reply_recovers_one_atomic_child_and_candidate_stays_isolated()
    -> Result<()> {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
            let session = "managed-fork-parent";
            let namespace = format!("{}/transcript/managed-fork-parent", options.project_scope);
            memory
                .create_session(session, Mode::Ifs, "fork parent")
                .await?;
            memory
                .checkpoint_session_turn(
                    &namespace,
                    session,
                    &[Message::text("user", "shared question")],
                    &[("managed-fork-journal".into(), json!({"state": "started"}))],
                    &store::SessionTurnCheckpoint::Admit {
                        expected_generation: 0,
                        turn_id: "shared-turn".into(),
                        label: None,
                        expected_transcript_rows: None,
                    },
                )
                .await?;
            memory
                .checkpoint_session_turn(
                    &namespace,
                    session,
                    &[Message::text("assistant", "shared answer")],
                    &[("managed-fork-journal".into(), json!({"state": "ended"}))],
                    &store::SessionTurnCheckpoint::Settle {
                        expected_generation: 0,
                        turn_id: "shared-turn".into(),
                        settlement: store::PublicTurnSettlement::Completed,
                        speaker_id: "part-a".into(),
                    },
                )
                .await?;
            let parent_page = memory.public_transcript_page(session, None, 16).await?;
            let selected = match parent_page.records.as_slice() {
                [store::PublicTranscriptEntry::Turn { record }] => record.clone(),
                _ => bail!("managed fork parent has the wrong settled prefix"),
            };

            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed fork fixture did not attach to the service")
            };
            let before_fork = sibling.revision().await?;
            cancel_before_session_acceptance(
                remote,
                memory.fork_session(
                    session,
                    0,
                    &selected.node_id,
                    "pre-acceptance-fork-child",
                    "cancelled fork",
                ),
            )
            .await?;
            ensure!(
                sibling.revision().await? == before_fork
                    && sibling
                        .session_catalog_record("pre-acceptance-fork-child")
                        .await?
                        .is_none(),
                "pre-acceptance fork cancellation published a child"
            );
            let pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(pause.clone());
            let fork = tokio::spawn({
                let memory = memory.clone();
                let node_id = selected.node_id.clone();
                async move {
                    memory
                        .fork_session(
                            session,
                            0,
                            &node_id,
                            "managed-fork-child",
                            "lost reply child",
                        )
                        .await
                }
            });
            let _fork_cleanup = AbortOnDrop(fork.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                .await
                .context("accepted fork frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let page = sibling.session_catalog_page(None, None, None, 16).await?;
                    if page
                        .records
                        .iter()
                        .any(|record| record.session_id == "managed-fork-child")
                    {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit the paused fork publication")??;
            fork.abort();
            let stopped = tokio::time::timeout(Duration::from_secs(5), fork)
                .await
                .context("cancelled fork future did not end")?;
            ensure!(
                stopped.is_err_and(|error| error.is_cancelled()),
                "accepted fork future completed instead of being cancelled"
            );
            ensure!(
                memory
                    .rename_session(session, 0, "must remain fenced")
                    .await
                    .is_err(),
                "uncertain fork receipt failed to fence later mutation"
            );
            ensure!(memory.reconcile().await? == Some(true));

            let catalog = memory.session_catalog_page(None, None, None, 16).await?;
            let children: Vec<_> = catalog
                .records
                .iter()
                .filter(|record| record.session_id == "managed-fork-child")
                .collect();
            ensure!(children.len() == 1, "lost fork reply duplicated its child");
            let child = children[0];
            ensure!(
                child.head_node_id.as_deref() == Some(selected.node_id.as_str())
                    && child.fork_provenance.as_ref().is_some_and(|fork| {
                        fork.source_session_id == session
                            && fork.source_node_id == selected.node_id
                            && fork.source_turn_id == selected.turn_id
                            && fork.source_label == "fork parent"
                            && fork.shares_current_project_memory
                    }),
                "recovered fork lost its immutable source provenance"
            );
            let child_page = memory
                .public_transcript_page("managed-fork-child", None, 16)
                .await?;
            ensure!(
                child_page.head_node_id.as_deref() == Some(selected.node_id.as_str())
                    && child_page.records == parent_page.records,
                "recovered child lost its selected settled prefix"
            );

            let candidate = memory.begin_candidate("isolated session fork").await?;
            candidate
                .view()
                .fork_session(
                    session,
                    0,
                    &selected.node_id,
                    "candidate-fork-child",
                    "candidate child",
                )
                .await?;
            ensure!(
                candidate
                    .view()
                    .session_catalog_page(None, None, None, 16)
                    .await?
                    .records
                    .iter()
                    .any(|record| record.session_id == "candidate-fork-child"),
                "candidate fork was not readable on its branch"
            );
            ensure!(
                !memory
                    .session_catalog_page(None, None, None, 16)
                    .await?
                    .records
                    .iter()
                    .any(|record| record.session_id == "candidate-fork-child"),
                "candidate fork leaked into main"
            );
            candidate.abandon().await?;

            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("managed fork fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("managed fork fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_mode_checkpoint_lost_reply_reconciles_catalog_and_state_once() -> Result<()> {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
                crate::test_support::open_options(root.path().join("private"), scope.clone())?;
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
            let session = "managed-mode";
            memory.create_session(session, Mode::Ifs, "").await?;
            let before = memory.revision().await?;
            let namespace = format!("{scope}/transcript/{session}");
            let state_key = format!("{scope}/session/{session}");
            let state = json!({"id": session, "mode": Mode::Jungian, "lifecycle_generation": 0});
            let updates = vec![(state_key.clone(), state.clone())];
            let wrong_scope = "project/foreign";
            let wrong_namespace = format!("{wrong_scope}/transcript/{session}");
            let wrong_updates = vec![(format!("{wrong_scope}/session/{session}"), state.clone())];
            ensure!(
                memory
                    .checkpoint_session_mode(
                        &wrong_namespace,
                        session,
                        &wrong_updates,
                        &store::SessionModeCheckpoint {
                            expected_generation: 0,
                            expected_mode: Mode::Ifs,
                            mode: Mode::Jungian,
                        },
                    )
                    .await
                    .is_err(),
                "foreign-scope mode checkpoint was accepted"
            );
            ensure!(
                memory.revision().await? == before,
                "foreign-scope mode checkpoint changed the project"
            );
            ensure!(
                memory.reconcile().await? == Some(false),
                "foreign-scope mode checkpoint did not reconcile as a no-effect request"
            );
            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed mode fixture did not attach to the service")
            };
            let malformed = remote
                .call(ViewOperation::CheckpointSession {
                    namespace: namespace.clone(),
                    session_id: session.into(),
                    messages: vec![Message::text("user", "forged")],
                    values: updates.clone(),
                    public_turn: Some(store::SessionTurnCheckpoint::Admit {
                        expected_generation: 0,
                        turn_id: "forged".into(),
                        label: None,
                        expected_transcript_rows: None,
                    }),
                    mode: Some(store::SessionModeCheckpoint {
                        expected_generation: 0,
                        expected_mode: Mode::Ifs,
                        mode: Mode::Jungian,
                    }),
                })
                .await;
            ensure!(
                malformed.is_err(),
                "server accepted combined mode and public turn checkpoint"
            );
            ensure!(
                memory.revision().await? == before,
                "malformed mode checkpoint changed the project"
            );
            ensure!(
                memory.reconcile().await? == Some(false),
                "malformed mode checkpoint did not reconcile as a no-effect request"
            );
            let _ = memory.revision().await?;
            let barrier = crate::test_support::ReplyBarrier::default();
            memory.fixture_pause_next_service_reply(&barrier).await?;
            let change = tokio::spawn({
                let memory = memory.clone();
                let namespace = namespace.clone();
                async move {
                    memory
                        .checkpoint_session_mode(
                            &namespace,
                            session,
                            &updates,
                            &store::SessionModeCheckpoint {
                                expected_generation: 0,
                                expected_mode: Mode::Ifs,
                                mode: Mode::Jungian,
                            },
                        )
                        .await
                }
            });
            let _cleanup = AbortOnDrop(change.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), barrier.wait_sent())
                .await
                .context("mode checkpoint request was not sent")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let catalog = sibling
                        .session_catalog_record(session)
                        .await?
                        .context("mode catalog disappeared")?;
                    let saved = sibling.get(&state_key).await?;
                    if catalog.mode == Mode::Jungian && saved.as_ref() == Some(&state) {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit paused mode and state")??;
            change.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), change)
                    .await
                    .context("cancelled mode checkpoint did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "mode checkpoint reply was not cancelled"
            );
            ensure!(
                memory.reconcile().await? == Some(true),
                "accepted mode checkpoint lost its exact receipt"
            );
            ensure!(
                memory.revision().await? != before,
                "accepted mode checkpoint did not commit"
            );
            ensure!(
                memory
                    .public_transcript_page(session, None, 16)
                    .await?
                    .records
                    .is_empty(),
                "mode checkpoint fabricated a public turn"
            );
            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("managed mode fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("managed mode fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_public_turn_lost_reply_reconciles_without_duplicate_settlement() -> Result<()>
    {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
            let namespace = format!("{}/transcript/managed-turn", options.project_scope);
            let candidate_namespace =
                format!("{}/transcript/candidate-turn", options.project_scope);
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
            memory
                .create_session("managed-turn", Mode::Ifs, "")
                .await?;
            let Backend::Remote(remote) = &memory.backend else {
                bail!("managed turn fixture did not attach to the service")
            };
            let pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(pause.clone());
            let admission = tokio::spawn({
                let memory = memory.clone();
                let namespace = namespace.clone();
                async move {
                    memory
                        .checkpoint_session_turn(
                            &namespace,
                            "managed-turn",
                            &[Message::text("user", "question")],
                            &[("managed-turn-journal".into(), json!("started"))],
                            &store::SessionTurnCheckpoint::Admit {
                                expected_generation: 0,
                                turn_id: "turn-1".into(),
                                label: Some("question".into()),
                                expected_transcript_rows: None,
                            },
                        )
                        .await
                }
            });
            let _admission_cleanup = AbortOnDrop(admission.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                .await
                .context("accepted public-turn admission frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let page = sibling
                        .public_transcript_page("managed-turn", None, 16)
                        .await?;
                    if matches!(page.pending.as_ref(), Some(record) if record.turn_id == "turn-1") {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit the paused public-turn admission")??;
            admission.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), admission)
                    .await
                    .context("cancelled public-turn admission did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted public-turn admission completed instead of being cancelled"
            );
            ensure!(
                memory
                    .checkpoint_session_turn(
                        &namespace,
                        "managed-turn",
                        &[Message::text("assistant", "answer")],
                        &[("managed-turn-journal".into(), json!("ended"))],
                        &store::SessionTurnCheckpoint::Settle {
                            expected_generation: 0,
                            turn_id: "turn-1".into(),
                            settlement: store::PublicTurnSettlement::Completed,
                            speaker_id: "part-a".into(),
                        },
                    )
                    .await
                    .is_err(),
                "uncertain public-turn admission failed to fence settlement"
            );
            ensure!(memory.reconcile().await? == Some(true));
            // The cancelled admission left its attachment incomplete. A read
            // reattaches it before the next reply barrier is installed.
            let _ = memory.revision().await?;
            let settlement_barrier = crate::test_support::ReplyBarrier::default();
            memory
                .fixture_pause_next_service_reply(&settlement_barrier)
                .await?;
            let mut settlement = tokio::spawn({
                let memory = memory.clone();
                let namespace = namespace.clone();
                async move {
                    memory
                        .checkpoint_session_turn(
                            &namespace,
                            "managed-turn",
                            &[Message::text("assistant", "answer")],
                            &[("managed-turn-journal".into(), json!("ended"))],
                            &store::SessionTurnCheckpoint::Settle {
                                expected_generation: 0,
                                turn_id: "turn-1".into(),
                                settlement: store::PublicTurnSettlement::Completed,
                                speaker_id: "part-a".into(),
                            },
                        )
                        .await
                }
            });
            let _settlement_cleanup = AbortOnDrop(settlement.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    () = settlement_barrier.wait_sent() => Ok(()),
                    result = &mut settlement => bail!("settlement completed before pause: {result:?}"),
                }
            })
            .await
            .context("accepted public-turn settlement frame was not flushed")??;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let page = sibling.public_transcript_page("managed-turn", None, 16).await?;
                    if matches!(page.records.as_slice(), [store::PublicTranscriptEntry::Turn { record }]
                        if record.turn_id == "turn-1"
                            && record.settlement == store::PublicTurnSettlement::Completed)
                    {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit the paused public-turn settlement")??;
            settlement.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), settlement)
                    .await
                    .context("cancelled public-turn settlement did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted public-turn settlement completed instead of being cancelled"
            );
            ensure!(memory.reconcile().await? == Some(true));
            let page = sibling
                .public_transcript_page("managed-turn", None, 16)
                .await?;
            ensure!(
                page.pending.is_none()
                    && matches!(page.records.as_slice(), [store::PublicTranscriptEntry::Turn { record }] if record.turn_id == "turn-1" && record.settlement == store::PublicTurnSettlement::Completed),
                "managed public turn did not settle exactly once"
            );
            ensure!(
                sibling
                    .history(&namespace, 16)
                    .await?
                    == [
                        Message::text("user", "question"),
                        Message::text("assistant", "answer")
                    ],
                "managed public turn duplicated its raw transcript"
            );

            let candidate = memory.begin_candidate("public turn candidate").await?;
            candidate
                .view()
                .create_session("candidate-turn", Mode::Ifs, "candidate")
                .await?;
            candidate
                .view()
                .checkpoint_session_turn(
                    &candidate_namespace,
                    "candidate-turn",
                    &[Message::text("user", "candidate question")],
                    &[("candidate-journal".into(), json!("started"))],
                    &store::SessionTurnCheckpoint::Admit {
                        expected_generation: 0,
                        turn_id: "candidate-turn-1".into(),
                        label: None,
                        expected_transcript_rows: None,
                    },
                )
                .await?;
            ensure!(
                candidate
                    .view()
                    .public_transcript_page("candidate-turn", None, 16)
                    .await?
                    .pending
                    .is_some()
                    && memory
                        .session_catalog_page(None, None, None, 16)
                        .await?
                        .records
                        .iter()
                        .all(|record| record.session_id != "candidate-turn"),
                "candidate public-turn admission leaked into main"
            );
            candidate.abandon().await?;

            memory
                .checkpoint_session_turn(
                    &namespace,
                    "managed-turn",
                    &[Message::text("user", "older question")],
                    &[("older-journal".into(), json!("started"))],
                    &store::SessionTurnCheckpoint::Admit {
                        expected_generation: 0,
                        turn_id: "older-turn".into(),
                        label: None,
                        expected_transcript_rows: None,
                    },
                )
                .await?;
            memory
                .checkpoint_session_turn(
                    &namespace,
                    "managed-turn",
                    &[Message::text("kuru-interruption", "retryable")],
                    &[("older-journal".into(), json!("interrupted"))],
                    &store::SessionTurnCheckpoint::MarkRetryableInterruption {
                        expected_generation: 0,
                        turn_id: "older-turn".into(),
                        speaker_id: "kuru-interruption".into(),
                    },
                )
                .await?;
            memory
                .checkpoint_session_turn(
                    &namespace,
                    "managed-turn",
                    &[Message::text("user", "later question")],
                    &[("later-journal".into(), json!("started"))],
                    &store::SessionTurnCheckpoint::Admit {
                        expected_generation: 0,
                        turn_id: "later-turn".into(),
                        label: None,
                        expected_transcript_rows: None,
                    },
                )
                .await?;
            memory
                .checkpoint_session_turn(
                    &namespace,
                    "managed-turn",
                    &[Message::text("assistant", "later answer")],
                    &[("later-journal".into(), json!("ended"))],
                    &store::SessionTurnCheckpoint::Settle {
                        expected_generation: 0,
                        turn_id: "later-turn".into(),
                        settlement: store::PublicTurnSettlement::Completed,
                        speaker_id: "part-later".into(),
                    },
                )
                .await?;
            let predecessor = sibling
                .public_transcript_page("managed-turn", None, 16)
                .await?
                .head_node_id;
            let older_node = store::public_turn_node_id("managed-turn", "older-turn")?;
            memory
                .fork_session(
                    "managed-turn",
                    0,
                    &older_node,
                    "older-boundary-fork",
                    "older boundary",
                )
                .await?;
            let fork_before = sibling
                .public_transcript_page("older-boundary-fork", None, 16)
                .await?;
            ensure!(
                fork_before.head_node_id.as_deref() == Some(older_node.as_str()),
                "older retry fork did not capture its interrupted boundary"
            );
            let _ = memory.revision().await?;
            let continuation_barrier = crate::test_support::ReplyBarrier::default();
            memory
                .fixture_pause_next_service_reply(&continuation_barrier)
                .await?;
            let mut continuation = tokio::spawn({
                let memory = memory.clone();
                let namespace = namespace.clone();
                async move {
                    memory
                        .checkpoint_session_turn(
                            &namespace,
                            "managed-turn",
                            &[],
                            &[("older-journal".into(), json!("resumed"))],
                            &store::SessionTurnCheckpoint::Resume {
                                expected_generation: 0,
                                turn_id: "older-turn".into(),
                                legacy: None,
                            },
                        )
                        .await
                }
            });
            let _continuation_cleanup = AbortOnDrop(continuation.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), async {
                tokio::select! {
                    () = continuation_barrier.wait_sent() => Ok(()),
                    result = &mut continuation => bail!("older continuation completed before paused reply: {result:?}"),
                }
            })
            .await
            .context("accepted older continuation frame was not flushed")??;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let page = sibling.public_transcript_page("managed-turn", None, 16).await?;
                    if matches!(page.pending.as_ref(), Some(record)
                        if record.turn_id == "older-turn"
                            && record.kind == store::PublicTurnKind::Continuation
                            && record.predecessor_node_id == predecessor)
                    {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit older continuation before reply loss")??;
            continuation.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), continuation)
                    .await
                    .context("cancelled older continuation did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted older continuation completed before cancellation"
            );
            ensure!(memory.reconcile().await? == Some(true));
            let pending = sibling
                .public_transcript_page("managed-turn", None, 16)
                .await?
                .pending
                .context("reconciled older continuation is not pending")?;
            ensure!(
                pending.turn_id == "older-turn"
                    && pending.kind == store::PublicTurnKind::Continuation
                    && pending.user_entry.is_none()
                    && pending.continuation_of_node_id
                        == Some(store::public_turn_node_id("managed-turn", "older-turn")?),
                "older continuation lost its exact assistant-only identity"
            );
            memory
                .checkpoint_session_turn(
                    &namespace,
                    "managed-turn",
                    &[Message::text("assistant", "older answer")],
                    &[("older-journal".into(), json!("ended"))],
                    &store::SessionTurnCheckpoint::Settle {
                        expected_generation: 0,
                        turn_id: "older-turn".into(),
                        settlement: store::PublicTurnSettlement::Completed,
                        speaker_id: "part-older".into(),
                    },
                )
                .await?;
            ensure!(
                sibling.history(&namespace, 16).await?
                    == [
                        Message::text("user", "question"),
                        Message::text("assistant", "answer"),
                        Message::text("user", "older question"),
                        Message::text("kuru-interruption", "retryable"),
                        Message::text("user", "later question"),
                        Message::text("assistant", "later answer"),
                        Message::text("assistant", "older answer"),
                    ],
                "older continuation duplicated a user or interruption row"
            );
            let fork_after = sibling
                .public_transcript_page("older-boundary-fork", None, 16)
                .await?;
            ensure!(
                fork_after.head_node_id == fork_before.head_node_id
                    && fork_after.records == fork_before.records,
                "older continuation changed the fork's settled prefix"
            );

            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("managed public-turn fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("managed public-turn fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_legacy_continuation_lost_reply_reconciles_without_a_user_row() -> Result<()> {
        // Real service lifecycles: a local seed open, then one service owner.
        let deadline = fixture_deadline(2);
        tokio::time::timeout(deadline, async {
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
                crate::test_support::open_options(root.path().join("private"), scope.clone())?;
            let namespace = format!("{scope}/transcript/legacy-managed");
            let journal_key = format!(
                "{scope}/session/legacy-managed/turn/{}",
                "b".repeat(64)
            );
            let expected_journal = json!({
                "format": 2,
                "id": "legacy-managed-turn",
                "prompt": "legacy question",
                "target": null,
                "transitions": ["Started", "Interrupted"],
                "possible_dispatch": false,
                "interruption_marker": true,
                "output": null
            });
            let resumed_journal = json!({
                "format": 2,
                "id": "legacy-managed-turn",
                "prompt": "legacy question",
                "target": null,
                "transitions": ["Started", "Interrupted", "Resumed"],
                "possible_dispatch": false,
                "interruption_marker": true,
                "output": null
            });
            let seed = store::MemoryStore::open(options.clone()).await?;
            seed.append_session_message(
                &namespace,
                "legacy-managed",
                &Message::text("user", "legacy question"),
            )
            .await?;
            let source = seed
                .session_history_window_after(&namespace, "legacy-managed", 0, 16)
                .await?;
            let source_row = source
                .rows
                .first()
                .context("legacy managed seed row is missing")?;
            let prefix = store::LegacyTranscriptPrefix {
                namespace: namespace.clone(),
                source_session_id: "legacy-managed".into(),
                source_revision: source.revision,
                first_sequence: source_row.sequence,
                through_sequence: source_row.sequence,
                row_count: 1,
                record_format: store::LEGACY_PREFIX_RECORD_FORMAT.into(),
            };
            seed.put(&journal_key, &expected_journal).await?;
            seed.fixture_insert_public_session(
                &store::SessionCatalogRecord {
                    session_id: "legacy-managed".into(),
                    mode: kuru_core::Mode::Ifs,
                    label: "legacy managed".into(),
                    created_order: 1,
                    updated_order: 1,
                    lifecycle_generation: 0,
                    lifecycle_state: store::SessionLifecycleState::Active,
                    head_node_id: None,
                    pending_node_id: None,
                    legacy_prefix: Some(prefix.clone()),
                    fork_provenance: None,
                    record_format: store::SESSION_CATALOG_RECORD_FORMAT.into(),
                },
                &[],
            )
            .await?;
            seed.close().await?;

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
            let barrier = crate::test_support::ReplyBarrier::default();
            memory.fixture_pause_next_service_reply(&barrier).await?;
            let resume = tokio::spawn({
                let memory = memory.clone();
                let namespace = namespace.clone();
                let journal_key = journal_key.clone();
                let prefix = prefix.clone();
                let expected_journal = expected_journal.clone();
                let resumed_journal = resumed_journal.clone();
                async move {
                    memory
                        .checkpoint_session_turn(
                            &namespace,
                            "legacy-managed",
                            &[],
                            &[(journal_key.clone(), resumed_journal)],
                            &store::SessionTurnCheckpoint::Resume {
                                expected_generation: 0,
                                turn_id: "legacy-managed-turn".into(),
                                legacy: Some(store::LegacySessionTurnResume {
                                    legacy_prefix: prefix,
                                    journal_key,
                                    expected_journal,
                                }),
                            },
                        )
                        .await
                }
            });
            let _resume_cleanup = AbortOnDrop(resume.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), barrier.wait_sent())
                .await
                .context("accepted legacy continuation frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let page = sibling
                        .public_transcript_page("legacy-managed", None, 16)
                        .await?;
                    if matches!(page.pending.as_ref(), Some(record) if record.kind == store::PublicTurnKind::LegacyContinuation) {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not commit the paused legacy continuation")??;
            resume.abort();
            ensure!(
                tokio::time::timeout(Duration::from_secs(5), resume)
                    .await
                    .context("cancelled legacy continuation did not end")?
                    .is_err_and(|error| error.is_cancelled()),
                "accepted legacy continuation completed instead of being cancelled"
            );
            ensure!(memory.reconcile().await? == Some(true));
            let page = sibling
                .public_transcript_page("legacy-managed", None, 16)
                .await?;
            ensure!(
                matches!(page.pending.as_ref(), Some(record)
                    if record.kind == store::PublicTurnKind::LegacyContinuation
                        && record.user_entry.is_none()
                        && record.continuation_of_node_id.is_none()),
                "reconciled legacy continuation changed its honest projection"
            );
            ensure!(
                sibling.history(&namespace, 16).await?
                    == [Message::text("user", "legacy question")],
                "legacy continuation duplicated its retained user row"
            );

            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("managed legacy continuation owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("managed legacy continuation fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_after_accepted_unit_frame_fences_clones_until_indexed_proof() -> Result<()>
    {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
        .with_context(|| {
            format!("accepted cancelled write fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn remote_reasoning_summary_lost_reply_reconciles_one_atomic_receipt() -> Result<()> {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
                bail!("reasoning summary fixture did not attach to the managed service")
            };
            let record = crate::ReasoningSummaryRecord {
                session_id: "session".into(),
                turn_id: Some("turn".into()),
                operation_id: None,
                actor_id: "actor".into(),
                invocation_id: "invocation".into(),
                item_id: Some("item".into()),
                output_index: Some(0),
                summary_index: 0,
                text: "settled private summary".into(),
            };
            let expected = serde_json::to_value(&record)?;
            let key = store::reasoning_summary_key(&record)?;
            let pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(pause.clone());
            let writer = tokio::spawn({
                let memory = memory.clone();
                let record = record.clone();
                async move { memory.put_reasoning_summaries(&[record]).await }
            });
            let _writer_cleanup = AbortOnDrop(writer.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                .await
                .context("reasoning summary reply frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if sibling.get(&key).await? == Some(expected.clone()) {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not durably commit the paused reasoning summary batch")??;
            writer.abort();
            let stopped = tokio::time::timeout(Duration::from_secs(5), writer)
                .await
                .context("cancelled reasoning summary write did not end")?;
            ensure!(
                stopped.is_err_and(|error| error.is_cancelled()),
                "paused reasoning summary write completed instead of being cancelled"
            );
            ensure!(
                memory
                    .put_reasoning_summaries(std::slice::from_ref(&record))
                    .await
                    .is_err(),
                "a cancelled summary receipt failed to fence further mutations"
            );
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    match memory.reconcile().await {
                        Ok(Some(true)) => break Ok::<(), anyhow::Error>(()),
                        Err(error) if error.to_string().contains("remains uncertain") => {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        other => {
                            bail!("paused reasoning summary had unexpected outcome: {other:?}")
                        }
                    }
                }
            })
            .await
            .context("reasoning summary indexed-outcome deadline")??;
            memory
                .put_reasoning_summaries(std::slice::from_ref(&record))
                .await?;
            let mut conflicting = record.clone();
            conflicting.text = "conflicting payload".into();
            let error = memory
                .put_reasoning_summaries(&[conflicting])
                .await
                .unwrap_err();
            ensure!(
                error
                    .downcast_ref::<store::ReasoningSummaryConflict>()
                    .is_some(),
                "managed settled-identity conflict was not typed definite rejection: {error:#}"
            );
            let mut later = record;
            later.summary_index = 1;
            later.text = "later settled private summary".into();
            memory.put_reasoning_summaries(&[later]).await?;
            memory.close().await?;
            sibling.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("reasoning summary fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!(
                "remote reasoning summary lost-reply fixture exceeded its {deadline:?} deadline"
            )
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn remote_session_checkpoint_lost_reply_preserves_pinned_provenance() -> Result<()> {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
            let inspector = open().await?;
            let actor = "project/example/ifs/identity/actor";
            memory
                .append_message(actor, &Message::text("user", "legacy"))
                .await?;
            memory
                .append_session_message(actor, "session-a", &Message::text("user", "source"))
                .await?;
            let session_window = memory
                .session_history_window(actor, "session-a", 16)
                .await?;
            ensure!(
                session_window.total_rows == 1
                    && session_window.messages[0].plain_text() == Some("source"),
                "managed session history included legacy or another-session rows"
            );
            let session_suffix = memory
                .session_history_window_after(actor, "session-a", 0, 16)
                .await?;
            ensure!(
                session_suffix.view == "main"
                    && session_suffix.total_rows == 1
                    && session_suffix.rows.len() == 1
                    && session_suffix.rows[0].message.plain_text() == Some("source"),
                "managed session cursor history returned the wrong source suffix"
            );
            let snapshot = memory
                .session_source_snapshot(actor, "session-a", actor, 0, 16)
                .await?;
            ensure!(
                snapshot.rows.len() == 1,
                "session snapshot leaked another row"
            );
            let through = snapshot
                .through_inclusive
                .context("session snapshot omitted its inclusive boundary")?;
            let record = store::ContextSummaryRecord {
                actor_namespace: actor.into(),
                session_id: "session-a".into(),
                source_namespace: actor.into(),
                summary_namespace: format!("{actor}/session/session-a/summaries"),
                source_view: snapshot.view,
                source_revision: snapshot.revision,
                after_sequence: snapshot.after_exclusive,
                through_sequence: through,
                turn_id: None,
                operation_id: Some("compact-a".into()),
                producer_actor_id: Some("producer-a".into()),
                invocation_id: "invocation-a".into(),
                summary: "managed summary".into(),
            };
            let private = crate::ReasoningSummaryRecord {
                session_id: "session-a".into(),
                turn_id: None,
                operation_id: Some("compact-a".into()),
                actor_id: "producer-a".into(),
                invocation_id: "invocation-a".into(),
                item_id: Some("item-a".into()),
                output_index: Some(0),
                summary_index: 0,
                text: "private compact reasoning".into(),
            };
            let private_key = store::reasoning_summary_key(&private)?;
            let checkpoint = store::ContextSummaryCheckpoint {
                record: record.clone(),
                private_reasoning: vec![private.clone()],
            };
            let Backend::Remote(remote) = &memory.backend else {
                bail!("session checkpoint fixture did not attach to the service")
            };
            let pause = Arc::new(service::rpc::ReplyPause::default());
            remote
                .attachment
                .lock()
                .await
                .pause_after_next_send(pause.clone());
            let mut invalid = checkpoint.clone();
            invalid.private_reasoning[0].actor_id = "wrong-producer".into();
            let rejected = memory
                .checkpoint_context_summary(&invalid)
                .await
                .expect_err("mismatched private producer reached the managed service");
            ensure!(
                rejected
                    .to_string()
                    .contains("private compact reasoning provenance does not match"),
                "checkpoint rejected the wrong invalid field: {rejected:#}"
            );
            ensure!(
                tokio::time::timeout(Duration::from_millis(100), pause.sent.notified())
                    .await
                    .is_err(),
                "invalid checkpoint acquired a mutating attachment and sent an RPC frame"
            );
            let mut oversized = checkpoint.clone();
            oversized.record.summary = "\u{0001}".repeat(16 * 1024 * 1024);
            let rejected = memory
                .checkpoint_context_summary(&oversized)
                .await
                .expect_err("oversized checkpoint reached the managed service");
            ensure!(
                rejected
                    .to_string()
                    .contains("context summary checkpoint exceeds 64 MiB"),
                "checkpoint rejected the wrong oversized field: {rejected:#}"
            );
            ensure!(
                tokio::time::timeout(Duration::from_millis(100), pause.sent.notified())
                    .await
                    .is_err(),
                "oversized checkpoint acquired a mutating attachment and sent an RPC frame"
            );
            drop(oversized);
            let request_id = Uuid::new_v4();
            let writer = tokio::spawn({
                let remote = remote.clone();
                let checkpoint = checkpoint.clone();
                async move {
                    let _mutation = remote.session.mutations.lock().await;
                    let mut attachment = remote.attachment.lock().await;
                    unit(
                        remote
                            .checked_call_locked_with_id(
                                &mut attachment,
                                ServiceCall::View {
                                    candidate: remote.candidate,
                                    operation: Box::new(ViewOperation::CheckpointContextSummary {
                                        record: checkpoint.record,
                                        private_reasoning: checkpoint.private_reasoning,
                                    }),
                                },
                                request_id,
                            )
                            .await?,
                    )
                }
            });
            let _writer_cleanup = AbortOnDrop(writer.abort_handle());
            tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                .await
                .context("context checkpoint frame was not flushed")?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if inspector
                        .context_summary_cursor(actor, "session-a", actor)
                        .await?
                        .is_some()
                    {
                        break Ok::<(), anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("owner did not atomically publish the context checkpoint")??;
            writer.abort();
            let stopped = tokio::time::timeout(Duration::from_secs(5), writer)
                .await
                .context("cancelled context checkpoint did not end")?;
            ensure!(
                stopped.is_err_and(|error| error.is_cancelled()),
                "paused context checkpoint completed instead of being cancelled"
            );
            ensure!(
                memory
                    .append_session_message(actor, "session-a", &Message::text("user", "blocked"),)
                    .await
                    .is_err(),
                "lost checkpoint reply did not fence its session"
            );
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    match memory.reconcile().await {
                        Ok(Some(true)) => break Ok::<(), anyhow::Error>(()),
                        Err(error) if error.to_string().contains("remains uncertain") => {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        other => bail!("context checkpoint had unexpected outcome: {other:?}"),
                    }
                }
            })
            .await
            .context("context checkpoint indexed-outcome deadline")??;
            {
                let _mutation = remote.session.mutations.lock().await;
                let mut attachment = remote.attachment.lock().await;
                unit(
                    remote
                        .checked_call_locked_with_id(
                            &mut attachment,
                            ServiceCall::View {
                                candidate: remote.candidate,
                                operation: Box::new(ViewOperation::CheckpointContextSummary {
                                    record: checkpoint.record.clone(),
                                    private_reasoning: checkpoint.private_reasoning.clone(),
                                }),
                            },
                            request_id,
                        )
                        .await?,
                )?;
            }
            let summaries = inspector
                .context_summary_window(
                    actor,
                    &record.summary_namespace,
                    Some("session-a"),
                    Some(actor),
                    16,
                )
                .await?;
            ensure!(
                summaries.total_rows == 1
                    && summaries.records.len() == 1
                    && summaries.records[0].record == record,
                "managed summary projection did not return the cursor-selected record"
            );
            ensure!(
                inspector.get(&private_key).await? == Some(serde_json::to_value(&private)?),
                "managed checkpoint did not atomically retain its private sidecar"
            );
            let stale = memory
                .checkpoint_context_summary(&checkpoint)
                .await
                .expect_err("moved context cursor accepted a duplicate checkpoint");
            ensure!(
                stale.downcast_ref::<store::ContextSummaryStale>().is_some(),
                "stale context checkpoint was not a typed definite refusal: {stale:#}"
            );
            memory
                .append_session_message(
                    actor,
                    "session-a",
                    &Message::text("assistant", "after proof"),
                )
                .await?;

            let candidate = memory.begin_candidate("session candidate").await?;
            candidate
                .view()
                .append_session_message(
                    actor,
                    "session-a",
                    &Message::text("assistant", "candidate only"),
                )
                .await?;
            let candidate_page = candidate
                .view()
                .session_source_snapshot(actor, "session-a", actor, through, 16)
                .await?;
            ensure!(
                candidate_page
                    .rows
                    .iter()
                    .any(|row| row.message.plain_text() == Some("candidate only")),
                "candidate-pinned source snapshot omitted its private row"
            );
            let candidate_suffix = candidate
                .view()
                .session_history_window_after(actor, "session-a", through, 16)
                .await?;
            ensure!(
                candidate_suffix.view == candidate.branch()
                    && candidate_suffix.total_rows == 2
                    && candidate_suffix
                        .rows
                        .iter()
                        .any(|row| row.message.plain_text() == Some("candidate only")),
                "candidate-pinned cursor history omitted its private row"
            );
            ensure!(
                candidate
                    .view()
                    .session_history_window(actor, "session-a", 16)
                    .await?
                    .messages
                    .iter()
                    .any(|message| message.plain_text() == Some("candidate only")),
                "candidate-pinned session history omitted its private row"
            );
            ensure!(
                memory
                    .session_source_snapshot(actor, "session-a", actor, through, 16)
                    .await?
                    .rows
                    .iter()
                    .all(|row| row.message.plain_text() != Some("candidate only")),
                "candidate row leaked into the main session snapshot"
            );
            let main_suffix = memory
                .session_history_window_after(actor, "session-a", through, 16)
                .await?;
            ensure!(
                main_suffix.view == "main"
                    && main_suffix.total_rows == 1
                    && main_suffix
                        .rows
                        .iter()
                        .all(|row| row.message.plain_text() != Some("candidate only")),
                "candidate row leaked into main cursor history"
            );
            ensure!(
                memory
                    .session_history_window(actor, "session-a", 16)
                    .await?
                    .messages
                    .iter()
                    .all(|message| message.plain_text() != Some("candidate only")),
                "candidate row leaked into main session history"
            );
            candidate.abandon().await?;
            memory.close().await?;
            inspector.close().await?;
            let permit = service::acquire_maintenance_permit(&options).await?;
            tokio::time::timeout(Duration::from_secs(10), served)
                .await
                .context("session checkpoint fixture owner did not reap")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("remote session checkpoint fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn selected_abandon_lost_reply_proves_staged_but_not_empty_ref() -> Result<()> {
        // Real service lifecycles: each of two iterations opens a local store,
        // then an owner and its successor.
        let deadline = fixture_deadline(6);
        tokio::time::timeout(deadline, async {
            for staged in [false, true] {
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
                let local = store::MemoryStore::open(options.clone()).await?;
                let candidate = local.begin_candidate("selected ref").await?;
                let branch = candidate.view().pinned_view().to_owned();
                let base = candidate.base().to_owned();
                if staged {
                    candidate.view().put("private-staged", &json!(true)).await?;
                }
                let target = candidate.view().revision().await?;
                ensure!((target != base) == staged);
                drop(candidate);
                local.close().await?;
                let owner = service::ServiceOwner::open(options.clone(), &project).await?;
                let owner_inspection = owner.inspection_store_for_test();
                let mut served = tokio::spawn(owner.serve());
                let _owner_cleanup = AbortOnDrop(served.abort_handle());
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
                let Backend::Remote(remote) = &memory.backend else {
                    bail!("selected ref fixture did not attach to the service")
                };
                let pause = Arc::new(service::rpc::ReplyPause::default());
                remote
                    .attachment
                    .lock()
                    .await
                    .pause_after_next_send(pause.clone());
                let writer = tokio::spawn({
                    let memory = memory.clone();
                    let branch = branch.clone();
                    let base = base.clone();
                    let target = target.clone();
                    async move { memory.abandon_candidate_ref(&branch, &base, &target).await }
                });
                let _writer_cleanup = AbortOnDrop(writer.abort_handle());
                tokio::time::timeout(Duration::from_secs(5), pause.sent.notified())
                    .await
                    .context("selected abandon frame was not flushed")?;
                // The frame-send barrier precedes owner dispatch. Inspect the
                // already-owned store without adding a sibling attachment;
                // otherwise that sibling can win the reservation race and
                // correctly cause selected abandonment to be refused.
                let mut last_status = None;
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if served.is_finished() {
                            let ended = (&mut served).await;
                            bail!("selected abandon owner exited before ref cleanup: {ended:?}");
                        }
                        let status = owner_inspection.candidate_ref_status(&branch).await?;
                        let reclaimed = status.state == store::CandidateRefState::Missing;
                        last_status = Some(status);
                        if reclaimed {
                            break Ok::<(), anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .with_context(|| format!("selected ref was not reclaimed: {last_status:?}"))??;
                drop(owner_inspection);
                // Now retain an authenticated sibling in the original
                // generation before cancelling the held client reply.
                let mut original_generation =
                    tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            match remote.session.factory.connect().await {
                                Ok(attachment) => break Ok::<_, anyhow::Error>(attachment),
                                Err(error) if service::is_peer_closed(&error) => {
                                    tokio::time::sleep(Duration::from_millis(20)).await;
                                }
                                Err(error) => return Err(error),
                            }
                        }
                    })
                    .await
                    .context("selected abandon reservation did not release")??;
                writer.abort();
                let stopped = tokio::time::timeout(Duration::from_secs(5), writer)
                    .await
                    .context("cancelled selected abandon did not end")?;
                ensure!(stopped.is_err_and(|error| error.is_cancelled()));
                ensure!(
                    matches!(
                        original_generation
                            .call(ServiceCall::CandidateRefStatus {
                                branch: branch.clone(),
                            })
                            .await?,
                        ServiceValue::CandidateRefStatus(status)
                            if status.state == store::CandidateRefState::Missing
                    ),
                    "sibling did not observe the reclaimed selected ref"
                );
                if staged {
                    // A newly invented same-generation request ID cannot
                    // turn a reclaimed ref into proof that it ran.
                    let generation = original_generation.generation().to_owned();
                    let unregistered = original_generation
                        .call(ServiceCall::SelectedAbandonOutcome {
                            original_id: Uuid::new_v4(),
                            original_generation: generation,
                            branch: branch.clone(),
                            base: base.clone(),
                            target: target.clone(),
                        })
                        .await?;
                    ensure!(matches!(
                        unregistered,
                        ServiceValue::CandidateTransitionOutcome(
                            service::rpc::CandidateTransitionResult::StillUncertain
                        )
                    ));
                }
                original_generation.close();
                // Force exact old-owner/Dolt retirement, then expose the
                // retained request only to a verified successor generation.
                let permit = service::acquire_maintenance_permit(&options).await?;
                tokio::time::timeout(Duration::from_secs(10), &mut served)
                    .await
                    .context("selected ref old owner did not reap")???;
                drop(permit);
                let successor = service::ServiceOwner::open(options.clone(), &project).await?;
                let successor_served = tokio::spawn(successor.serve());
                let _successor_cleanup = AbortOnDrop(successor_served.abort_handle());
                if staged {
                    ensure!(
                        memory.recover_selected_candidate_abandon().await?
                            == Some(SelectedAbandonResolution::Abandoned),
                        "staged selected abandonment lost its typed result"
                    );
                    let rebound = memory
                        .reopen_after_checked_recovery()
                        .await?
                        .context("settled selected abandonment did not rebind main")?;
                    ensure!(
                        memory
                            .put("old-main-remains-closed", &json!(true))
                            .await
                            .is_err(),
                        "checked rebind revived the retired main view"
                    );
                    rebound.put("after-proof", &json!(true)).await?;
                    rebound.close().await?;
                } else {
                    let error = memory
                        .recover_selected_candidate_abandon()
                        .await
                        .unwrap_err();
                    ensure!(
                        error.is::<SelectedAbandonUncertain>(),
                        "missing no-op ref falsely proved the selected request: {error:#}"
                    );
                    ensure!(memory.put("blocked", &json!(true)).await.is_err());
                }
                memory.close().await?;
                let inspector = open().await?;
                ensure!(
                    inspector.candidate_ref_status(&branch).await?.state
                        == store::CandidateRefState::Missing
                );
                inspector.close().await?;
                let permit = service::acquire_maintenance_permit(&options).await?;
                tokio::time::timeout(Duration::from_secs(10), successor_served)
                    .await
                    .context("selected ref successor owner did not reap")???;
                drop(permit);
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("selected abandonment fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn lost_candidate_unit_reply_reattaches_before_read_write_and_promotion() -> Result<()> {
        // Real service lifecycles: one owner, then an owner and its restarted successor.
        let deadline = fixture_deadline(3);
        tokio::time::timeout(deadline, async {
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
                let old_main_clone = memory.clone();
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
                                operation: Box::new(ViewOperation::Get {
                                    key: "accepted-private".into(),
                                }),
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
                let (rebound_main, rebound_clone) = if restart_owner {
                    // The first close completes the main attachment, then
                    // waits on this held old candidate attachment. Cancelling
                    // there must retain both weak registry entries so either
                    // old main clone can retry checked successor rebind.
                    let held = remote.attachment.lock().await;
                    ensure!(
                        tokio::time::timeout(
                            Duration::from_millis(50),
                            memory.reopen_after_checked_recovery(),
                        )
                        .await
                        .is_err(),
                        "retired attachment drain escaped its held candidate"
                    );
                    ensure!(
                        remote.session.extra_connections.is_closed(),
                        "cancelled rebind did not enter retired-session drain"
                    );
                    drop(held);
                    let rebound_clone = old_main_clone
                        .reopen_after_checked_recovery()
                        .await?
                        .context("old clone could not independently rebind")?;
                    let rebound_main = memory
                        .reopen_after_checked_recovery()
                        .await?
                        .context("cancelled main rebind could not retry")?;
                    ensure!(memory.get("accepted-private").await.is_err());
                    ensure!(old_main_clone.get("accepted-private").await.is_err());
                    (Some(rebound_main), Some(rebound_clone))
                } else {
                    (None, None)
                };
                ensure!(candidate.view().get("accepted-private").await? == Some(json!(1)));
                candidate.view().put("after-proof", &json!(2)).await?;
                candidate.promote().await?;
                let observer = if restart_owner {
                    rebound_main.context("missing checked successor main view")?
                } else {
                    memory.clone()
                };
                ensure!(observer.get("accepted-private").await? == Some(json!(1)));
                ensure!(observer.get("after-proof").await? == Some(json!(2)));
                observer.close().await?;
                if restart_owner {
                    rebound_clone
                        .context("missing independently rebound clone")?
                        .close()
                        .await?;
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
        .with_context(|| {
            format!("candidate unit recovery fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_promotion_recovery_uses_exact_target_and_releases_clone_fence() -> Result<()>
    {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
        .with_context(|| {
            format!("candidate promotion facade fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn preserved_candidate_conflict_reattaches_before_releasing_mutation_fence() -> Result<()>
    {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
        .with_context(|| {
            format!("candidate conflict reattachment fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn usage_reply_recovery_fences_clones_until_natural_key_is_proven() -> Result<()> {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
        .with_context(|| {
            format!("usage facade recovery fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn candidate_begin_recovery_keeps_clones_fenced_until_exact_ref_reattaches() -> Result<()>
    {
        // Real service lifecycles: one owner, then an owner and its restarted successor.
        let deadline = fixture_deadline(3);
        tokio::time::timeout(deadline, async {
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
        .with_context(|| {
            format!("candidate facade recovery fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_dream_lease_serializes_dreams_without_blocking_ordinary_memory() -> Result<()>
    {
        // Real service lifecycles: one service owner.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
            let _owner_cleanup = AbortOnDrop(served.abort_handle());
            let executable = std::env::current_exe()?;
            let open = || {
                MemoryStore::open_managed_observed(
                    options.clone(),
                    project.clone(),
                    executable.clone(),
                )
                .1
            };
            let first = open().await?;
            let second = open().await?;

            let first_lease = first.acquire_dream_lease().await?;
            let mut waiting = tokio::spawn({
                let second = second.clone();
                async move { second.acquire_dream_lease().await }
            });
            let waiting_cleanup = AbortOnDrop(waiting.abort_handle());
            ensure!(
                tokio::time::timeout(Duration::from_millis(250), &mut waiting)
                    .await
                    .is_err(),
                "a second managed dream acquired the project lease concurrently"
            );
            second
                .append("private/actor", "user", "ordinary write while dreaming")
                .await?;
            ensure!(
                second.history("private/actor", 10).await?.len() == 1,
                "the dream lease blocked an ordinary memory write"
            );

            drop(first_lease);
            let second_lease = tokio::time::timeout(Duration::from_secs(10), &mut waiting)
                .await
                .context("the waiting dream did not acquire after lease release")???;
            drop(waiting_cleanup);

            let mut cancelled = tokio::spawn({
                let first = first.clone();
                async move { first.acquire_dream_lease().await }
            });
            ensure!(
                tokio::time::timeout(Duration::from_millis(250), &mut cancelled)
                    .await
                    .is_err(),
                "the cancellation probe acquired while another dream held the lease"
            );
            cancelled.abort();
            let cancelled = tokio::time::timeout(Duration::from_secs(5), cancelled)
                .await
                .context("the cancelled dream-lease acquisition did not end")?;
            ensure!(
                cancelled.is_err_and(|error| error.is_cancelled()),
                "the dream-lease acquisition completed instead of being cancelled"
            );
            drop(second_lease);

            let final_lease =
                tokio::time::timeout(Duration::from_secs(10), first.acquire_dream_lease())
                    .await
                    .context("cancelled dream-lease acquisition retained owner authority")??;
            drop(final_lease);

            first.close().await?;
            second.close().await?;
            let permit = tokio::time::timeout(
                Duration::from_secs(20),
                service::acquire_maintenance_permit(&options),
            )
            .await
            .context("managed owner did not retire after dream clients closed")??;
            tokio::time::timeout(Duration::from_secs(5), served)
                .await
                .context("managed dream owner did not finish reaping")???;
            drop(permit);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("managed dream-lease fixture exceeded its {deadline:?} deadline")
        })??;
        Ok(())
    }

    #[tokio::test]
    async fn managed_facade_preserves_views_ledger_export_and_independent_clients() -> Result<()> {
        // Real service lifecycles: one service owner. The competing local open
        // is refused by the owner's store lease under its own inner bound.
        let deadline = fixture_deadline(1);
        tokio::time::timeout(deadline, async {
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
            let mut context_summaries = 0;
            let mut context_cursors = 0;
            let mut session_catalog = 0;
            let mut public_turns = 0;
            loop {
                let page = export.page(cursor).await?;
                for record in page.records {
                    match record {
                        StorageRecord::Message { .. } => messages += 1,
                        StorageRecord::State { .. } => state += 1,
                        StorageRecord::ContextSummary { .. } => context_summaries += 1,
                        StorageRecord::ContextCursor { .. } => context_cursors += 1,
                        StorageRecord::SessionCatalog { .. } => session_catalog += 1,
                        StorageRecord::PublicTurn { .. } => public_turns += 1,
                    }
                }
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            export.verify_counts(
                messages,
                state,
                context_summaries,
                context_cursors,
                session_catalog,
                public_turns,
            )?;
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
            let CandidateBackend::Remote(candidate_remote) = &candidate.backend else {
                bail!("managed fixture candidate lost its remote attachment")
            };
            let held_candidate = candidate_remote.view.attachment.lock().await;
            ensure!(
                tokio::time::timeout(Duration::from_millis(50), second.clone().close())
                    .await
                    .is_err(),
                "session close did not wait for its held candidate attachment"
            );
            drop(held_candidate);
            // A cancelled close cannot take and lose the remaining weak
            // attachment registry; this retry must close the candidate too.
            second.close().await?;
            ensure!(
                !candidate_remote
                    .view
                    .attachment
                    .lock()
                    .await
                    .has_complete_exchange(),
                "retry left the previously held candidate transport alive"
            );
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
        .with_context(|| format!("managed facade fixture exceeded its {deadline:?} deadline"))??;
        Ok(())
    }
}
