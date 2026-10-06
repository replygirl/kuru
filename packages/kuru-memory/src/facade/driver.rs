//! One driver's presence attachment, distinct from replaceable SQL streams.

use std::{
    any::Any,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, bail, ensure};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc, oneshot};
use uuid::Uuid;

use super::{Backend, MemoryStore, RemoteSession};
use crate::{
    LiveSessionDriver, SessionDriverOutcome, SessionDriverProof, SessionDriverRefusal,
    SessionDriverRejected, SessionDriverSelection, SessionDriverTarget,
    service::{AttachmentFactory, ServiceAttachment, ServiceCall, ServiceValue},
    session_driver::NativeSessionLease,
    store,
};

/// A conversation driver's owned presence and checked native drain barrier.
/// Clones share this one driver; a separately bound Harness gets a new one.
#[derive(Clone)]
pub struct SessionDriver {
    inner: Arc<DriverInner>,
}

/// A loss observation owns no driver sender, attachment or native barrier.
#[derive(Clone)]
pub struct DriverPresence {
    state: Arc<PresenceState>,
}

#[derive(Default)]
struct PresenceState {
    lost: AtomicBool,
    closing: AtomicBool,
    changed: Notify,
}

impl DriverPresence {
    /// True is unexpected loss; false is deliberate closing. Holding this
    /// observation while idle cannot retain the driver or its transport.
    pub async fn lost(&self) -> bool {
        loop {
            let changed = self.state.changed.notified();
            if self.state.closing.load(Ordering::Acquire) {
                return false;
            }
            if self.state.lost.load(Ordering::Acquire) {
                return true;
            }
            changed.await;
        }
    }
}

struct DriverInner {
    commands: mpsc::UnboundedSender<Command>,
    serial: AsyncMutex<()>,
    state: Mutex<State>,
    data: PathBuf,
    project: PathBuf,
    presence: Arc<PresenceState>,
    proof: Arc<Mutex<Option<SessionDriverProof>>>,
    factory: Option<AttachmentFactory>,
}

impl Drop for DriverInner {
    fn drop(&mut self) {
        // Observers may outlive all clients, but cannot outlive notification
        // of that deliberate end. They retain no sender or ownership guard.
        self.presence.closing.store(true, Ordering::Release);
        self.presence.changed.notify_waiters();
    }
}

#[derive(Default)]
struct State {
    current: Option<Selected>,
    pending: Option<Pending>,
    closed: bool,
}
struct Selected {
    proof: SessionDriverProof,
    lease: Arc<NativeSessionLease>,
}
struct Pending {
    id: Uuid,
    generation: String,
    selection: SessionDriverSelection,
    lease: Arc<NativeSessionLease>,
    completed: Option<Completed>,
}
#[derive(Clone)]
enum Completed {
    Selected(SessionDriverProof),
    NotSelected(SessionDriverProof),
    Rejected(SessionDriverRefusal),
    Uncertain,
}
enum Command {
    Select(oneshot::Sender<()>),
    Recover(oneshot::Sender<()>),
    Close(oneshot::Sender<()>),
    #[cfg(any(test, feature = "test-support"))]
    Disconnect(oneshot::Sender<()>),
    #[cfg(any(test, feature = "test-support"))]
    PauseNextReply(
        crate::test_support::ReplyBarrier,
        oneshot::Sender<Result<()>>,
    ),
}
enum Presence {
    Local {
        store: store::MemoryStore,
        client: Uuid,
        connection: Uuid,
        generation: String,
        claim: Option<store::SessionClaimHandle>,
    },
    Remote {
        attachment: Box<ServiceAttachment>,
        factory: AttachmentFactory,
    },
}

impl MemoryStore {
    /// Bind a conversation driver using this checked store's own data root.
    /// This performs no provider/tool launch and grants no selected session.
    pub async fn bind_project_driver(&self, project: &Path) -> Result<(Self, SessionDriver)> {
        let data = match &self.backend {
            Backend::Local(local) => local.driver_data_root()?.to_path_buf(),
            Backend::Remote(remote) => remote.session.options.data_dir.clone(),
        };
        self.bind_session_driver(&data, project).await
    }

    /// Derive an independently authenticated driver from a settled main view.
    /// The input's uncertain mutation must settle before this new identity can
    /// obtain authority; creating a driver cannot bypass its recovery fence.
    pub async fn bind_session_driver(
        &self,
        data: &Path,
        project: &Path,
    ) -> Result<(Self, SessionDriver)> {
        self.bind_session_driver_identity(data, project, true).await
    }

    async fn bind_session_driver_identity(
        &self,
        data: &Path,
        project: &Path,
        independent: bool,
    ) -> Result<(Self, SessionDriver)> {
        self.reconcile().await?;
        let project = project.canonicalize().context("canonical driver project")?;
        self.ensure_project_scope(&crate::service::canonical_project_scope(&project)?)?;
        let proof = Arc::new(Mutex::new(None));
        let (backend, presence, factory) = match &self.backend {
            Backend::Local(local) => {
                ensure!(
                    local.owns_session_presence(),
                    "driver admission requires writable live memory"
                );
                ensure!(
                    local.driver_data_root()? == data,
                    "driver data directory differs from the checked memory view"
                );
                ensure!(
                    independent,
                    "local owner cannot recover a remote driver identity"
                );
                ensure!(
                    local.pinned_view() == "main",
                    "driver admission requires live memory"
                );
                let client = Uuid::new_v4();
                let presence = Presence::Local {
                    store: local.clone(),
                    client,
                    connection: Uuid::new_v4(),
                    generation: Uuid::new_v4().to_string(),
                    claim: None,
                };
                (
                    Backend::Local(local.with_local_session_caller(client, &proof)),
                    presence,
                    None,
                )
            }
            Backend::Remote(remote) => {
                ensure!(
                    !remote.read_only && remote.candidate.is_none() && remote.pinned_view == "main",
                    "driver admission requires writable live memory"
                );
                ensure!(
                    remote.session.options.data_dir == data,
                    "driver data directory differs from the checked memory view"
                );
                ensure!(
                    project == remote.session.project.canonicalize()?,
                    "driver project differs from the checked memory owner"
                );
                remote.ensure_writable()?;
                let factory = if independent {
                    remote.session.factory.for_new_driver()
                } else {
                    remote.session.factory.clone()
                };
                let attachment = factory.connect().await?;
                let view = RemoteSession::new_view(
                    attachment,
                    remote.session.options.clone(),
                    remote.session.project.clone(),
                    remote.session.executable.clone(),
                )?;
                let mut attachment = factory.connect().await?;
                // Retain an uncertain selection stream until exact outcome
                // recovery transfers the claim to a checked new connection.
                attachment.retain_after_abandon();
                (
                    Backend::Remote(view),
                    Presence::Remote {
                        attachment: Box::new(attachment),
                        factory: factory.clone(),
                    },
                    Some(factory),
                )
            }
        };
        let (commands, receiver) = mpsc::unbounded_channel();
        let inner = Arc::new(DriverInner {
            commands,
            serial: AsyncMutex::new(()),
            state: Mutex::new(State::default()),
            data: data.into(),
            project,
            presence: Arc::new(PresenceState::default()),
            proof,
            factory,
        });
        // No sender or strong DriverInner is captured by its own task. Last
        // client drop closes the receiver and the actual presence transport.
        tokio::spawn(run_presence(Arc::downgrade(&inner), receiver, presence));
        Ok((
            Self {
                backend,
                #[cfg(any(test, feature = "test-support"))]
                reject_next_state_write: self.reject_next_state_write.clone(),
            },
            SessionDriver { inner },
        ))
    }

    pub async fn live_session_drivers(&self) -> Result<Vec<LiveSessionDriver>> {
        match &self.backend {
            Backend::Local(store) => store.live_session_drivers(),
            Backend::Remote(remote) => {
                match remote.call_raw(ServiceCall::LiveSessionDrivers).await? {
                    ServiceValue::LiveSessionDrivers(drivers) => Ok(drivers),
                    _ => bail!("unexpected live session reply"),
                }
            }
        }
    }

    /// Standalone read-only inspection has no live-owner registry evidence.
    /// This metadata grants no driver or lifetime authority.
    pub fn session_presence_known(&self) -> bool {
        match &self.backend {
            Backend::Local(store) => store.owns_session_presence(),
            Backend::Remote(_) => true,
        }
    }
}

impl SessionDriver {
    /// Hold the next presence exchange after its completed owner reply.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn pause_next_selection_reply_for_test(
        &self,
        barrier: &crate::test_support::ReplyBarrier,
    ) -> Result<()> {
        self.ensure_ready()?;
        let (reply, replied) = oneshot::channel();
        self.inner
            .commands
            .send(Command::PauseNextReply(barrier.clone(), reply))
            .map_err(|_| anyhow::anyhow!("presence task ended"))?;
        replied
            .await
            .context("presence reply pause not installed")?
    }

    /// Drop the exact presence transport while retaining its native drain
    /// barrier, so cross-package fixtures can observe real EOF cancellation.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn disconnect_presence_for_test(&self) -> Result<()> {
        let (reply, replied) = oneshot::channel();
        self.inner
            .commands
            .send(Command::Disconnect(reply))
            .map_err(|_| anyhow::anyhow!("presence task ended"))?;
        replied.await.context("presence teardown reply lost")
    }

    /// Unexpected presence loss; deliberate closing is a separate state.
    pub fn is_lost(&self) -> bool {
        self.inner.presence.lost.load(Ordering::Acquire)
            && !self.inner.presence.closing.load(Ordering::Acquire)
    }

    /// After owning operation cancellation/drain, reopen only the retained
    /// store instance. This does not select a session or replay any work.
    /// A new exact selection acquires the real native lock, so outstanding
    /// immutable cleanup holds still produce Draining after the old driver
    /// releases its own reference. Reference counts never authorize reuse.
    pub async fn reopen_after_loss(
        &self,
        memory: &MemoryStore,
    ) -> Result<(MemoryStore, SessionDriver)> {
        ensure!(
            self.inner.presence.lost.load(Ordering::Acquire),
            "driver presence is still live"
        );
        ensure!(
            self.inner
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("driver state unavailable"))?
                .pending
                .is_none(),
            "owner loss left a pending session selection; inspect and close that transition before choosing a session"
        );
        let mut checked_main = memory.reopen_after_checked_recovery().await?;
        let current = checked_main.as_ref().unwrap_or(memory);
        if let Backend::Remote(remote) = &current.backend
            && remote.session.uncertain_write.load(Ordering::Acquire)
        {
            // Actual uncertain receipts need exact recovery. A settled old
            // view must not send a generic read through obsolete authority
            // before opening the checked successor attachment.
            current.reconcile().await?;
            if let Some(rebound) = current.reopen_after_checked_recovery().await? {
                checked_main = Some(rebound);
            }
        }
        let memory = checked_main.as_ref().unwrap_or(memory);
        let Backend::Remote(remote) = &memory.backend else {
            bail!("local memory requires its retained owner, not remote generation recovery");
        };
        remote.session.ensure_mutation_allowed()?;
        let attachment = remote.session.attach_for_recovery().await?;
        let view = RemoteSession::new_view(
            attachment,
            remote.session.options.clone(),
            remote.session.project.clone(),
            remote.session.executable.clone(),
        )?;
        let reopened = MemoryStore {
            backend: Backend::Remote(view),
            #[cfg(any(test, feature = "test-support"))]
            reject_next_state_write: memory.reject_next_state_write.clone(),
        };
        self.close().await?;
        reopened
            .bind_session_driver_identity(&self.inner.data, &self.inner.project, false)
            .await
    }

    pub fn ensure_ready(&self) -> Result<()> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?;
        ensure!(
            !state.closed
                && !self.inner.presence.closing.load(Ordering::Acquire)
                && !self.inner.presence.lost.load(Ordering::Acquire),
            "session driver presence was lost; drain admitted work and recover ownership before continuing"
        );
        ensure!(
            state.pending.is_none(),
            "session selection is pending; reconcile it before continuing"
        );
        let selected = state
            .current
            .as_ref()
            .context("no session driver is selected")?;
        selected.lease.verify()
    }

    pub fn proof(&self) -> Result<Option<SessionDriverProof>> {
        Ok(self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?
            .current
            .as_ref()
            .map(|selected| selected.proof.clone()))
    }

    /// Immutable per-invocation ownership. Retained native cleanup workers
    /// keep this exact session's barrier, even after a later session switch.
    pub fn invocation_hold(&self) -> Result<Arc<dyn Any + Send + Sync>> {
        self.invocation_hold_for(None)
    }

    /// Capture ownership only if this exact selected session is still ready.
    pub fn invocation_hold_for_session(&self, session: &str) -> Result<Arc<dyn Any + Send + Sync>> {
        self.invocation_hold_for(Some(session))
    }

    fn invocation_hold_for(&self, session: Option<&str>) -> Result<Arc<dyn Any + Send + Sync>> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?;
        ensure!(
            !state.closed
                && !self.inner.presence.closing.load(Ordering::Acquire)
                && !self.inner.presence.lost.load(Ordering::Acquire),
            "session driver presence was lost; drain admitted work and recover ownership before continuing"
        );
        ensure!(
            state.pending.is_none(),
            "session selection is pending; reconcile it before continuing"
        );
        let selected = state
            .current
            .as_ref()
            .context("no session driver is selected")?;
        ensure!(
            session.is_none_or(|session| session == selected.proof.session_id),
            "selected session driver changed; no work was admitted"
        );
        selected.lease.verify()?;
        Ok(selected.lease.clone())
    }

    /// Drop-only ownership for draining already admitted work after loss.
    /// This does not prove readiness or authorize another dispatch.
    pub fn retain_for_cleanup(&self) -> Result<Option<Arc<dyn Any + Send + Sync>>> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?;
        Ok(state
            .current
            .as_ref()
            .map(|selected| selected.lease.clone() as Arc<dyn Any + Send + Sync>))
    }

    pub fn presence(&self) -> DriverPresence {
        DriverPresence {
            state: self.inner.presence.clone(),
        }
    }

    /// Prepare the target native barrier before the exact atomic owner switch.
    /// Cancellation retains the complete tuple and both barriers for recovery.
    pub async fn select(&self, target: SessionDriverTarget) -> Result<SessionDriverProof> {
        let _serial = self.inner.serial.lock().await;
        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("driver state unavailable"))?;
            ensure!(
                !state.closed
                    && !self.inner.presence.closing.load(Ordering::Acquire)
                    && !self.inner.presence.lost.load(Ordering::Acquire),
                "driver presence is unavailable"
            );
            ensure!(
                state.pending.is_none(),
                "reconcile the pending session selection first"
            );
            let lease = match &state.current {
                Some(selected) if selected.proof.session_id == target.session_id() => {
                    selected.lease.clone()
                }
                _ => Arc::new(NativeSessionLease::acquire(
                    &self.inner.data,
                    &self.inner.project,
                    target.session_id(),
                )?),
            };
            let expected = state
                .current
                .as_ref()
                .map(|selected| selected.proof.clone());
            state.pending = Some(Pending {
                id: Uuid::new_v4(),
                generation: String::new(),
                selection: SessionDriverSelection { expected, target },
                lease,
                completed: None,
            });
        }
        self.send(false).await?;
        self.finish_selection()
    }

    /// Resolve the exact previous tuple. Unknown evidence stays fenced; it
    /// never creates an inferred successful switch or replays the mutation.
    pub async fn reconcile_selection(&self) -> Result<Option<SessionDriverProof>> {
        let _serial = self.inner.serial.lock().await;
        if self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?
            .pending
            .is_none()
        {
            return Ok(None);
        }
        self.send(true).await?;
        self.finish_selection().map(Some)
    }

    async fn send(&self, recover: bool) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.inner
            .commands
            .send(if recover {
                Command::Recover(sender)
            } else {
                Command::Select(sender)
            })
            .map_err(|_| anyhow::anyhow!("driver presence task ended"))?;
        receiver.await.context("driver selection remains pending")
    }

    fn finish_selection(&self) -> Result<SessionDriverProof> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?;
        let completed = state
            .pending
            .as_ref()
            .and_then(|pending| pending.completed.clone())
            .context("driver selection remains pending")?;
        match completed {
            Completed::Selected(proof) => {
                ensure!(
                    !self.inner.presence.lost.load(Ordering::Acquire),
                    "selected driver presence was lost before publication; recover the exact tuple first"
                );
                let mut published = self
                    .inner
                    .proof
                    .lock()
                    .map_err(|_| anyhow::anyhow!("driver proof state unavailable"))?;
                if let Some(factory) = &self.inner.factory {
                    factory.publish_driver_proof(Some(proof.clone()))?;
                }
                *published = Some(proof.clone());
                let pending = state.pending.take().expect("checked pending selection");
                state.current = Some(Selected {
                    proof: proof.clone(),
                    lease: pending.lease,
                });
                Ok(proof)
            }
            Completed::Rejected(refusal) => {
                state.pending.take();
                Err(SessionDriverRejected(refusal).into())
            }
            Completed::NotSelected(proof) => {
                ensure!(
                    !self.inner.presence.lost.load(Ordering::Acquire),
                    "old driver presence was lost before checked restoration"
                );
                ensure!(
                    state
                        .current
                        .as_ref()
                        .is_some_and(|selected| selected.proof == proof),
                    "unchanged driver proof differs from the retained old selection"
                );
                if let Some(factory) = &self.inner.factory {
                    factory.publish_driver_proof(Some(proof.clone()))?;
                }
                *self
                    .inner
                    .proof
                    .lock()
                    .map_err(|_| anyhow::anyhow!("driver proof state unavailable"))? = Some(proof);
                state.pending.take();
                Err(SessionDriverRejected(SessionDriverRefusal::SelectionNotAccepted).into())
            }
            Completed::Uncertain => {
                bail!("session selection outcome remains uncertain; no driver work was admitted")
            }
        }
    }

    /// Caller invokes this after its actor/tool cleanup has settled. Retained
    /// cleanup may still own immutable invocation holds and keep exclusion.
    pub async fn close(&self) -> Result<()> {
        let _serial = self.inner.serial.lock().await;
        // Fence work and terminate observers before asking for intentional
        // EOF. Terminal closed is published only by the connection owner.
        self.inner.presence.closing.store(true, Ordering::Release);
        self.inner.presence.changed.notify_waiters();
        let terminal = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?
            .closed;
        if !terminal {
            let (sender, receiver) = oneshot::channel();
            self.inner
                .commands
                .send(Command::Close(sender))
                .map_err(|_| anyhow::anyhow!("driver presence task already ended"))?;
            receiver
                .await
                .context("driver presence close interrupted")?;
        }
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("driver state unavailable"))?;
        state.closed = true;
        state.current.take();
        state.pending.take();
        if let Some(factory) = &self.inner.factory {
            factory.publish_driver_proof(None)?;
        }
        *self
            .inner
            .proof
            .lock()
            .map_err(|_| anyhow::anyhow!("driver proof state unavailable"))? = None;
        Ok(())
    }
}

async fn run_presence(
    inner: Weak<DriverInner>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut presence: Presence,
) {
    let mut disconnected = false;
    loop {
        let command = match &mut presence {
            Presence::Remote { attachment, .. }
                if !disconnected && attachment.has_complete_exchange() =>
            {
                tokio::select! {
                    biased;
                    command = commands.recv() => command,
                    _ = attachment.wait_for_presence_loss() => { mark_lost(&inner); disconnected = true; continue; }
                }
            }
            _ => commands.recv().await,
        };
        let Some(command) = command else {
            break;
        };
        let (recover, reply) = match command {
            #[cfg(any(test, feature = "test-support"))]
            Command::PauseNextReply(barrier, reply) => {
                let result = match &mut presence {
                    Presence::Remote { attachment, .. } => {
                        attachment.pause_after_next_send(barrier.inner);
                        Ok(())
                    }
                    Presence::Local { .. } => {
                        Err(anyhow::anyhow!("reply pause requires a managed presence"))
                    }
                };
                let _ = reply.send(result);
                continue;
            }
            #[cfg(any(test, feature = "test-support"))]
            Command::Disconnect(reply) => {
                match &mut presence {
                    Presence::Remote { attachment, .. } => attachment.close(),
                    Presence::Local { claim, .. } => {
                        claim.take();
                    }
                }
                mark_lost(&inner);
                disconnected = true;
                let _ = reply.send(());
                continue;
            }
            Command::Close(reply) => {
                drop(presence);
                if let Some(shared) = inner.upgrade() {
                    shared
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .closed = true;
                }
                let _ = reply.send(());
                return;
            }
            Command::Select(reply) => (false, reply),
            Command::Recover(reply) => (true, reply),
        };
        let Some(shared) = inner.upgrade() else {
            break;
        };
        let tuple = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(pending) = state.pending.as_mut() else {
                let _ = reply.send(());
                continue;
            };
            if pending.generation.is_empty() {
                pending.generation = match &presence {
                    Presence::Local { generation, .. } => generation.clone(),
                    Presence::Remote { attachment, .. } => attachment.generation().into(),
                };
            }
            (
                pending.id,
                pending.generation.clone(),
                pending.selection.clone(),
                pending.lease.clone(),
                pending.completed.clone(),
            )
        };
        drop(shared);
        let (id, generation, selection, _native_hold, completed) = tuple;
        let was_lost = inner
            .upgrade()
            .is_none_or(|inner| inner.presence.lost.load(Ordering::Acquire));
        let result = if recover
            && !was_lost
            && matches!(
                completed,
                Some(Completed::Selected(_) | Completed::NotSelected(_) | Completed::Rejected(_))
            ) {
            completed.expect("checked completed selection")
        } else {
            exchange(&mut presence, id, &generation, &selection, recover).await
        };
        if let Some(shared) = inner.upgrade() {
            if matches!(result, Completed::Uncertain) {
                shared.presence.lost.store(true, Ordering::Release);
                shared.presence.changed.notify_waiters();
            } else if matches!(result, Completed::Selected(_) | Completed::NotSelected(_)) {
                // Only an actual successful exact exchange can restore
                // presence. A previously cached response after EOF cannot.
                shared.presence.lost.store(false, Ordering::Release);
                disconnected = false;
            }
            if let Some(pending) = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending
                .as_mut()
            {
                pending.completed = Some(result);
            }
        }
        let _ = reply.send(());
    }
    mark_lost(&inner);
}

fn mark_lost(inner: &Weak<DriverInner>) {
    if let Some(inner) = inner.upgrade() {
        inner.presence.lost.store(true, Ordering::Release);
        inner.presence.changed.notify_waiters();
    }
}

async fn exchange(
    presence: &mut Presence,
    id: Uuid,
    generation: &str,
    selection: &SessionDriverSelection,
    recover: bool,
) -> Completed {
    match presence {
        Presence::Local {
            store,
            client,
            connection,
            generation: current,
            claim,
        } => {
            if generation != current {
                return Completed::Uncertain;
            }
            if recover {
                return match store
                    .session_driver_outcome(*client, generation, id, selection)
                    .await
                {
                    Ok(SessionDriverOutcome::Selected(proof)) => Completed::Selected(proof),
                    Ok(SessionDriverOutcome::NotSelected(Some(proof))) => {
                        Completed::NotSelected(proof)
                    }
                    _ => Completed::Uncertain,
                };
            }
            match store
                .select_session_driver(*client, *connection, generation, id, selection)
                .await
            {
                Ok(handle) => {
                    let proof = handle.proof().clone();
                    *claim = Some(handle);
                    Completed::Selected(proof)
                }
                Err(error) => rejected(error),
            }
        }
        Presence::Remote {
            attachment,
            factory,
        } => {
            if !recover {
                return match attachment
                    .call_with_id(
                        id,
                        ServiceCall::SelectSessionDriver {
                            selection: selection.clone(),
                        },
                    )
                    .await
                {
                    Ok(ServiceValue::SessionDriver(proof)) => Completed::Selected(proof),
                    Err(error) if attachment.has_definite_mutation_reply() => rejected(error),
                    _ => Completed::Uncertain,
                };
            }
            let Ok(mut checked) = factory.connect().await else {
                return Completed::Uncertain;
            };
            if checked.generation() != generation {
                return Completed::Uncertain;
            }
            let request = ServiceCall::SessionDriverOutcome {
                original_id: id,
                original_generation: generation.into(),
                selection: selection.clone(),
            };
            match checked.call(request).await {
                Ok(ServiceValue::SessionDriverOutcome(
                    outcome @ (SessionDriverOutcome::Selected(_)
                    | SessionDriverOutcome::NotSelected(Some(_))),
                )) => {
                    let (proof, selected) = match outcome {
                        SessionDriverOutcome::Selected(proof) => (proof, true),
                        SessionDriverOutcome::NotSelected(Some(proof)) => (proof, false),
                        _ => unreachable!("checked exact outcome"),
                    };
                    let request = ServiceCall::ReattachSessionDriver {
                        original_id: id,
                        original_generation: generation.into(),
                        selection: selection.clone(),
                    };
                    match checked.call(request).await {
                        Ok(ServiceValue::SessionDriver(transferred)) if transferred == proof => {
                            checked.retain_after_abandon();
                            **attachment = checked;
                            if selected {
                                Completed::Selected(proof)
                            } else {
                                Completed::NotSelected(proof)
                            }
                        }
                        _ => Completed::Uncertain,
                    }
                }
                // Absence or incomplete/mismatched progress cannot establish
                // either successful selection or unchanged old ownership.
                _ => Completed::Uncertain,
            }
        }
    }
}

fn rejected(error: anyhow::Error) -> Completed {
    error
        .downcast_ref::<SessionDriverRejected>()
        .map_or(Completed::Uncertain, |error| {
            Completed::Rejected(error.0.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate as kuru_memory;
    use kuru_core::{Message, Mode};

    #[test]
    fn driver_async_fixtures_use_closing_scope() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        crate::test_support::assert_async_tests_run_in_closing(
            &root,
            &[root.join("facade/driver.rs")],
        );
    }

    async fn target(memory: &MemoryStore, id: &str) -> Result<SessionDriverTarget> {
        Ok(SessionDriverTarget::Catalog(Box::new(
            memory
                .session_catalog_record(id)
                .await?
                .context("missing fixture session")?,
        )))
    }

    #[tokio::test]
    async fn managed_driver_presence_fences_switches_and_unproved_reattachment() -> Result<()> {
        kuru_memory::test_support::closing(async {
            use sha2::{Digest, Sha256};
            use std::time::Duration;

            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let scope = format!(
                "project/{}",
                Sha256::digest(project.as_os_str().as_encoded_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
            let options =
                crate::test_support::warmed_open_options(root.path().join("private"), scope)
                    .await?;
            let deadline = crate::test_support::FixtureDeadline::start(
                Duration::from_secs(90),
                "managed driver presence fixture",
            );
            let result = deadline
                .serve(
                    async |served| {
                        let _spawning = crate::spawn_gate::spawning().await;
                        served.serve(
                            crate::service::ServiceOwner::open(options.clone(), &project).await?,
                        )?;
                        let memory = MemoryStore::open_managed_observed(
                            options.clone(),
                            project.clone(),
                            std::env::current_exe()?,
                        )
                        .1
                        .await?;
                        for id in ["first", "second", "third"] {
                            memory.create_session(id, Mode::Ifs, id).await?;
                        }
                        let (first, driver) = memory.bind_project_driver(&project).await?;
                        let (second, peer) = memory.bind_project_driver(&project).await?;
                        let original = driver.select(target(&memory, "first").await?).await?;
                        peer.select(target(&memory, "second").await?).await?;
                        ensure!(memory.live_session_drivers().await?.len() == 2);
                        first
                            .append_session_message(
                                "actor",
                                "first",
                                &Message::text("user", "FIRST_PRIVATE_SENTINEL"),
                            )
                            .await?;
                        second
                            .append_session_message(
                                "actor",
                                "second",
                                &Message::text("user", "SECOND_PRIVATE_SENTINEL"),
                            )
                            .await?;
                        let revision = memory.revision().await?;
                        ensure!(
                            driver
                                .select(target(&memory, "second").await?)
                                .await
                                .is_err()
                        );
                        ensure!(driver.proof()? == Some(original.clone()));
                        driver.ensure_ready()?;
                        ensure!(memory.revision().await? == revision);
                        ensure!(
                            first
                                .append_session_message(
                                    "actor",
                                    "second",
                                    &Message::text("user", "wrong session")
                                )
                                .await
                                .is_err()
                        );
                        let first_history =
                            first.session_history_window("actor", "first", 8).await?;
                        ensure!(
                            first_history.messages.len() == 1
                                && first_history.messages[0].plain_text()
                                    == Some("FIRST_PRIVATE_SENTINEL")
                        );
                        let stale = target(&memory, "third").await?;
                        let third = memory
                            .session_catalog_record("third")
                            .await?
                            .context("third catalog")?;
                        memory
                            .rename_session("third", third.lifecycle_generation, "changed target")
                            .await?;
                        ensure!(driver.select(stale).await.is_err());
                        driver.ensure_ready()?;
                        let catalog = first
                            .session_catalog_record("first")
                            .await?
                            .context("first catalog")?;
                        first
                            .rename_session("first", catalog.lifecycle_generation, "own rename")
                            .await?;
                        driver.ensure_ready()?;
                        first
                            .append_session_message(
                                "actor",
                                "first",
                                &Message::text("user", "after own rename"),
                            )
                            .await?;
                        // A direct resource transition cannot transfer a claim from a
                        // merely matching tuple without original Completed evidence.
                        let mut unproved = driver
                            .inner
                            .factory
                            .as_ref()
                            .context("managed factory")?
                            .connect()
                            .await?;
                        ensure!(matches!(
                            unproved
                                .call(ServiceCall::ReattachSessionDriver {
                                    original_id: Uuid::new_v4(),
                                    original_generation: original.service_generation.clone(),
                                    selection: SessionDriverSelection {
                                        expected: Some(original.clone()),
                                        target: target(&first, "first").await?
                                    },
                                })
                                .await?,
                            ServiceValue::SessionDriverOutcome(
                                SessionDriverOutcome::StillUncertain
                            )
                        ));
                        unproved.close();
                        driver.ensure_ready()?;
                        ensure!(driver.proof()? == Some(original));
                        ensure!(memory.live_session_drivers().await?.len() == 2);
                        let foreign = root.path().join("foreign");
                        std::fs::create_dir(&foreign)?;
                        ensure!(memory.bind_project_driver(&foreign).await.is_err());
                        let held_cleanup = driver.invocation_hold()?;
                        ensure!(MemoryStore::purge(options.clone()).await.is_err());
                        driver.close().await?;
                        peer.close().await?;
                        ensure!(memory.live_session_drivers().await?.is_empty());
                        first.close().await?;
                        second.close().await?;
                        memory.close().await?;
                        let directory =
                            store::project_directory(&options.data_dir, &options.project_scope)?;
                        let refused = MemoryStore::purge(options.clone()).await.unwrap_err();
                        ensure!(refused.to_string().contains("draining work"));
                        ensure!(
                            directory.exists(),
                            "retained cleanup allowed purge to move the store"
                        );
                        ensure!(
                            !options
                                .data_dir
                                .join("memory/controls")
                                .join(format!(
                                    "{}.json",
                                    options.project_scope.trim_start_matches("project/")
                                ))
                                .exists(),
                            "refused purge wrote durable intent"
                        );
                        drop(held_cleanup);
                        MemoryStore::purge(options.clone()).await?;
                        ensure!(!directory.exists());
                        served
                            .reap(Duration::from_secs(30), "purged driver owner did not reap")
                            .await?;
                        Ok(())
                    },
                    async |served| {
                        served
                            .retire(
                                &options,
                                None,
                                Duration::from_secs(30),
                                "driver owner did not reap",
                            )
                            .await
                    },
                )
                .await;
            root.release(result)
        })
        .await
    }

    #[tokio::test]
    async fn restarted_driver_adopts_identity_for_retained_candidate_checkpoint() -> Result<()> {
        kuru_memory::test_support::closing(async {
            use sha2::{Digest, Sha256};
            use std::time::Duration;

            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let project = project.canonicalize()?;
            let scope = format!("project/{}", Sha256::digest(project.as_os_str().as_encoded_bytes())
                .iter().map(|byte| format!("{byte:02x}")).collect::<String>());
            let options = crate::test_support::warmed_open_options(root.path().join("private"), scope).await?;
            let deadline = crate::test_support::FixtureDeadline::start(Duration::from_secs(90), "driver candidate successor fixture");
            let result = deadline.serve(async |served| {
                let gate = crate::spawn_gate::spawning().await;
                served.serve(crate::service::ServiceOwner::open(options.clone(), &project).await?)?;
                let memory = MemoryStore::open_managed_observed(options.clone(), project.clone(), std::env::current_exe()?).1.await?;
                memory.create_session("selected", Mode::Ifs, "selected").await?;
                let (main, driver) = memory.bind_project_driver(&project).await?;
                memory.close().await?;
                let original = driver.select(target(&main, "selected").await?).await?;
                let untouched = main.begin_candidate("untouched retained candidate").await?;
                let untouched_view = untouched.view();
                let candidate = main.begin_candidate("retained private summary").await?;
                let private = candidate.view();
                let actor = "project/example/ifs/identity/actor";
                private.append_session_message(actor, "selected", &Message::text("user", "PRIVATE_CANDIDATE_SOURCE")).await?;
                let source = private.session_source_snapshot(actor, "selected", actor, 0, 16).await?;
                let record = store::ContextSummaryRecord {
                    actor_namespace: actor.into(), session_id: "selected".into(), source_namespace: actor.into(),
                    summary_namespace: format!("{actor}/session/selected/summaries"),
                    source_view: source.view, source_revision: source.revision,
                    after_sequence: source.after_exclusive, through_sequence: source.through_inclusive.context("missing candidate source boundary")?,
                    turn_id: None, operation_id: Some("compact-retained".into()), producer_actor_id: Some("actor".into()),
                    invocation_id: "retained-invocation".into(), summary: "PRIVATE_CANDIDATE_SUMMARY".into(),
                };
                let checkpoint = store::ContextSummaryCheckpoint { record: record.clone(), private_reasoning: vec![] };
                let Backend::Remote(remote) = &private.backend else { bail!("candidate must be managed") };
                let pause = Arc::new(crate::service::rpc::ReplyPause::default());
                remote.attachment.lock().await.pause_after_next_send(pause.clone());
                {
                    let write = private.checkpoint_context_summary(&checkpoint);
                    tokio::pin!(write);
                    tokio::select! {
                        _ = pause.replied.notified() => {},
                        result = &mut write => { result?; bail!("checkpoint reply escaped its barrier"); }
                    }
                }
                ensure!(main.put("fenced-before-recovery", &serde_json::json!(true)).await.is_err());
                let factory = driver.inner.factory.as_ref().context("missing authenticated factory")?.clone();
                let (reply, replied) = oneshot::channel();
                driver.inner.commands.send(Command::Disconnect(reply)).map_err(|_| anyhow::anyhow!("presence task ended"))?;
                replied.await?;
                ensure!(driver.is_lost());
                main.close_transport_for_test().await?;
                private.close_transport_for_test().await?;
                untouched_view.close_transport_for_test().await?;
                let ((), gate) = crate::spawn_gate::excluding_spawns(gate, async {
                    // Retire only the now-idle owner. The original driver's
                    // native barrier remains held; this is owner replacement,
                    // not authority to perform storage maintenance.
                    loop {
                        let mut retirement = factory.connect().await?;
                        let result = retirement.call(ServiceCall::RetireIfIdle).await?;
                        retirement.close();
                        if matches!(result, ServiceValue::Retirement { accepted: true }) { break; }
                        ensure!(matches!(result, ServiceValue::Retirement { accepted: false }));
                        tokio::task::yield_now().await;
                    }
                    served.reap(Duration::from_secs(30), "old candidate owner did not reap").await?;
                    served.serve(crate::service::ServiceOwner::open(options.clone(), &project).await?)
                }).await?;
                let recovered = private.recover_candidate_unit().await?.context("missing exact candidate checkpoint recovery")?;
                ensure!(recovered.committed);
                let candidate = recovered.candidate;
                let retained = candidate.view();
                ensure!(retained.append_session_message(actor, "selected", &Message::text("user", "before-reclaim")).await.is_err());
                let (fresh, successor) = driver.reopen_after_loss(&main).await?;
                let proof = successor.select(target(&fresh, "selected").await?).await?;
                ensure!(proof.service_generation != original.service_generation);
                let dream_lease = fresh.acquire_dream_lease().await?;
                let observed = fresh.candidate_ref_status(untouched.branch()).await?;
                ensure!(observed.base.as_deref() == Some(untouched.base()));
                let observed_head = observed.head.context("untouched candidate lost its checked head")?;
                let different_head = retained.revision().await?;
                ensure!(different_head != observed_head);
                ensure!(untouched.reattach_checked(&different_head).await.is_err(), "candidate accepted a different head");
                let untouched = untouched.reattach_checked(&observed_head).await?;
                untouched.view().append_session_message(actor, "selected", &Message::text("user", "UNTOUCHED_RECLAIMED_PRIVATE")).await?;
                let confirmation = store::ContextSummaryConfirmation::from_record(&record)?;
                ensure!(retained.context_summary_confirmation(&confirmation.summary_id).await? == Some(confirmation.clone()));
                ensure!(fresh.context_summary_confirmation(&confirmation.summary_id).await?.is_none());
                retained.append_session_message(actor, "selected", &Message::text("user", "after-reclaim")).await?;
                let source = retained.session_source_snapshot(actor, "selected", actor, record.through_sequence, 16).await?;
                retained.checkpoint_context_summary(&store::ContextSummaryCheckpoint {
                    record: store::ContextSummaryRecord {
                        source_view: source.view, source_revision: source.revision,
                        after_sequence: source.after_exclusive,
                        through_sequence: source.through_inclusive.context("missing successor source boundary")?,
                        operation_id: Some("compact-successor".into()), invocation_id: "successor-invocation".into(),
                        summary: "PRIVATE_SUCCESSOR_SUMMARY".into(), ..record
                    }, private_reasoning: vec![],
                }).await?;
                ensure!(fresh.session_history_window(actor, "selected", 16).await?.total_rows == 0);
                candidate.abandon().await?;
                untouched.abandon().await?;
                untouched.view().close().await?;
                drop(dream_lease);
                retained.close().await?;
                main.close().await?;
                // Repeat with no pending receipt: the old primary cannot
                // authenticate a generic Reconcile to the successor owner.
                let factory = successor.inner.factory.as_ref().context("missing successor identity")?.clone();
                let (reply, replied) = oneshot::channel();
                successor.inner.commands.send(Command::Disconnect(reply)).map_err(|_| anyhow::anyhow!("successor presence ended"))?;
                replied.await?;
                fresh.close_transport_for_test().await?;
                let ((), gate) = crate::spawn_gate::excluding_spawns(gate, async {
                    loop {
                        let mut retirement = factory.connect().await?;
                        let outcome = retirement.call(ServiceCall::RetireIfIdle).await?;
                        retirement.close();
                        if matches!(outcome, ServiceValue::Retirement { accepted: true }) { break; }
                        ensure!(matches!(outcome, ServiceValue::Retirement { accepted: false }));
                        tokio::task::yield_now().await;
                    }
                    served.reap(Duration::from_secs(30), "settled old owner did not reap").await?;
                    served.serve(crate::service::ServiceOwner::open(options.clone(), &project).await?)
                }).await?;
                let (third, third_driver) = successor.reopen_after_loss(&fresh).await?;
                let third_proof = third_driver.select(target(&third, "selected").await?).await?;
                ensure!(third_proof.service_generation != proof.service_generation);
                third.append_session_message(actor, "selected", &Message::text("user", "SETTLED_RECLAIMED_PRIVATE")).await?;
                ensure!(third.session_history_window(actor, "selected", 16).await?.total_rows == 1);
                third_driver.close().await?;
                third.close().await?;
                fresh.close().await?;
                drop(gate);
                Ok(())
            }, async |served| {
                served.retire(&options, None, Duration::from_secs(30), "candidate successor owner did not reap").await
            }).await;
            root.release(result)
        }).await
    }

    #[tokio::test]
    async fn checked_existing_instance_refuses_missing_and_replaced_store_before_init() -> Result<()>
    {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let options = crate::test_support::warmed_open_options(
                root.path().join("private"),
                format!("project/{}", "a".repeat(64)),
            )
            .await?;
            let directory = store::project_directory(&options.data_dir, &options.project_scope)?;
            let mut expected = options.clone();
            expected.expected_instance = Some(Uuid::new_v4().to_string());
            ensure!(MemoryStore::open(expected.clone()).await.is_err());
            ensure!(
                !directory.exists(),
                "stale recovery initialized a missing store"
            );
            let memory = MemoryStore::open(options.clone()).await?;
            let revision = memory.revision().await?;
            memory
                .put("retained", &serde_json::json!({"value":"original"}))
                .await?;
            let after = memory.revision().await?;
            ensure!(after != revision);
            memory.close().await?;
            ensure!(MemoryStore::open(expected).await.is_err());
            let reopened = MemoryStore::open(options).await?;
            ensure!(reopened.revision().await? == after);
            ensure!(
                reopened.get("retained").await? == Some(serde_json::json!({"value":"original"}))
            );
            reopened.close().await?;
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn local_driver_bindings_are_distinct_and_retained_work_excludes_reuse() -> Result<()> {
        kuru_memory::test_support::closing(async {
            let root = crate::test_support::tempdir()?;
            let project = root.path().join("project");
            std::fs::create_dir(&project)?;
            let memory = MemoryStore::temporary().await?;
            let Backend::Local(local) = &memory.backend else {
                bail!("local fixture attached remotely")
            };
            let data = local.driver_data_root()?.to_path_buf();
            for id in ["first", "second", "third"] {
                memory.create_session(id, Mode::Ifs, id).await?;
            }
            let revision = memory.revision().await?;
            ensure!(
                memory
                    .bind_session_driver(&root.path().join("wrong-private"), &project)
                    .await
                    .is_err()
            );
            ensure!(memory.revision().await? == revision);
            ensure!(memory.live_session_drivers().await?.is_empty());
            let (first, driver) = memory.bind_session_driver(&data, &project).await?;
            let (second, peer) = memory.bind_session_driver(&data, &project).await?;
            let original = driver.select(target(&memory, "first").await?).await?;
            peer.select(target(&memory, "second").await?).await?;
            ensure!(memory.live_session_drivers().await?.len() == 2);
            first
                .append_session_message("actor", "first", &Message::text("user", "first private"))
                .await?;
            second
                .append_session_message("actor", "second", &Message::text("user", "second private"))
                .await?;
            let revision = memory.revision().await?;
            let refused = first
                .append_session_message("actor", "second", &Message::text("user", "wrong driver"))
                .await
                .unwrap_err();
            ensure!(refused.downcast_ref::<SessionDriverRejected>().is_some());
            ensure!(memory.revision().await? == revision);
            let busy = driver
                .select(target(&memory, "second").await?)
                .await
                .unwrap_err();
            ensure!(matches!(
                busy.downcast_ref::<SessionDriverRejected>()
                    .map(|error| &error.0),
                Some(SessionDriverRefusal::Draining { .. })
            ));
            ensure!(driver.proof()? == Some(original));
            driver.ensure_ready()?;
            let held_work = driver.invocation_hold()?;
            driver.select(target(&memory, "third").await?).await?;
            first
                .append_session_message("actor", "third", &Message::text("user", "new selection"))
                .await?;
            ensure!(
                first
                    .append_session_message(
                        "actor",
                        "first",
                        &Message::text("user", "old selection")
                    )
                    .await
                    .is_err()
            );
            let (_, replacement) = memory.bind_session_driver(&data, &project).await?;
            ensure!(
                replacement
                    .select(target(&memory, "first").await?)
                    .await
                    .is_err(),
                "switch released the old session's retained native work barrier"
            );
            drop(held_work);
            replacement.select(target(&memory, "first").await?).await?;
            replacement.close().await?;
            driver.close().await?;
            peer.close().await?;
            ensure!(memory.live_session_drivers().await?.is_empty());
            memory.close().await?;
            Ok(())
        })
        .await
    }
}
