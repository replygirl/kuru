//! Safe ownership of one standard Unix child and its fresh process group.
//!
//! [`OwnedProcessGroup`] keeps the child's wait identity until it has consumed
//! its initial group-then-root transition and bounded pre-reap settlement. It exposes
//! neither a PID nor the child, so callers cannot reap the root and later
//! signal a recycled numeric group. This is process authority, not sandboxing:
//! processes that leave the fresh group are outside this boundary.
//!
//! Owned spawns create their stdio pipes close-on-exec and start the child
//! under one platform spawn lock, so a concurrent owned child cannot inherit
//! another's pipe ends (std's macOS pipes set close-on-exec in a second step,
//! and posix_spawn and fork copy the descriptor table as it is). The bounded
//! process snapshot spawns under the same lock. Unrelated legacy spawns and
//! other non-atomic descriptor creation can still inherit, or leak into owned
//! children; concurrent callers requiring isolation must use the platform
//! consistently. On std's fork path, a legacy spawn that inherits an owned
//! spawn's exec-error pipe stalls that spawn until the legacy child exits, and
//! later ordinary owned spawns wait behind it; pre-reap snapshots instead
//! use bounded lock admission and return unobserved when it is busy;
//! the remedy is moving those legacy spawns behind the platform, not a
//! timeout.

use rustix::{
    io::Errno,
    process::{
        Pid, Signal, WaitId, WaitIdOptions, geteuid, getuid, kill_process, kill_process_group,
        test_kill_process_group, waitid,
    },
};
#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::{
    fmt, io,
    os::{fd::OwnedFd, unix::process::CommandExt},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub mod snapshot;

/// Orders every platform spawn's descriptor creation against every other
/// platform spawn's descriptor-table copy. Guards no data.
static SPAWN: Mutex<()> = Mutex::new(());

/// Hold the platform spawn lock across native descriptor and child creation only.
/// No caller code, waiting or child I/O runs under it in product builds.
pub(crate) fn spawn_lock() -> MutexGuard<'static, ()> {
    match SPAWN.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            blocked_seam();
            SPAWN.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }
}

/// Start an independently owned child under the same descriptor-copy lock as
/// owned groups. The caller retains and reaps the returned child; this does not
/// give a parent group owner authority to terminate the independent service.
/// Standard input/output are null, and stderr is null or a caller-owned file.
/// No child waiting or caller callback runs under the spawn lock.
pub fn spawn_independent(mut command: Command, stderr: Option<std::fs::File>) -> io::Result<Child> {
    let spawning = spawn_lock();
    command.process_group(0);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(stderr.map_or_else(Stdio::null, Stdio::from));
    let child = command.spawn();
    drop(command);
    drop(spawning);
    child
}

/// Start one separately retained child with lifetime input/output pipes.
/// Pipe creation and descriptor copying stay under the shared platform lock;
/// the caller owns all waiting, lifetime closure and eventual reap.
pub fn spawn_piped(mut command: Command) -> io::Result<Child> {
    let spawning = spawn_lock();
    command.process_group(0);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    let child = command.spawn();
    drop(command);
    drop(spawning);
    child
}

const MAX_EINTR_ATTEMPTS: usize = 8;

/// Why this owner can no longer safely signal its remembered group or root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisarmReason {
    /// The standard child's wait identity was already lost.
    OwnershipLost,
    /// A non-interruption observation error made numeric signal authority
    /// unsafe. The kind is retained without carrying remote/process text.
    ObservationError(io::ErrorKind),
}

/// Non-reaping state of the owned standard root.
#[derive(Debug)]
pub enum RootState {
    Running,
    Exited,
    /// All bounded EINTR attempts were consumed; a later poll may retry.
    Interrupted,
    /// Signal authority is permanently unavailable.
    Disarmed(DisarmReason),
    /// The exact standard-child status was already reaped and cached.
    Reaped(ExitStatus),
}

/// Result for one attempted destructive signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalOutcome {
    Sent,
    AlreadyAbsent,
    PermissionDenied,
    Failed(io::ErrorKind),
}

/// Ordered results for the single destructive transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminationReport {
    /// The original fresh process group is attempted first.
    pub group: SignalOutcome,
    /// The anchored root is attempted second in case it moved groups.
    pub root: SignalOutcome,
}

/// Result of trying to consume the one destructive transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Termination {
    Signalled(TerminationReport),
    /// No signal was attempted; bounded EINTR retained authority.
    Interrupted,
    /// No signal was attempted; authority was permanently disarmed.
    Disarmed(DisarmReason),
    /// A prior transition, reap, or disarm makes signalling unavailable.
    InvalidPhase,
}

/// Result of an exact standard-child reap attempt.
#[derive(Debug)]
pub enum Reap {
    Reaped(ExitStatus),
    NotExited,
    Interrupted,
    Disarmed(DisarmReason),
    /// Reaping is only valid after the destructive transition has begun.
    InvalidPhase,
}

/// One nonblocking pre-reap cleanup poll, never overall cleanup success.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreReap {
    Pending,
    /// A completed listing contained no member except the retained root.
    Ready,
    /// The root was already reaped; continue through read-only final checks.
    Reaped,
    /// No new observation or repeated signal is admitted at this deadline.
    Expired,
    /// The deadline expired and the cancelled read-only helper still owns
    /// cleanup. Retain the owner and report unconfirmed at the caller's limit.
    ExpiredPending,
    /// Listing failed or could not enter the spawn lock; a later poll may retry.
    Unobserved(io::ErrorKind),
    Disarmed(DisarmReason),
    /// The immediate initial termination transition is still required.
    InvalidPhase,
}

struct MembershipJob {
    worker: JoinHandle<io::Result<Vec<snapshot::GroupMember>>>,
    completion: Option<tokio::sync::oneshot::Receiver<()>>,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

/// A read-only process-group presence observation after root reap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupPresence {
    Absent,
    Present,
    /// Only from [`PermissionListing`]: signal zero was refused with `EPERM`,
    /// and the one bounded listing showed only members running as another
    /// user, among them a new leader whose process ID is the group number.
    /// That leader shows the old group emptied before its number was reissued,
    /// so no process of the reaped tree, under any user ID, is in the group.
    /// (`EPERM` followed by an empty listing is [`Self::Absent`].)
    Recycled,
    /// Signal zero was refused with `EPERM`. [`PermissionListing`] keeps this
    /// when the listing showed a member of ours, showed other users' members
    /// without a new leader, or was unavailable.
    PermissionDenied,
    ObservationError(io::ErrorKind),
    /// Group presence is only meaningful after the standard child is reaped.
    InvalidPhase,
}

/// How one standard stream of an owned child is connected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdioSlot {
    /// A pipe the platform creates close-on-exec; the parent end is taken once
    /// through the owner.
    Pipe,
    /// `/dev/null`.
    Null,
    /// The parent's own descriptor.
    Inherit,
}

/// The declared standard streams of one owned child.
///
/// [`OwnedProcessGroup::spawn`] applies this plan to the `Command`, replacing
/// any standard stream the caller configured there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StdioPlan {
    pub stdin: StdioSlot,
    pub stdout: StdioSlot,
    pub stderr: StdioSlot,
}

impl StdioPlan {
    pub const fn new(stdin: StdioSlot, stdout: StdioSlot, stderr: StdioSlot) -> Self {
        Self {
            stdin,
            stdout,
            stderr,
        }
    }
}

#[derive(Debug)]
enum Phase {
    Anchored,
    PostSignal,
    Reaped(ExitStatus),
    Disarmed(DisarmReason),
}

/// A standard child anchored to a fresh Unix process group.
///
/// This value is intentionally non-`Clone` and does not expose its numeric
/// identity or child handle. Callers retain it through pipe capture, make the
/// initial transition with [`Self::terminate_before_reap`], drive
/// [`Self::pre_reap_step`] under their existing deadline, reap the exact root,
/// and then use only [`Self::presence_after_reap`].
pub struct OwnedProcessGroup {
    child: Child,
    stdin: Option<OwnedFd>,
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
    group: Pid,
    phase: Phase,
    observed_exit: bool,
    membership: Option<MembershipJob>,
    #[cfg(test)]
    syscalls: Cell<usize>,
    #[cfg(test)]
    omit_initial_group: bool,
    #[cfg(test)]
    suppress_repeat: bool,
    #[cfg(test)]
    membership_jobs: usize,
}

impl OwnedProcessGroup {
    /// Spawn a standard child in a fresh process group led by that child.
    ///
    /// `stdio` replaces any standard stream configured on `command`. The
    /// platform creates every declared pipe close-on-exec and hands std only
    /// the child's ends, so std creates no stdio pipe of its own.
    pub fn spawn(mut command: Command, stdio: StdioPlan) -> io::Result<Self> {
        command.process_group(0);
        let spawning = spawn_lock();
        let started = launch(&mut command, stdio);
        // The Command owns the child's pipe ends. Close them before returning:
        // a parent copy of its own child's stdout write end would keep that
        // pipe from ever reaching end of file.
        drop(command);
        drop(spawning);
        let (child, [stdin, stdout, stderr]) = started?;
        // rustix reads the standard Child's native identity infallibly. Keep
        // this immediately after spawn: an error return must never drop a
        // newly-created child before it is represented by this owner.
        let group = Pid::from_child(&child);
        Ok(Self {
            child,
            stdin,
            stdout,
            stderr,
            group,
            phase: Phase::Anchored,
            observed_exit: false,
            membership: None,
            #[cfg(test)]
            syscalls: Cell::new(0),
            #[cfg(test)]
            omit_initial_group: false,
            #[cfg(test)]
            suppress_repeat: false,
            #[cfg(test)]
            membership_jobs: 0,
        })
    }

    /// Take the owned standard-input pipe once.
    pub fn take_stdin(&mut self) -> io::Result<ChildStdin> {
        self.stdin
            .take()
            .map(ChildStdin::from)
            .ok_or_else(|| io::Error::other("owned child stdin was not piped"))
    }

    /// Take the owned standard-output pipe once.
    pub fn take_stdout(&mut self) -> io::Result<ChildStdout> {
        self.stdout
            .take()
            .map(ChildStdout::from)
            .ok_or_else(|| io::Error::other("owned child stdout was not piped"))
    }

    /// Take the owned standard-error pipe once.
    pub fn take_stderr(&mut self) -> io::Result<ChildStderr> {
        self.stderr
            .take()
            .map(ChildStderr::from)
            .ok_or_else(|| io::Error::other("owned child stderr was not piped"))
    }

    /// Observe the root without reaping it.
    pub fn root_state(&mut self) -> RootState {
        match &self.phase {
            Phase::Reaped(status) => return RootState::Reaped(*status),
            Phase::Disarmed(reason) => return RootState::Disarmed(*reason),
            Phase::Anchored | Phase::PostSignal => {}
        }
        match self.observe() {
            Observation::Running => RootState::Running,
            Observation::Exited => RootState::Exited,
            Observation::Interrupted => RootState::Interrupted,
            Observation::Disarmed(reason) => RootState::Disarmed(reason),
        }
    }

    /// Consume the one ordered group-then-root destructive transition.
    ///
    /// The root is observed again immediately before either signal. Interrupted
    /// observation sends nothing and leaves an anchored caller able to retry.
    pub fn terminate_before_reap(&mut self) -> Termination {
        if !matches!(self.phase, Phase::Anchored) {
            return match self.phase {
                Phase::Disarmed(reason) => Termination::Disarmed(reason),
                Phase::Anchored => unreachable!(),
                Phase::PostSignal | Phase::Reaped(_) => Termination::InvalidPhase,
            };
        }
        let observation = self.observe();
        let group = self.group;
        #[cfg(test)]
        let omit_group = self.omit_initial_group;
        let transition = take_transition(
            &mut self.phase,
            observation,
            || {
                #[cfg(test)]
                if omit_group {
                    return SignalOutcome::AlreadyAbsent;
                }
                signal_group(group)
            },
            || signal_root(group),
        );
        if matches!(transition, Termination::Signalled(_)) {
            self.record_syscalls(2);
        }
        transition
    }

    /// Advance cleanup without sleeping, root reap, or blocking worker joins.
    ///
    /// The caller drives this inside its existing loop and allowance. Membership
    /// is readiness evidence only: final post-reap absence and pipe checks are
    /// still required. Expiry permits the caller's existing reap fallback, not
    /// another cleanup allowance. A cancelled caller retains this same worker.
    pub fn pre_reap_step(&mut self, deadline: Instant) -> PreReap {
        match self.phase {
            Phase::Anchored => return PreReap::InvalidPhase,
            Phase::Reaped(_) => {
                self.cancel_membership();
                return if self.membership.is_some() {
                    PreReap::Pending
                } else {
                    PreReap::Reaped
                };
            }
            Phase::Disarmed(reason) => {
                self.cancel_membership();
                return PreReap::Disarmed(reason);
            }
            Phase::PostSignal => {}
        }
        if Instant::now() >= deadline {
            self.cancel_membership();
            return if self.membership.is_some() {
                PreReap::ExpiredPending
            } else {
                PreReap::Expired
            };
        }
        // Fresh observation also disarms an owner whose wait identity was
        // lost during a pending read-only job. A running root never starts ps.
        match self.observe() {
            Observation::Running | Observation::Interrupted => return PreReap::Pending,
            Observation::Disarmed(reason) => {
                self.cancel_membership();
                return PreReap::Disarmed(reason);
            }
            Observation::Exited => {}
        }
        if let Some(job) = &self.membership {
            if !job.worker.is_finished() {
                return PreReap::Pending;
            }
            let job = self
                .membership
                .take()
                .expect("finished membership job is retained");
            let result = join_membership(job);
            if Instant::now() >= deadline {
                return PreReap::Expired;
            }
            let members = match result {
                Ok(members) => members,
                Err(error) => return PreReap::Unobserved(error.kind()),
            };
            let root = self.group.as_raw_nonzero().get().unsigned_abs();
            match membership_state(&members, root) {
                Membership::Empty => return PreReap::Ready,
                Membership::Live => return self.resignal_before_reap(deadline),
                Membership::Zombies => {}
            }
            // Descendant zombies remain members until their real parent/init
            // reaps them. Another destructive signal cannot advance that.
            return PreReap::Pending;
        }
        if Instant::now() >= deadline {
            return PreReap::Expired;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = Arc::clone(&cancelled);
        let group = self.group.as_raw_nonzero().get().unsigned_abs();
        let (completed, completion) = tokio::sync::oneshot::channel();
        let started = thread::Builder::new()
            .name("kuru-group-membership".into())
            .spawn(move || {
                let result = snapshot::group_members_until(
                    std::path::Path::new(snapshot::PS),
                    group,
                    deadline,
                    &cancellation,
                );
                // The wake follows helper reap and both reader joins. Result
                // classification and the finished-thread join stay in the owner.
                let _ = completed.send(());
                result
            });
        match started {
            Ok(worker) => {
                self.membership = Some(MembershipJob {
                    worker,
                    completion: Some(completion),
                    cancelled,
                    deadline,
                });
                #[cfg(test)]
                {
                    self.membership_jobs += 1;
                }
                PreReap::Pending
            }
            Err(error) => PreReap::Unobserved(error.kind()),
        }
    }

    /// Wait for the retained worker or the caller's next poll, without taking
    /// its result, joining it, or granting signal authority.
    ///
    /// Cancellation retains the receiver. Completion is only a wake hint;
    /// [`Self::pre_reap_step`] still checks deadlines and joins finished work.
    pub async fn wait_pre_reap(&mut self, poll: Duration, deadline: Instant) {
        let limit = Instant::now()
            .checked_add(poll)
            .map_or(deadline, |next| next.min(deadline));
        if Instant::now() >= limit {
            return;
        }
        let Some(job) = &mut self.membership else {
            tokio::time::sleep_until(limit.into()).await;
            return;
        };
        if job.worker.is_finished() {
            return;
        }
        if let Some(completion) = &mut job.completion {
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(limit.into()) => return,
                _ = completion => job.completion = None,
            }
        }
        // Sending the wake precedes the native thread's finished flag. Yield
        // through that small exit gap within the same poll/deadline, never
        // blocking on a join or turning the wake into readiness.
        while !job.worker.is_finished() && Instant::now() < limit {
            tokio::task::yield_now().await;
        }
    }

    fn cancel_membership(&mut self) {
        if let Some(job) = &self.membership {
            job.cancelled.store(true, Ordering::Release);
            if job.worker.is_finished() {
                let _ =
                    join_membership(self.membership.take().expect("membership job is retained"));
            }
        }
    }

    fn resignal_before_reap(&mut self, deadline: Instant) -> PreReap {
        if !matches!(self.phase, Phase::PostSignal) {
            return PreReap::InvalidPhase;
        }
        if Instant::now() >= deadline {
            return PreReap::Expired;
        }
        // A snapshot, or even observed_exit, never substitutes for this fresh
        // exact wait identity check immediately before repeated destruction.
        let observation = self.observe();
        match observation {
            Observation::Running | Observation::Exited => {}
            Observation::Interrupted => return PreReap::Pending,
            Observation::Disarmed(reason) => return PreReap::Disarmed(reason),
        }
        if Instant::now() >= deadline {
            return PreReap::Expired;
        }
        #[cfg(test)]
        if self.suppress_repeat {
            return PreReap::Pending;
        }
        let group = self.group;
        match repeat_transition(
            &mut self.phase,
            observation,
            || signal_group(group),
            || signal_root(group),
        ) {
            Termination::Signalled(report) => {
                self.record_syscalls(2);
                match (report.group, report.root) {
                    (SignalOutcome::PermissionDenied, _) | (_, SignalOutcome::PermissionDenied) => {
                        PreReap::Unobserved(io::ErrorKind::PermissionDenied)
                    }
                    (SignalOutcome::Failed(kind), _) | (_, SignalOutcome::Failed(kind)) => {
                        PreReap::Unobserved(kind)
                    }
                    // Even ESRCH is not readiness; require a later listing.
                    _ => PreReap::Pending,
                }
            }
            Termination::Interrupted => PreReap::Pending,
            Termination::Disarmed(reason) => PreReap::Disarmed(reason),
            Termination::InvalidPhase => PreReap::InvalidPhase,
        }
    }

    /// Reap only after the destructive transition and an observed root exit.
    pub fn reap_if_exited(&mut self) -> Reap {
        match self.phase {
            Phase::Anchored => return Reap::InvalidPhase,
            Phase::Reaped(status) => return Reap::Reaped(status),
            Phase::Disarmed(reason) => return Reap::Disarmed(reason),
            Phase::PostSignal => {}
        }
        if !self.observed_exit {
            return match self.observe() {
                Observation::Running => Reap::NotExited,
                Observation::Exited => self.reap_observed(),
                Observation::Interrupted => Reap::Interrupted,
                Observation::Disarmed(reason) => Reap::Disarmed(reason),
            };
        }
        self.reap_observed()
    }

    /// Observe group absence after reaping the standard child.
    ///
    /// This performs signal-zero only; it never regains destructive authority.
    pub fn presence_after_reap(&self) -> GroupPresence {
        if !matches!(self.phase, Phase::Reaped(_)) {
            return GroupPresence::InvalidPhase;
        }
        self.record_syscalls(1);
        presence(test_kill_process_group(self.group))
    }

    fn observe(&mut self) -> Observation {
        let (observation, calls) = observe_root_with_count(|| {
            waitid(
                WaitId::Pid(self.group),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .map(|status| status.map(|_| ()))
        });
        self.record_syscalls(calls);
        match observation {
            Observation::Exited => {
                self.observed_exit = true;
                Observation::Exited
            }
            Observation::Disarmed(reason) => {
                self.phase = Phase::Disarmed(reason);
                Observation::Disarmed(reason)
            }
            other => other,
        }
    }

    fn reap_observed(&mut self) -> Reap {
        debug_assert!(self.observed_exit);
        self.record_syscalls(1);
        match self.child.wait() {
            Ok(status) => {
                self.phase = Phase::Reaped(status);
                self.cancel_membership();
                Reap::Reaped(status)
            }
            Err(error) => {
                let reason = if error.raw_os_error() == Some(Errno::CHILD.raw_os_error()) {
                    DisarmReason::OwnershipLost
                } else {
                    DisarmReason::ObservationError(error.kind())
                };
                self.phase = Phase::Disarmed(reason);
                Reap::Disarmed(reason)
            }
        }
    }

    #[cfg(test)]
    fn syscall_count(&self) -> usize {
        self.syscalls.get()
    }

    #[cfg(test)]
    fn record_syscalls(&self, calls: usize) {
        self.syscalls.set(self.syscalls.get().saturating_add(calls));
    }

    #[cfg(not(test))]
    fn record_syscalls(&self, _: usize) {}
}

impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        // This is an emergency request only. Ordinary users retain the value
        // and confirm reap/absence explicitly; Drop cannot make that claim.
        if matches!(self.phase, Phase::Anchored) {
            let _ = self.terminate_before_reap();
        }
        if let Some(job) = &self.membership {
            // Never join even a finished worker in emergency Drop. The worker
            // retains its helper and drains through cleanup on its own thread.
            job.cancelled.store(true, Ordering::Release);
        }
    }
}

fn join_membership(job: MembershipJob) -> io::Result<Vec<snapshot::GroupMember>> {
    let MembershipJob {
        worker,
        cancelled,
        deadline,
        ..
    } = job;
    let result = worker
        .join()
        .map_err(|_| io::Error::other("membership worker panicked"))?;
    if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "membership observation expired",
        ));
    }
    result
}

#[derive(Debug, Eq, PartialEq)]
enum Membership {
    Empty,
    Live,
    Zombies,
}

fn membership_state(members: &[snapshot::GroupMember], root: u32) -> Membership {
    let mut state = Membership::Empty;
    for row in members.iter().filter(|row| row.pid != root) {
        if !row.state.starts_with('Z') {
            return Membership::Live;
        }
        state = Membership::Zombies;
    }
    state
}

/// Create the declared streams and start the child. Returns the parent ends
/// in stdin, stdout, stderr order.
fn launch(command: &mut Command, plan: StdioPlan) -> io::Result<(Child, [Option<OwnedFd>; 3])> {
    let stdin = prepare(plan.stdin, Stream::Input)?;
    let stdout = prepare(plan.stdout, Stream::Output)?;
    let stderr = prepare(plan.stderr, Stream::Output)?;
    command
        .stdin(stdin.child)
        .stdout(stdout.child)
        .stderr(stderr.child);
    let child = command.spawn()?;
    Ok((child, [stdin.parent, stdout.parent, stderr.parent]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stream {
    /// The child reads it (standard input).
    Input,
    /// The child writes it (standard output or error).
    Output,
}

struct Prepared {
    parent: Option<OwnedFd>,
    child: Stdio,
}

fn prepare(slot: StdioSlot, stream: Stream) -> io::Result<Prepared> {
    let (parent, child) = match slot {
        StdioSlot::Null => (None, Stdio::null()),
        StdioSlot::Inherit => (None, Stdio::inherit()),
        StdioSlot::Pipe => {
            let (read, write) = cloexec_pipe(stream)?;
            match stream {
                Stream::Input => (Some(write), Stdio::from(read)),
                Stream::Output => (Some(read), Stdio::from(write)),
            }
        }
    };
    Ok(Prepared { parent, child })
}

/// A pipe whose ends are both close-on-exec before any platform spawn can
/// copy them: the caller holds the platform spawn lock.
#[cfg(not(target_vendor = "apple"))]
fn cloexec_pipe(stream: Stream) -> io::Result<(OwnedFd, OwnedFd)> {
    let ends = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
    window_seam(stream);
    Ok(ends)
}

/// Apple has no `pipe2`: create the pipe, then mark each end close-on-exec as
/// std does. The two steps are not atomic against a concurrent spawn.
#[cfg(target_vendor = "apple")]
fn cloexec_pipe(stream: Stream) -> io::Result<(OwnedFd, OwnedFd)> {
    let (read, write) = rustix::pipe::pipe()?;
    window_seam(stream);
    rustix::io::ioctl_fioclex(&read)?;
    rustix::io::ioctl_fioclex(&write)?;
    Ok((read, write))
}

#[cfg(test)]
type Seam = Option<Box<dyn FnOnce()>>;

#[cfg(test)]
thread_local! {
    /// Fired once, on this thread, while creating a standard-input pipe: on
    /// Apple between `pipe()` and close-on-exec.
    static WINDOW: RefCell<Seam> = const { RefCell::new(None) };
    /// Fired once, on this thread, when the platform spawn lock is already
    /// held, before waiting for it.
    static BLOCKED: RefCell<Seam> = const { RefCell::new(None) };
}

#[cfg(test)]
fn window_seam(stream: Stream) {
    if stream == Stream::Input
        && let Some(seam) = WINDOW.with(|window| window.borrow_mut().take())
    {
        seam();
    }
}

#[cfg(not(test))]
fn window_seam(_: Stream) {}

#[cfg(test)]
fn blocked_seam() {
    if let Some(seam) = BLOCKED.with(|blocked| blocked.borrow_mut().take()) {
        seam();
    }
}

#[cfg(not(test))]
fn blocked_seam() {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Observation {
    Running,
    Exited,
    Interrupted,
    Disarmed(DisarmReason),
}

fn observe_root_with_count(
    mut syscall: impl FnMut() -> rustix::io::Result<Option<()>>,
) -> (Observation, usize) {
    for attempts in 1..=MAX_EINTR_ATTEMPTS {
        match syscall() {
            Ok(None) => return (Observation::Running, attempts),
            Ok(Some(_)) => return (Observation::Exited, attempts),
            Err(Errno::INTR) => continue,
            Err(Errno::CHILD) => {
                return (Observation::Disarmed(DisarmReason::OwnershipLost), attempts);
            }
            Err(error) => {
                return (
                    Observation::Disarmed(DisarmReason::ObservationError(error.kind())),
                    attempts,
                );
            }
        }
    }
    (Observation::Interrupted, MAX_EINTR_ATTEMPTS)
}

fn take_transition(
    phase: &mut Phase,
    observation: Observation,
    group: impl FnOnce() -> SignalOutcome,
    root: impl FnOnce() -> SignalOutcome,
) -> Termination {
    match phase {
        Phase::Disarmed(reason) => Termination::Disarmed(*reason),
        Phase::PostSignal | Phase::Reaped(_) => Termination::InvalidPhase,
        Phase::Anchored => match observation {
            Observation::Running | Observation::Exited => {
                // Consume this initial operation before its first syscall.
                // A later private sweep must obtain fresh wait authority.
                *phase = Phase::PostSignal;
                Termination::Signalled(TerminationReport {
                    group: group(),
                    root: root(),
                })
            }
            Observation::Interrupted => Termination::Interrupted,
            Observation::Disarmed(reason) => {
                *phase = Phase::Disarmed(reason);
                Termination::Disarmed(reason)
            }
        },
    }
}

fn repeat_transition(
    phase: &mut Phase,
    observation: Observation,
    group: impl FnOnce() -> SignalOutcome,
    root: impl FnOnce() -> SignalOutcome,
) -> Termination {
    if let Phase::Disarmed(reason) = phase {
        return Termination::Disarmed(*reason);
    }
    if !matches!(phase, Phase::PostSignal) {
        return Termination::InvalidPhase;
    }
    match observation {
        Observation::Running | Observation::Exited => Termination::Signalled(TerminationReport {
            group: group(),
            root: root(),
        }),
        Observation::Interrupted => Termination::Interrupted,
        Observation::Disarmed(reason) => {
            *phase = Phase::Disarmed(reason);
            Termination::Disarmed(reason)
        }
    }
}

fn signal_group(group: Pid) -> SignalOutcome {
    signal(kill_process_group(group, Signal::KILL))
}

fn signal_root(root: Pid) -> SignalOutcome {
    signal(kill_process(root, Signal::KILL))
}

fn signal(result: rustix::io::Result<()>) -> SignalOutcome {
    match result {
        Ok(()) => SignalOutcome::Sent,
        Err(Errno::SRCH) => SignalOutcome::AlreadyAbsent,
        Err(Errno::PERM) => SignalOutcome::PermissionDenied,
        Err(error) => SignalOutcome::Failed(error.kind()),
    }
}

/// Read-only evidence about a numeric process group whose leader the caller
/// already reaped.
///
/// The group number may by now belong to anything. Nothing here signals
/// anything except signal zero, and no variant authorizes a later signal: a
/// caller that needs to terminate must do so before reaping, through
/// [`OwnedProcessGroup`]. Cleanup loops use [`PermissionListing`] instead;
/// this is a single-shot observation for assertions and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GroupObservation {
    /// Signal zero reported `ESRCH`, or it was refused with `EPERM` and the
    /// following listing showed no member at all. Every process of ours is
    /// listed, so an empty listing is direct evidence that none is in the
    /// group, whatever the reason for the refusal.
    Absent,
    /// A member of ours may survive: signal zero succeeded, or it was refused
    /// with `EPERM` while the listing showed a member whose effective or real
    /// user ID is this process's real or effective user ID. Zombies count as
    /// members. The listing is diagnostic, or the reason it was unavailable.
    Survivors(Result<Vec<snapshot::ProcessRow>, String>),
    /// Signal zero was refused with `EPERM` and the listing showed only
    /// members of other users, among them a new leader whose process ID is
    /// the group number. A new process ID never matches an active process
    /// group ID (POSIX `fork()`), so the old group emptied completely before
    /// the number was reissued: no process of the reaped tree, even one whose
    /// user ID changed, is in it. The rows are diagnostic only.
    Recycled(Vec<snapshot::ProcessRow>),
    /// The group could not be classified: an invalid group number (0 and 1
    /// would address our own group or broadcast), another errno, or `EPERM`
    /// with an unavailable listing or with other users' members but no new
    /// leader. Such members may be descendants of the reaped root that
    /// changed user ID (for example through `sudo`), so they are not
    /// dismissed.
    Unobserved(String),
}

impl GroupObservation {
    /// True only when evidence shows no process of ours is in the group.
    pub fn none_of_ours(&self) -> bool {
        matches!(self, Self::Absent | Self::Recycled(_))
    }
}

impl fmt::Display for GroupObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => write!(formatter, "absent"),
            Self::Survivors(Ok(members)) => {
                write!(formatter, "survivors [{}]", rows(members))
            }
            Self::Survivors(Err(error)) => {
                write!(formatter, "survivors (listing unavailable: {error})")
            }
            Self::Recycled(members) => write!(
                formatter,
                "recycled by another user ({} listed members) [{}]",
                members.len(),
                rows(members)
            ),
            Self::Unobserved(reason) => write!(formatter, "unobserved: {reason}"),
        }
    }
}

fn rows(rows: &[snapshot::ProcessRow]) -> String {
    rows.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// [`OwnedProcessGroup::presence_after_reap`] for a numeric group whose
/// leader the caller already reaped through its own retained child: one
/// signal-zero query and nothing else. Resolve `EPERM` with one
/// [`PermissionListing`] per cleanup. Group numbers 0 and 1, which would
/// address our own group or broadcast, are an observation error. This grants
/// no signal authority.
pub fn group_presence_after_reap(group: u32) -> GroupPresence {
    match reaped_group(group) {
        Some(pid) => presence(test_kill_process_group(pid)),
        None => GroupPresence::ObservationError(io::ErrorKind::InvalidInput),
    }
}

/// Classify numeric process group `group` after its leader was reaped.
///
/// One signal-zero query decides absence. Only when that query is answered,
/// either success or `EPERM`, does one [`snapshot::group_members`] listing
/// run; there is no retry or wait. This grants no signal authority.
pub fn observe_group_after_reap(group: u32) -> GroupObservation {
    let Some(pid) = reaped_group(group) else {
        return GroupObservation::Unobserved(format!("invalid process group {group}"));
    };
    classify_group(test_kill_process_group(pid), group, &own_uids(), || {
        snapshot::group_members(group)
    })
}

/// [`observe_group_after_reap`]'s decision for an already-made signal-zero
/// query. `members` is called at most once, and only when the query was
/// answered with success or `EPERM`.
fn classify_group(
    signal_zero: rustix::io::Result<()>,
    group: u32,
    own_uids: &[u32],
    members: impl FnOnce() -> io::Result<Vec<snapshot::ProcessRow>>,
) -> GroupObservation {
    match signal_zero {
        Err(Errno::SRCH) => GroupObservation::Absent,
        Ok(()) => GroupObservation::Survivors(members().map_err(|error| error.to_string())),
        Err(Errno::PERM) => match members() {
            Ok(members) if members.is_empty() => GroupObservation::Absent,
            Ok(members) if has_own_member(&members, own_uids) => {
                GroupObservation::Survivors(Ok(members))
            }
            Ok(members) if has_new_leader(&members, group) => GroupObservation::Recycled(members),
            Ok(members) => GroupObservation::Unobserved(format!(
                "signal zero refused with EPERM; only other users' members listed, without a new leader pid={group} [{}]",
                rows(&members)
            )),
            Err(error) => GroupObservation::Unobserved(format!(
                "signal zero refused with EPERM; listing unavailable: {error}"
            )),
        },
        Err(error) => GroupObservation::Unobserved(format!("signal zero failed: {error}")),
    }
}

/// A reaped group number that addresses one group: 0 and 1 would address our
/// own group or broadcast.
fn reaped_group(group: u32) -> Option<Pid> {
    i32::try_from(group)
        .ok()
        .filter(|group| *group > 1)
        .and_then(Pid::from_raw)
}

/// This process's real and effective user IDs.
pub fn own_uids() -> [u32; 2] {
    [getuid().as_raw(), geteuid().as_raw()]
}

fn has_own_member(members: &[snapshot::ProcessRow], own_uids: &[u32]) -> bool {
    members
        .iter()
        .any(|row| own_uids.contains(&row.uid) || own_uids.contains(&row.ruid))
}

/// Whether a listed member leads the group under the group's own number.
///
/// Our reaped root was the group's original leader, so a listed process with
/// that process ID is a new process. POSIX `fork()` requires that a new
/// process ID not match any active process group ID (Linux and XNU both keep
/// such a number reserved), so the old group had no member left, of any user,
/// when the number was reissued. Inferred from the standard; not measured.
fn has_new_leader(members: &[snapshot::ProcessRow], group: u32) -> bool {
    members.iter().any(|row| row.pid == group)
}

/// A bounded `ps` listing of one numeric group, given the time it may take.
pub type GroupLister =
    Arc<dyn Fn(u32, Duration) -> io::Result<Vec<snapshot::ProcessRow>> + Send + Sync>;

/// One cleanup's resolution of `EPERM` from post-reap signal zero.
///
/// After the root is reaped, signal zero stays the cheap poll. Signal zero is
/// refused with `EPERM` when no member it reaches may be signalled, which is
/// also what a group number recycled by another user's process looks like.
/// The first [`GroupPresence::PermissionDenied`] passed to [`Self::resolve`] or
/// [`Self::resolve_blocking`] takes one bounded listing of the group's members,
/// limited to [`snapshot::SNAPSHOT_TIMEOUT`] and to the time left before the
/// cleanup deadline; [`Self::resolve`] runs it on a blocking thread so the
/// async executor is never held. The listing yields
/// [`GroupPresence::Absent`] when it lists no member,
/// [`GroupPresence::Recycled`] when every listed member runs as another user
/// and one of them is a new leader whose process ID is the group number, and
/// otherwise leaves `PermissionDenied`, so the caller keeps polling and
/// fails at its unchanged deadline. Every other observation, and every
/// observation after the first listing or once no time is left, is returned
/// unchanged. Nothing here signals anything.
pub struct PermissionListing {
    group: Option<u32>,
    deadline: Instant,
    listed: bool,
    evidence: Option<String>,
    lister: GroupLister,
}

impl fmt::Debug for PermissionListing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PermissionListing")
            .field("group", &self.group)
            .field("deadline", &self.deadline)
            .field("listed", &self.listed)
            .field("evidence", &self.evidence)
            .finish_non_exhaustive()
    }
}

impl PermissionListing {
    /// For the numeric group led by a root the caller itself spawned in a
    /// fresh group and has already reaped. An invalid group number (0, 1 or
    /// out of range) never lists.
    pub fn for_reaped_group(group: u32, deadline: Instant) -> Self {
        Self::with_lister(group, deadline, Arc::new(snapshot::group_members_within))
    }

    /// [`Self::for_reaped_group`] with an explicit lister, for tests.
    pub fn with_lister(group: u32, deadline: Instant, lister: GroupLister) -> Self {
        Self {
            group: reaped_group(group).map(|_| group),
            deadline,
            listed: false,
            evidence: None,
            lister,
        }
    }

    /// Whether this cleanup has taken its one listing.
    pub fn listed(&self) -> bool {
        self.listed
    }

    /// The listing's outcome, as diagnostic text, once it was taken.
    pub fn evidence(&self) -> Option<&str> {
        self.evidence.as_deref()
    }

    /// Resolve `presence`, listing on a blocking thread at most once.
    pub async fn resolve(&mut self, presence: GroupPresence) -> GroupPresence {
        let Some((group, budget)) = self.claim(presence) else {
            return presence;
        };
        let lister = Arc::clone(&self.lister);
        let listed = tokio::task::spawn_blocking(move || lister(group, budget))
            .await
            .unwrap_or_else(|error| Err(io::Error::other(format!("listing task: {error}"))));
        self.settle(group, listed)
    }

    /// [`Self::resolve`] for a caller that already runs off the async executor.
    pub fn resolve_blocking(&mut self, presence: GroupPresence) -> GroupPresence {
        let Some((group, budget)) = self.claim(presence) else {
            return presence;
        };
        let listed = (self.lister)(group, budget);
        self.settle(group, listed)
    }

    fn claim(&mut self, presence: GroupPresence) -> Option<(u32, Duration)> {
        if presence != GroupPresence::PermissionDenied || self.listed {
            return None;
        }
        let group = self.group?;
        let budget = self
            .deadline
            .saturating_duration_since(Instant::now())
            .min(snapshot::SNAPSHOT_TIMEOUT);
        if budget.is_zero() {
            return None;
        }
        self.listed = true;
        Some((group, budget))
    }

    fn settle(
        &mut self,
        group: u32,
        listed: io::Result<Vec<snapshot::ProcessRow>>,
    ) -> GroupPresence {
        let (presence, evidence) = listed_presence(listed, group, &own_uids());
        self.evidence = Some(evidence);
        presence
    }
}

impl OwnedProcessGroup {
    /// The [`PermissionListing`] for this owner's cleanup ending at `deadline`.
    ///
    /// It retains the group number privately for listing only; it is never a
    /// basis for a signal.
    pub fn permission_listing(&self, deadline: Instant) -> PermissionListing {
        PermissionListing::for_reaped_group(
            self.group.as_raw_nonzero().get().unsigned_abs(),
            deadline,
        )
    }
}

/// How one listing taken after `EPERM` resolves a post-reap group.
fn listed_presence(
    listed: io::Result<Vec<snapshot::ProcessRow>>,
    group: u32,
    own_uids: &[u32],
) -> (GroupPresence, String) {
    match listed {
        Ok(members) if members.is_empty() => {
            (GroupPresence::Absent, "EPERM; no member listed".to_owned())
        }
        Ok(members) if has_own_member(&members, own_uids) => (
            GroupPresence::PermissionDenied,
            format!("EPERM; a member of ours is listed [{}]", rows(&members)),
        ),
        Ok(members) if !has_new_leader(&members, group) => (
            GroupPresence::PermissionDenied,
            format!(
                "EPERM; only other users' members listed, without a new leader pid={group} [{}]",
                rows(&members)
            ),
        ),
        Ok(members) => (
            GroupPresence::Recycled,
            format!(
                "EPERM; recycled by another user ({} listed members) [{}]",
                members.len(),
                rows(&members)
            ),
        ),
        Err(error) => (
            GroupPresence::PermissionDenied,
            format!("EPERM; listing unavailable: {error}"),
        ),
    }
}

fn presence(result: rustix::io::Result<()>) -> GroupPresence {
    match result {
        Err(Errno::SRCH) => GroupPresence::Absent,
        Ok(()) => GroupPresence::Present,
        Err(Errno::PERM) => GroupPresence::PermissionDenied,
        Err(error) => GroupPresence::ObservationError(error.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    const INHERITED: StdioPlan =
        StdioPlan::new(StdioSlot::Inherit, StdioSlot::Inherit, StdioSlot::Inherit);
    // The existing native owner fixture allowance, shared by new positive waits.
    pub(super) const TEST_BOUND: Duration = Duration::from_secs(5);

    fn pre_reap_until_ready(
        owner: &mut OwnedProcessGroup,
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            match owner.pre_reap_step(deadline) {
                PreReap::Ready | PreReap::Reaped => return Ok(()),
                PreReap::Pending | PreReap::Unobserved(_) => {}
                result => return Err(format!("pre-reap readiness: {result:?}")),
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn test_row(pid: u32, root: u32, zombie: bool) -> snapshot::GroupMember {
        snapshot::GroupMember {
            pid,
            pgid: root,
            state: if zombie { "Z" } else { "S" }.into(),
        }
    }

    struct WakeFixture {
        owner: OwnedProcessGroup,
        release: Option<std::sync::mpsc::Sender<()>>,
    }

    impl WakeFixture {
        fn new() -> Self {
            let mut command = Command::new("/bin/sleep");
            command.arg("30");
            let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
            owner.terminate_before_reap();
            let limit = Instant::now() + TEST_BOUND;
            while !matches!(owner.root_state(), RootState::Exited) && Instant::now() < limit {
                thread::yield_now();
            }
            let exited = matches!(owner.root_state(), RootState::Exited);
            if !exited {
                finish_test_owner(&mut owner).unwrap();
            }
            assert!(exited, "wake fixture root did not exit");
            Self {
                owner,
                release: None,
            }
        }

        fn install(
            &mut self,
            work: impl FnOnce(
                tokio::sync::oneshot::Sender<()>,
            ) -> io::Result<Vec<snapshot::GroupMember>>
            + Send
            + 'static,
        ) {
            let (completed, completion) = tokio::sync::oneshot::channel();
            self.owner.membership = Some(MembershipJob {
                worker: thread::spawn(move || work(completed)),
                completion: Some(completion),
                cancelled: Arc::new(AtomicBool::new(false)),
                deadline: Instant::now() + TEST_BOUND,
            });
        }
    }

    impl Drop for WakeFixture {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            // Every fixture root was already observed exited. Recover it even
            // if an assertion fails while the read-only worker is gated.
            let _ = self.owner.reap_if_exited();
        }
    }

    #[tokio::test]
    async fn completion_before_wait_is_not_lost_or_consumed_as_readiness() {
        use std::{future::Future, task::Context};

        let mut fixture = WakeFixture::new();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        fixture.release = Some(release);
        fixture.install(move |completed| {
            let _ = gate.recv();
            completed.send(()).unwrap();
            Ok(Vec::new())
        });
        let limit = Instant::now() + TEST_BOUND;
        while !fixture
            .owner
            .membership
            .as_ref()
            .unwrap()
            .worker
            .is_finished()
            && Instant::now() < limit
        {
            // Observe the pending worker before allowing its completion.
            drop(fixture.release.take());
            tokio::task::yield_now().await;
        }
        assert!(
            fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .worker
                .is_finished()
        );
        let mut waiting = Box::pin(fixture.owner.wait_pre_reap(TEST_BOUND, limit));
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
                .is_ready()
        );
        drop(waiting);
        // Waiting never takes the result or reaps the retained root.
        assert!(fixture.owner.membership.is_some());
        assert!(matches!(fixture.owner.root_state(), RootState::Exited));
        assert_eq!(fixture.owner.pre_reap_step(limit), PreReap::Ready);
    }

    #[tokio::test]
    async fn cancelled_completion_wait_retains_the_same_receiver_and_job() {
        use std::{future::Future, task::Context};

        let mut fixture = WakeFixture::new();
        let (release, gate) = std::sync::mpsc::channel();
        fixture.release = Some(release);
        let (entered, entry) = tokio::sync::oneshot::channel();
        fixture.install(move |completed| {
            entered.send(()).unwrap();
            gate.recv().unwrap();
            completed.send(()).unwrap();
            Ok(Vec::new())
        });
        tokio::time::timeout(TEST_BOUND, entry)
            .await
            .unwrap()
            .unwrap();
        let limit = Instant::now() + TEST_BOUND;
        let mut waiting = Box::pin(fixture.owner.wait_pre_reap(TEST_BOUND, limit));
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
        drop(waiting);
        assert!(
            fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .completion
                .is_some()
        );
        fixture.release.take().unwrap().send(()).unwrap();
        tokio::time::timeout(TEST_BOUND, fixture.owner.wait_pre_reap(TEST_BOUND, limit))
            .await
            .unwrap();
        assert_eq!(
            fixture.owner.membership_jobs, 0,
            "resumption admitted another worker"
        );
        assert!(
            fixture.owner.membership.is_some(),
            "wait consumed the listing result"
        );
        assert_eq!(fixture.owner.pre_reap_step(limit), PreReap::Ready);
    }

    #[tokio::test]
    async fn completion_exit_gap_and_expired_event_do_not_join_or_signal() {
        use std::{future::Future, task::Context};

        let mut fixture = WakeFixture::new();
        let (release, gate) = std::sync::mpsc::channel();
        fixture.release = Some(release);
        let (entered, entry) = tokio::sync::oneshot::channel();
        fixture.install(move |completed| {
            // Model the real publication-to-thread-exit gap causally.
            completed.send(()).unwrap();
            entered.send(()).unwrap();
            gate.recv().unwrap();
            Ok(Vec::new())
        });
        tokio::time::timeout(TEST_BOUND, entry)
            .await
            .unwrap()
            .unwrap();
        // An already-published wake cannot outrun an expired caller limit.
        fixture
            .owner
            .wait_pre_reap(TEST_BOUND, Instant::now())
            .await;
        assert!(
            fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .completion
                .is_some()
        );
        let limit = Instant::now() + TEST_BOUND;
        let mut waiting = Box::pin(fixture.owner.wait_pre_reap(TEST_BOUND, limit));
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
        drop(waiting);
        assert!(
            fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .completion
                .is_none()
        );
        assert!(
            !fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .worker
                .is_finished()
        );
        let calls = fixture.owner.syscall_count();
        assert_eq!(fixture.owner.pre_reap_step(limit), PreReap::Pending);
        assert_eq!(
            fixture.owner.syscall_count(),
            calls + 1,
            "completion signalled or reaped the root"
        );
        // Completion and expiry are both visible: expiry wins and cancels,
        // while an unfinished join remains forbidden.
        let expired = Instant::now();
        fixture.owner.wait_pre_reap(TEST_BOUND, expired).await;
        let calls = fixture.owner.syscall_count();
        assert_eq!(
            fixture.owner.pre_reap_step(expired),
            PreReap::ExpiredPending
        );
        assert_eq!(fixture.owner.syscall_count(), calls);
        assert!(
            fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .cancelled
                .load(Ordering::Acquire)
        );
    }

    #[tokio::test]
    async fn disconnected_completion_wait_exposes_worker_panic_without_readiness() {
        let mut fixture = WakeFixture::new();
        fixture.install(|_completed| panic!("injected membership worker panic"));
        let limit = Instant::now() + TEST_BOUND;
        tokio::time::timeout(TEST_BOUND, fixture.owner.wait_pre_reap(TEST_BOUND, limit))
            .await
            .unwrap();
        assert!(
            fixture
                .owner
                .membership
                .as_ref()
                .unwrap()
                .worker
                .is_finished()
        );
        assert_eq!(
            fixture.owner.pre_reap_step(limit),
            PreReap::Unobserved(io::ErrorKind::Other)
        );
        assert!(matches!(fixture.owner.root_state(), RootState::Exited));
    }

    #[test]
    fn real_second_sweep_removes_omitted_descendant_before_root_reap() {
        for suppress in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let ready = root.path().join("descendant-ready");
            let mut command = Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "(printf ready > \"$1\"; exec /bin/sleep 30) & wait",
                    "kuru-second-sweep",
                ])
                .arg(&ready);
            let mut owner = OwnedProcessGroup::spawn(
                command,
                StdioPlan::new(StdioSlot::Null, StdioSlot::Null, StdioSlot::Null),
            )
            .unwrap();
            let observation = (|| -> Result<(), String> {
                let limit = Instant::now() + TEST_BOUND;
                while !ready.exists() {
                    if Instant::now() >= limit {
                        return Err("descendant did not publish readiness".into());
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                owner.omit_initial_group = true;
                owner.suppress_repeat = suppress;
                if !matches!(owner.terminate_before_reap(), Termination::Signalled(_)) {
                    return Err("initial root signal missing".into());
                }
                while !matches!(owner.root_state(), RootState::Exited) {
                    if Instant::now() >= limit {
                        return Err("root did not exit".into());
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                let group = owner.group.as_raw_nonzero().get().unsigned_abs();
                let remaining = || {
                    snapshot::group_members(group)
                        .map(|rows| {
                            rows.into_iter()
                                .filter(|row| row.pid != group && !row.state.starts_with('Z'))
                                .collect::<Vec<_>>()
                        })
                        .map_err(|error| error.to_string())
                };
                if remaining()?.is_empty() {
                    return Err("omitted descendant was not alive after first sweep".into());
                }
                let allowance = if suppress {
                    Duration::from_millis(100)
                } else {
                    TEST_BOUND
                };
                let result = pre_reap_until_ready(&mut owner, Instant::now() + allowance);
                // Both controls use this same physical assertion while the
                // root's exact wait identity is still retained.
                if !remaining()?.is_empty() {
                    return Err("live descendant survived pre-reap settlement".into());
                }
                result?;
                if !matches!(owner.phase, Phase::PostSignal) {
                    return Err("root reaped before the second sweep settled".into());
                }
                Ok(())
            })();
            // Restore the real path before any assertion, including negative
            // control failure. Never rescue through recorded numeric rows.
            owner.suppress_repeat = false;
            let cleanup = finish_test_owner(&mut owner);
            if let Err(error) = cleanup {
                thread::spawn(move || {
                    while finish_test_owner(&mut owner).is_err() {
                        thread::sleep(Duration::from_millis(10));
                    }
                });
                panic!("second sweep fixture cleanup: {error}");
            }
            if suppress {
                assert_eq!(
                    observation.unwrap_err(),
                    "live descendant survived pre-reap settlement"
                );
                eprintln!(
                    "negative control: live descendant survived suppressed second sweep; real cleanup restored before assertion"
                );
            } else {
                observation.unwrap();
            }
        }
    }

    #[test]
    fn membership_excludes_only_root_and_keeps_all_other_members_and_zombies() {
        let root = test_row(40, 40, true);
        assert_eq!(
            membership_state(std::slice::from_ref(&root), 40),
            Membership::Empty
        );
        let zombie = test_row(41, 40, true);
        assert_eq!(
            membership_state(&[root.clone(), zombie.clone()], 40),
            Membership::Zombies
        );
        let live = test_row(42, 40, false);
        assert_eq!(
            membership_state(&[root, zombie, live], 40),
            Membership::Live
        );
    }

    #[test]
    fn completed_jobs_do_not_cache_readiness_or_treat_zombies_and_failures_as_ready() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
        let observed = (|| -> Result<(), String> {
            owner.terminate_before_reap();
            let limit = Instant::now() + TEST_BOUND;
            while !matches!(owner.root_state(), RootState::Exited) && Instant::now() < limit {
                thread::yield_now();
            }
            let group = owner.group.as_raw_nonzero().get().unsigned_abs();
            let completed = [
                (
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "malformed listing",
                    )),
                    PreReap::Unobserved(io::ErrorKind::InvalidData),
                    1,
                ),
                (
                    Ok(vec![test_row(group + 1, group, true)]),
                    PreReap::Pending,
                    1,
                ),
                (Ok(Vec::new()), PreReap::Ready, 1),
                // A member in the next observation after empty readiness must
                // cause a new fresh sweep rather than use an empty cache.
                (
                    Ok(vec![test_row(group + 2, group, false)]),
                    PreReap::Pending,
                    4,
                ),
            ];
            for (rows, expected, expected_calls) in completed {
                let worker = thread::spawn(move || rows);
                while !worker.is_finished() && Instant::now() < limit {
                    thread::yield_now();
                }
                owner.membership = Some(MembershipJob {
                    worker,
                    completion: None,
                    cancelled: Arc::new(AtomicBool::new(false)),
                    deadline: limit,
                });
                let before = owner.syscall_count();
                let state = owner.pre_reap_step(limit);
                // macOS can refuse a signal to the retained root zombie with
                // EPERM. That remains unobserved, never readiness.
                if state != expected
                    && !(expected_calls == 4
                        && state == PreReap::Unobserved(io::ErrorKind::PermissionDenied))
                {
                    return Err(format!(
                        "completed job expected {expected:?}, got {state:?}"
                    ));
                }
                let calls = owner.syscall_count() - before;
                if calls != expected_calls {
                    return Err(format!("unexpected job syscall count: {calls}"));
                }
            }
            Ok(())
        })();
        finish_test_owner(&mut owner).unwrap();
        observed.unwrap();
    }

    #[test]
    fn repeated_sweep_checks_fresh_errors_and_never_rearms_disarm() {
        for errno in [Errno::CHILD, Errno::IO] {
            let mut phase = Phase::PostSignal;
            let observation = observe_root_with_count(|| Err(errno)).0;
            assert!(matches!(
                repeat_transition(
                    &mut phase,
                    observation,
                    || panic!("group signal after ownership loss"),
                    || panic!("root signal after ownership loss")
                ),
                Termination::Disarmed(_)
            ));
            assert!(matches!(
                repeat_transition(
                    &mut phase,
                    Observation::Exited,
                    || panic!("group rearmed"),
                    || panic!("root rearmed")
                ),
                Termination::Disarmed(_)
            ));
        }
        let mut phase = Phase::PostSignal;
        let (observation, calls) = observe_root_with_count(|| Err(Errno::INTR));
        assert_eq!(calls, MAX_EINTR_ATTEMPTS);
        assert_eq!(
            repeat_transition(
                &mut phase,
                observation,
                || panic!("group after EINTR"),
                || panic!("root after EINTR")
            ),
            Termination::Interrupted
        );
        assert_eq!(
            repeat_transition(
                &mut phase,
                Observation::Exited,
                || SignalOutcome::PermissionDenied,
                || SignalOutcome::Sent
            ),
            Termination::Signalled(TerminationReport {
                group: SignalOutcome::PermissionDenied,
                root: SignalOutcome::Sent
            })
        );
        let mut phase = Phase::Anchored;
        assert_eq!(
            repeat_transition(
                &mut phase,
                Observation::Exited,
                || unreachable!(),
                || unreachable!()
            ),
            Termination::InvalidPhase
        );
    }

    #[tokio::test]
    async fn pre_reap_spawn_lock_contention_keeps_timers_and_deadline_responsive() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
        assert!(matches!(
            owner.terminate_before_reap(),
            Termination::Signalled(_)
        ));
        let (held, holding) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let locker = thread::spawn(move || {
            let guard = spawn_lock();
            let _ = held.send(());
            let _ = wait.recv_timeout(TEST_BOUND);
            drop(guard);
        });
        let held = tokio::time::timeout(TEST_BOUND, holding).await;
        let admission_limit = Instant::now() + TEST_BOUND;
        // First prove busy admission causally, under the normal fixture bound.
        let mut unobserved = false;
        let mut ticks = 0;
        loop {
            match owner.pre_reap_step(admission_limit) {
                PreReap::Expired => break,
                PreReap::Unobserved(io::ErrorKind::WouldBlock) => {
                    unobserved = true;
                    break;
                }
                PreReap::Pending => {}
                other => panic!("unexpected lock contention state: {other:?}"),
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
            ticks += 1;
        }
        // This is a short expiry stimulus, not a positive scheduler allowance.
        let deadline = Instant::now() + Duration::from_millis(20);
        while owner.pre_reap_step(deadline) != PreReap::Expired && Instant::now() < admission_limit
        {
            tokio::time::sleep(Duration::from_millis(2)).await;
            ticks += 1;
        }
        let _ = release.send(());
        locker.join().unwrap();
        let jobs = owner.membership_jobs;
        assert!(matches!(
            owner.pre_reap_step(deadline),
            PreReap::Expired | PreReap::ExpiredPending
        ));
        assert_eq!(owner.membership_jobs, jobs, "expiry admitted a late helper");
        finish_test_owner(&mut owner).unwrap();
        assert!(
            matches!(held, Ok(Ok(()))),
            "lock admission fixture failed: {held:?}"
        );
        assert!(
            unobserved && ticks > 0,
            "unobserved={unobserved}, ticks={ticks}"
        );
    }

    #[test]
    fn pending_job_is_retained_on_resume_and_late_result_cannot_signal() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
        assert!(matches!(
            owner.terminate_before_reap(),
            Termination::Signalled(_)
        ));
        let root_limit = Instant::now() + TEST_BOUND;
        while !matches!(owner.root_state(), RootState::Exited) && Instant::now() < root_limit {
            thread::yield_now();
        }
        let exited = matches!(owner.root_state(), RootState::Exited);
        if !exited {
            finish_test_owner(&mut owner).unwrap();
        }
        assert!(exited, "root did not exit within the fixture allowance");
        let (release, wait) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let group = owner.group.as_raw_nonzero().get().unsigned_abs();
        let deadline = Instant::now() + Duration::from_millis(20);
        owner.membership = Some(MembershipJob {
            worker: thread::spawn(move || {
                wait.recv().unwrap();
                Ok(vec![test_row(group + 1, group, false)])
            }),
            completion: None,
            cancelled: cancelled.clone(),
            deadline,
        });
        let jobs = owner.membership_jobs;
        for _ in 0..4 {
            assert_eq!(owner.pre_reap_step(deadline), PreReap::Pending);
        }
        assert_eq!(
            owner.membership_jobs, jobs,
            "resumption duplicated a pending job"
        );
        while Instant::now() < deadline {
            thread::yield_now();
        }
        assert_eq!(owner.pre_reap_step(deadline), PreReap::ExpiredPending);
        assert!(cancelled.load(Ordering::Acquire));
        let before_reap = owner.syscall_count();
        assert_eq!(owner.resignal_before_reap(deadline), PreReap::Expired);
        assert_eq!(
            owner.syscall_count(),
            before_reap,
            "expiry admitted another destructive attempt"
        );
        assert!(matches!(owner.reap_if_exited(), Reap::Reaped(_)));
        let after_reap = owner.syscall_count();
        assert_eq!(after_reap, before_reap + 1);
        release.send(()).unwrap();
        let limit = Instant::now() + TEST_BOUND;
        while owner
            .membership
            .as_ref()
            .is_some_and(|job| !job.worker.is_finished())
            && Instant::now() < limit
        {
            thread::yield_now();
        }
        assert_eq!(owner.pre_reap_step(limit), PreReap::Reaped);
        assert_eq!(
            owner.syscall_count(),
            after_reap,
            "late result issued a product-tree syscall"
        );
        assert!(owner.membership.is_none());
        finish_test_owner(&mut owner).unwrap();
    }

    #[test]
    fn simple_exited_root_uses_one_job_and_running_root_uses_none() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
        assert_eq!(owner.pre_reap_step(Instant::now()), PreReap::InvalidPhase);
        // Represents a failed initial attempt against a still-running root.
        owner.phase = Phase::PostSignal;
        let limit = Instant::now() + TEST_BOUND;
        assert_eq!(owner.pre_reap_step(limit), PreReap::Pending);
        assert_eq!(owner.membership_jobs, 0);
        owner.resignal_before_reap(limit);
        pre_reap_until_ready(&mut owner, limit).unwrap();
        assert_eq!(owner.membership_jobs, 1);
        // Readiness is consumed, not cached if a caller pauses before reap.
        assert_eq!(owner.pre_reap_step(limit), PreReap::Pending);
        assert_eq!(owner.membership_jobs, 2);
        finish_test_owner(&mut owner).unwrap();
    }

    fn close_on_exec(fd: &OwnedFd) -> bool {
        rustix::io::fcntl_getfd(fd)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    }

    #[test]
    fn platform_pipes_are_close_on_exec() {
        for stream in [Stream::Input, Stream::Output] {
            let (read, write) = cloexec_pipe(stream).unwrap();
            assert!(close_on_exec(&read), "{stream:?} read end");
            assert!(close_on_exec(&write), "{stream:?} write end");
        }
        let mut command = Command::new("/bin/cat");
        command.env_clear();
        let plan = StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Null);
        let mut owner = OwnedProcessGroup::spawn(command, plan).unwrap();
        let held =
            [owner.stdin.as_ref(), owner.stdout.as_ref()].map(|end| end.is_some_and(close_on_exec));
        assert!(owner.stderr.is_none());
        assert!(owner.take_stderr().is_err(), "a null stream is not piped");
        drop(owner.take_stdin().unwrap());
        let finished = finish_test_owner(&mut owner);
        assert_eq!(held, [true, true], "parent ends carry close-on-exec");
        finished.unwrap();
    }

    #[test]
    fn signal_and_post_reap_presence_keep_absence_and_permission_distinct() {
        assert_eq!(signal(Err(Errno::SRCH)), SignalOutcome::AlreadyAbsent);
        assert_eq!(signal(Err(Errno::PERM)), SignalOutcome::PermissionDenied);
        assert_eq!(presence(Err(Errno::SRCH)), GroupPresence::Absent);
        assert_eq!(presence(Err(Errno::PERM)), GroupPresence::PermissionDenied);
    }

    fn member(pid: u32, uid: u32, ruid: u32, state: &str) -> snapshot::ProcessRow {
        snapshot::ProcessRow {
            pid,
            ppid: 1,
            pgid: 40,
            uid,
            ruid,
            state: state.to_owned(),
            cpu_time: "0:00.00".to_owned(),
            rss_kib: 1,
            command: format!("process {pid}"),
        }
    }

    #[test]
    fn group_classification_requires_evidence_before_ignoring_permission() {
        let own = [501, 501];
        let unlisted = || -> io::Result<Vec<snapshot::ProcessRow>> {
            panic!("absence and other errors must not list")
        };
        assert_eq!(
            classify_group(Err(Errno::SRCH), 40, &own, unlisted),
            GroupObservation::Absent
        );
        assert!(matches!(
            classify_group(Err(Errno::INVAL), 40, &own, unlisted),
            GroupObservation::Unobserved(reason) if reason.contains("signal zero failed")
        ));

        let foreign = vec![member(40, 0, 0, "Ss"), member(41, 88, 88, "S")];
        let recycled = classify_group(Err(Errno::PERM), 40, &own, || Ok(foreign.clone()));
        assert_eq!(recycled, GroupObservation::Recycled(foreign));
        assert!(recycled.none_of_ours());
        assert!(recycled.to_string().contains("recycled by another user (2"));
        // Other users' members without a new leader may be descendants of the
        // reaped root whose user ID changed: never dismissed as recycled.
        let leaderless = vec![member(41, 0, 0, "S"), member(42, 88, 88, "S")];
        let ambiguous = classify_group(Err(Errno::PERM), 40, &own, || Ok(leaderless.clone()));
        assert!(
            matches!(&ambiguous, GroupObservation::Unobserved(reason)
                if reason.contains("without a new leader pid=40") && reason.contains("pid=41 ")),
            "{ambiguous}"
        );
        assert!(!ambiguous.none_of_ours());
        // Members that exited after the refusal, or hidden foreign members,
        // leave nothing listed and nothing of ours.
        assert_eq!(
            classify_group(Err(Errno::PERM), 40, &own, || Ok(Vec::new())),
            GroupObservation::Absent
        );

        // A member running as us, by effective or real ID and even as a
        // zombie, is never dismissed as recycled.
        for ours in [
            member(42, 501, 0, "S"),
            member(42, 0, 501, "S"),
            member(42, 501, 501, "Z"),
        ] {
            let rows = vec![member(40, 0, 0, "Ss"), ours];
            let observed = classify_group(Err(Errno::PERM), 40, &own, || Ok(rows.clone()));
            assert_eq!(observed, GroupObservation::Survivors(Ok(rows)));
            assert!(!observed.none_of_ours());
            assert!(observed.to_string().contains("pid=42 "), "{observed}");
        }
        let unavailable = classify_group(Err(Errno::PERM), 40, &own, || {
            Err(io::Error::other("ps failed"))
        });
        assert!(!unavailable.none_of_ours());
        assert!(
            unavailable
                .to_string()
                .contains("listing unavailable: ps failed")
        );

        let live = classify_group(Ok(()), 40, &own, || Err(io::Error::other("ps failed")));
        assert!(matches!(&live, GroupObservation::Survivors(Err(error)) if error == "ps failed"));
        assert!(!live.none_of_ours());
    }

    #[test]
    fn owned_listing_resolves_permission_only_with_foreign_evidence() {
        let own = [501, 501];
        let foreign = listed_presence(Ok(vec![member(40, 0, 0, "Ss")]), 40, &own);
        assert_eq!(foreign.0, GroupPresence::Recycled);
        assert!(
            foreign.1.contains("recycled by another user (1"),
            "{}",
            foreign.1
        );
        let leaderless = listed_presence(
            Ok(vec![member(41, 0, 0, "S"), member(42, 88, 88, "S")]),
            40,
            &own,
        );
        assert_eq!(leaderless.0, GroupPresence::PermissionDenied);
        assert!(
            leaderless.1.contains("without a new leader pid=40")
                && leaderless.1.contains("pid=42 "),
            "{}",
            leaderless.1
        );
        let empty = listed_presence(Ok(Vec::new()), 40, &own);
        assert_eq!(empty.0, GroupPresence::Absent);
        assert!(empty.1.contains("no member listed"));
        let ours = listed_presence(
            Ok(vec![member(40, 0, 0, "Ss"), member(41, 501, 501, "Z")]),
            40,
            &own,
        );
        assert_eq!(ours.0, GroupPresence::PermissionDenied);
        assert!(ours.1.contains("pid=41 "), "{}", ours.1);
        let unavailable = listed_presence(Err(io::Error::other("ps")), 40, &own);
        assert_eq!(unavailable.0, GroupPresence::PermissionDenied);
        assert!(unavailable.1.contains("listing unavailable: ps"));
    }

    fn counting_lister(
        calls: &Arc<std::sync::atomic::AtomicUsize>,
        budgets: &Arc<std::sync::Mutex<Vec<Duration>>>,
        rows: Vec<snapshot::ProcessRow>,
    ) -> GroupLister {
        let calls = Arc::clone(calls);
        let budgets = Arc::clone(budgets);
        Arc::new(move |group, budget| {
            assert_eq!(group, 40);
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            budgets.lock().unwrap().push(budget);
            Ok(rows.clone())
        })
    }

    #[test]
    fn permission_listing_lists_once_within_the_cleanup_budget() {
        let calls = Arc::default();
        let budgets = Arc::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        // The listing classifies against this process's real IDs, so the
        // fixtures are built from them rather than from a host's usual uid.
        let own = own_uids();
        let [ruid, euid] = own;
        let foreign = (0..).find(|uid| !own.contains(uid)).unwrap();
        let mut listing = PermissionListing::with_lister(
            40,
            deadline,
            counting_lister(&calls, &budgets, vec![member(40, euid, ruid, "Z")]),
        );
        assert!(!listing.listed());
        assert!(format!("{listing:?}").contains("listed: false"));
        // Signal-zero answers other than EPERM never list.
        for presence in [
            GroupPresence::Absent,
            GroupPresence::Present,
            GroupPresence::Recycled,
            GroupPresence::ObservationError(io::ErrorKind::Other),
            GroupPresence::InvalidPhase,
        ] {
            assert_eq!(listing.resolve_blocking(presence), presence);
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        // Repeated EPERM over one cleanup lists exactly once.
        for _ in 0..5 {
            assert_eq!(
                listing.resolve_blocking(GroupPresence::PermissionDenied),
                GroupPresence::PermissionDenied
            );
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(listing.listed());
        assert!(
            listing
                .evidence()
                .is_some_and(|text| text.contains("a member of ours")),
            "{listing:?}"
        );
        // The listing never outlasts the snapshot bound, even with a distant
        // cleanup deadline.
        assert_eq!(*budgets.lock().unwrap(), [snapshot::SNAPSHOT_TIMEOUT]);

        // A near deadline bounds the listing to the time left.
        let budgets = Arc::default();
        let mut near = PermissionListing::with_lister(
            40,
            Instant::now() + Duration::from_millis(300),
            counting_lister(&calls, &budgets, vec![member(40, foreign, foreign, "Ss")]),
        );
        assert_eq!(
            near.resolve_blocking(GroupPresence::PermissionDenied),
            GroupPresence::Recycled
        );
        let budget = budgets.lock().unwrap()[0];
        assert!(budget <= Duration::from_millis(300), "{budget:?}");
    }

    #[test]
    fn permission_listing_skips_spent_budgets_and_invalid_groups() {
        let calls = Arc::default();
        let budgets = Arc::default();
        let mut spent = PermissionListing::with_lister(
            40,
            Instant::now(),
            counting_lister(&calls, &budgets, Vec::new()),
        );
        assert_eq!(
            spent.resolve_blocking(GroupPresence::PermissionDenied),
            GroupPresence::PermissionDenied
        );
        assert!(!spent.listed());
        assert_eq!(spent.evidence(), None);
        for group in [0, 1, u32::MAX] {
            let mut invalid = PermissionListing::with_lister(
                group,
                Instant::now() + Duration::from_secs(60),
                counting_lister(&calls, &budgets, Vec::new()),
            );
            assert_eq!(
                invalid.resolve_blocking(GroupPresence::PermissionDenied),
                GroupPresence::PermissionDenied
            );
            assert!(!invalid.listed());
            assert_eq!(
                group_presence_after_reap(group),
                GroupPresence::ObservationError(io::ErrorKind::InvalidInput)
            );
            assert!(matches!(
                observe_group_after_reap(group),
                GroupObservation::Unobserved(reason) if reason.contains("invalid process group")
            ));
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn permission_listing_runs_off_the_executor_and_reports_task_failure() {
        let calls = Arc::default();
        let budgets = Arc::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut empty = PermissionListing::with_lister(
            40,
            deadline,
            counting_lister(&calls, &budgets, Vec::new()),
        );
        assert_eq!(
            empty.resolve(GroupPresence::Present).await,
            GroupPresence::Present
        );
        assert_eq!(
            empty.resolve(GroupPresence::PermissionDenied).await,
            GroupPresence::Absent
        );
        assert_eq!(
            empty.resolve(GroupPresence::PermissionDenied).await,
            GroupPresence::PermissionDenied
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let mut panicked =
            PermissionListing::with_lister(40, deadline, Arc::new(|_, _| panic!("lister")));
        assert_eq!(
            panicked.resolve(GroupPresence::PermissionDenied).await,
            GroupPresence::PermissionDenied
        );
        assert!(
            panicked
                .evidence()
                .is_some_and(|text| text.contains("listing unavailable: listing task")),
            "{panicked:?}"
        );
    }

    #[test]
    fn owned_listing_uses_the_real_listing_for_its_own_group() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
        let mut listing = owner.permission_listing(Instant::now() + Duration::from_secs(5));
        // Before EPERM nothing is listed; the real lister then finds no member
        // of a group whose only member was reaped.
        assert_eq!(
            listing.resolve_blocking(GroupPresence::Absent),
            GroupPresence::Absent
        );
        assert!(!listing.listed());
        assert!(matches!(
            owner.terminate_before_reap(),
            Termination::Signalled(_)
        ));
        let reap_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match owner.reap_if_exited() {
                Reap::Reaped(_) => break,
                Reap::NotExited | Reap::Interrupted if Instant::now() < reap_deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => panic!("owned test root was not reaped: {other:?}"),
            }
        }
        // The poll itself stays one signal-zero query; listing is separate.
        let before = owner.syscall_count();
        let presence = owner.presence_after_reap();
        assert_eq!(owner.syscall_count(), before + 1, "{presence:?}");
        let resolved = listing.resolve_blocking(GroupPresence::PermissionDenied);
        assert!(listing.listed());
        assert!(
            matches!(resolved, GroupPresence::Absent | GroupPresence::Recycled),
            "{resolved:?} {listing:?}"
        );
    }

    #[test]
    fn bounded_eintr_is_distinct_from_disarm() {
        let mut calls = 0;
        let state = observe_root_with_count(|| {
            calls += 1;
            Err(Errno::INTR)
        })
        .0;
        assert_eq!(state, Observation::Interrupted);
        assert_eq!(
            calls, 8,
            "bounded EINTR must retain authority after exactly eight calls"
        );
    }

    #[test]
    fn deterministic_transition_consumes_signal_authority_once() {
        let mut phase = Phase::Anchored;
        let calls = std::cell::RefCell::new(Vec::new());
        assert!(matches!(
            take_transition(
                &mut phase,
                Observation::Running,
                || {
                    calls.borrow_mut().push("group");
                    // A vanished original group still requires the anchored
                    // root attempt because it may have moved groups.
                    SignalOutcome::AlreadyAbsent
                },
                || {
                    calls.borrow_mut().push("root");
                    SignalOutcome::Sent
                },
            ),
            Termination::Signalled(_)
        ));
        assert_eq!(*calls.borrow(), ["group", "root"]);
        assert!(matches!(
            take_transition(
                &mut phase,
                Observation::Exited,
                || {
                    calls.borrow_mut().push("group-again");
                    SignalOutcome::Sent
                },
                || {
                    calls.borrow_mut().push("root-again");
                    SignalOutcome::Sent
                },
            ),
            Termination::InvalidPhase
        ));
        assert_eq!(
            *calls.borrow(),
            ["group", "root"],
            "repeated transition must issue no additional destructive syscall"
        );
    }

    #[test]
    fn deterministic_lost_identity_is_cached_without_another_observation() {
        let mut phase = Phase::Anchored;
        let mut calls = 0;
        let state = observe_root_with_count(|| {
            calls += 1;
            Err(Errno::CHILD)
        })
        .0;
        assert_eq!(state, Observation::Disarmed(DisarmReason::OwnershipLost));
        assert!(matches!(
            take_transition(&mut phase, state, || unreachable!(), || unreachable!()),
            Termination::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(calls, 1);
        assert!(matches!(
            take_transition(
                &mut phase,
                Observation::Running,
                || unreachable!(),
                || unreachable!(),
            ),
            Termination::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(
            calls, 1,
            "cached disarm must not require another wait syscall"
        );
    }

    #[test]
    fn deterministic_unexpected_observation_disarms_before_any_signal() {
        let mut phase = Phase::Anchored;
        let observation = observe_root_with_count(|| Err(Errno::IO)).0;
        assert!(matches!(
            observation,
            Observation::Disarmed(DisarmReason::ObservationError(_))
        ));
        assert!(matches!(
            take_transition(
                &mut phase,
                observation,
                || unreachable!(),
                || unreachable!()
            ),
            Termination::Disarmed(DisarmReason::ObservationError(_))
        ));
    }

    #[test]
    fn actual_owner_phase_guards_issue_no_more_syscalls_after_reap() {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30")
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut owner = OwnedProcessGroup::spawn(command, INHERITED).unwrap();
        let observation = (|| -> Result<(usize, usize), String> {
            if !matches!(owner.terminate_before_reap(), Termination::Signalled(_)) {
                return Err("first transition was not accepted".to_owned());
            }
            let after_transition = owner.syscall_count();
            if !matches!(owner.terminate_before_reap(), Termination::InvalidPhase) {
                return Err("repeated transition was not rejected".to_owned());
            }
            if owner.syscall_count() != after_transition {
                return Err("repeated transition issued another syscall".to_owned());
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match owner.reap_if_exited() {
                    Reap::Reaped(_) => break,
                    Reap::NotExited => {}
                    result => return Err(format!("unexpected reap result: {result:?}")),
                }
                if std::time::Instant::now() >= deadline {
                    return Err("terminated root did not exit".to_owned());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let after_reap = owner.syscall_count();
            if !matches!(owner.root_state(), RootState::Reaped(_))
                || !matches!(owner.reap_if_exited(), Reap::Reaped(_))
                || !matches!(owner.terminate_before_reap(), Termination::InvalidPhase)
                || owner.pre_reap_step(deadline) != PreReap::Reaped
            {
                return Err("cached reaped phase changed unexpectedly".to_owned());
            }
            if owner.syscall_count() != after_reap {
                return Err("cached reaped methods issued another syscall".to_owned());
            }
            Ok((after_transition, after_reap))
        })();
        // Always complete the physical lifecycle before asserting observations,
        // including when a pre-reap observation above failed.
        if let Err(error) = finish_test_owner(&mut owner) {
            // Preserve the real owner through a retrying continuation rather
            // than letting an assertion drop a post-signal child unreaped.
            std::thread::spawn(move || {
                loop {
                    if finish_test_owner(&mut owner).is_ok() {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            });
            panic!("test owner cleanup: {error}");
        }
        let (after_transition, after_reap) = observation.unwrap();
        assert_eq!(
            after_transition, 3,
            "one observe and ordered group/root signals"
        );
        assert!(after_reap >= after_transition);
        let after_cleanup = owner.syscall_count();
        // The production state reaches Disarmed only before a reap, but this
        // fixture has already explicitly reaped its known child above. That
        // lets the test exercise the permanent guard without stranding a
        // zombie when it forces the otherwise unreachable state.
        owner.phase = Phase::Disarmed(DisarmReason::OwnershipLost);
        assert!(matches!(
            owner.root_state(),
            RootState::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert!(matches!(
            owner.reap_if_exited(),
            Reap::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert!(matches!(
            owner.terminate_before_reap(),
            Termination::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(
            owner.pre_reap_step(Instant::now() + TEST_BOUND),
            PreReap::Disarmed(DisarmReason::OwnershipLost)
        );
        assert_eq!(
            owner.syscall_count(),
            after_cleanup,
            "disarmed calls must not retry wait, reap, or signal"
        );
        assert!(matches!(
            owner.presence_after_reap(),
            GroupPresence::InvalidPhase
        ));
        assert_eq!(owner.syscall_count(), after_cleanup);
    }

    #[test]
    fn actual_owner_exposes_each_configured_pipe_once() {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("cat >/dev/null")
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let plan = StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Pipe);
        let mut owner = OwnedProcessGroup::spawn(command, plan).unwrap();
        let input = owner.take_stdin().unwrap();
        let output = owner.take_stdout().unwrap();
        let error = owner.take_stderr().unwrap();
        assert!(owner.take_stdin().is_err());
        assert!(owner.take_stdout().is_err());
        assert!(owner.take_stderr().is_err());
        drop((input, output, error));
        finish_test_owner(&mut owner).unwrap();
    }

    /// What the second spawn did first while the first was in its window.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Second {
        Blocked,
        Spawned,
        Failed,
    }

    #[tokio::test]
    async fn checked_listener_binding_waits_for_descriptor_copy_and_cannot_leak_to_owned_child() {
        use crate::{
            fs::Directory,
            local_ipc::{PrivateServiceListener, connect},
        };
        use std::{
            ffi::OsStr,
            io::Write as _,
            os::unix::fs::{FileTypeExt, MetadataExt},
        };

        let temporary = tempfile::tempdir().unwrap();
        let private = temporary.path().join("private");
        let directory = Directory::ensure_private(&private).unwrap();
        std::fs::write(private.join("adjacent"), b"unchanged").unwrap();
        let (first, listener) = {
            let runtime = tokio::runtime::Handle::current();
            let (event, events) = std::sync::mpsc::channel();
            let spawning = spawn_lock();
            let binder = thread::spawn(move || {
                let _runtime = runtime.enter();
                let blocked = event.clone();
                BLOCKED.with(|seam| {
                    *seam.borrow_mut() = Some(Box::new(move || {
                        let _ = blocked.send(true);
                    }));
                });
                let result =
                    PrivateServiceListener::bind_at(directory, OsStr::new("generation.sock"));
                BLOCKED.with(|seam| seam.borrow_mut().take());
                let _ = event.send(false);
                result
            });
            let first = events.recv_timeout(TEST_BOUND);
            drop(spawning);
            (first, binder.join().unwrap().unwrap())
        };
        assert_eq!(
            first,
            Ok(true),
            "checked listener binding must wait for descriptor-copy admission"
        );

        // A completed cat roundtrip proves the owned child is alive after exec.
        // Its retained input keeps it alive through the listener's close probe.
        let mut command = Command::new("/bin/cat");
        command.env_clear();
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut owner = OwnedProcessGroup::spawn(
            command,
            StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Null),
        )
        .unwrap();
        let mut input = owner.take_stdin().unwrap();
        let mut output = owner.take_stdout().unwrap();
        let (read, readback) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            let mut bytes = [0; 6];
            let result = output.read_exact(&mut bytes).map(|()| bytes);
            let _ = read.send(result);
        });
        let written = input.write_all(b"ready\n");
        let ready = readback.recv_timeout(TEST_BOUND);
        let observed: Result<(), Box<dyn std::error::Error>> = async {
            written?;
            let bytes = ready??;
            if bytes != *b"ready\n" || !matches!(owner.root_state(), RootState::Running) {
                return Err(io::Error::other(
                    "owned cat was not live after the completed roundtrip",
                )
                .into());
            }
            let original = listener.path();
            let stale = private.join("stale.sock");
            std::fs::rename(&original, &stale)?;
            let identity = std::fs::symlink_metadata(&stale)?.ino();
            drop(listener);
            let directory = Directory::ensure_private(&private)?;
            let refused =
                tokio::time::timeout(TEST_BOUND, connect(&directory, OsStr::new("stale.sock")))
                    .await?;
            match refused {
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
                result => {
                    return Err(io::Error::other(format!(
                        "closed listener was not refused: {result:?}"
                    ))
                    .into());
                }
            }
            let retained = std::fs::symlink_metadata(&stale)?;
            if !retained.file_type().is_socket() || retained.ino() != identity {
                return Err(io::Error::other("stale connect changed the retained endpoint").into());
            }
            if PrivateServiceListener::bind_at(directory, OsStr::new("stale.sock")).is_ok() {
                return Err(
                    io::Error::other("stale endpoint was adopted by a checked bind").into(),
                );
            }
            if std::fs::read(private.join("adjacent"))? != b"unchanged"
                || !matches!(owner.root_state(), RootState::Running)
            {
                return Err(io::Error::other(
                    "adjacent state changed or the owned child exited during the probe",
                )
                .into());
            }
            Ok(())
        }
        .await;
        drop(input);
        let settled = settle_without_sleep(&mut owner, TEST_BOUND);
        reader.join().unwrap();
        settled.unwrap();
        observed.unwrap();
    }

    async fn private_ipc_operation_waits_for_descriptor_copy(accepting: bool) {
        use crate::{
            fs::Directory,
            local_ipc::{PrivateServiceListener, connect},
        };
        use std::{
            ffi::OsStr,
            future::Future as _,
            task::{Context, Poll, Waker},
        };
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let temporary = tempfile::tempdir().unwrap();
        let private = temporary.path().join("private");
        let listener = Arc::new(
            PrivateServiceListener::bind_at(
                Directory::ensure_private(&private).unwrap(),
                OsStr::new("generation.sock"),
            )
            .unwrap(),
        );
        let directory = Directory::ensure_private(&private).unwrap();
        // The accept case starts with a real queued connection, retained until
        // the protected poll completes and its accepted stream exchanges bytes.
        let queued = if accepting {
            Some(
                tokio::time::timeout(
                    TEST_BOUND,
                    connect(&directory, OsStr::new("generation.sock")),
                )
                .await
                .unwrap()
                .unwrap(),
            )
        } else {
            None
        };
        let (first, polled, operation) = {
            let runtime = tokio::runtime::Handle::current();
            let serving = Arc::clone(&listener);
            let (event, events) = std::sync::mpsc::channel();
            let spawning = spawn_lock();
            let worker = thread::spawn(move || {
                let _runtime = runtime.enter();
                let blocked = event.clone();
                BLOCKED.with(|seam| {
                    *seam.borrow_mut() = Some(Box::new(move || {
                        let _ = blocked.send(true);
                    }));
                });
                let mut operation = Box::pin(async move {
                    if accepting {
                        serving.accept().await
                    } else {
                        connect(&directory, OsStr::new("generation.sock")).await
                    }
                });
                let polled = operation
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()));
                BLOCKED.with(|seam| seam.borrow_mut().take());
                let _ = event.send(false);
                (polled, operation)
            });
            let first = events.recv_timeout(TEST_BOUND);
            drop(spawning);
            let (polled, operation) = worker.join().unwrap();
            (first, polled, operation)
        };
        let stream = match polled {
            Poll::Ready(result) => {
                drop(operation);
                result.unwrap()
            }
            Poll::Pending => tokio::time::timeout(TEST_BOUND, operation)
                .await
                .unwrap()
                .unwrap(),
        };
        let (mut client, mut server) = if accepting {
            (queued.unwrap(), stream)
        } else {
            let server = tokio::time::timeout(TEST_BOUND, listener.accept())
                .await
                .unwrap()
                .unwrap();
            (stream, server)
        };
        let exchanged = tokio::time::timeout(TEST_BOUND, async {
            client.write_all(b"ipc").await?;
            let mut bytes = [0; 3];
            server.read_exact(&mut bytes).await?;
            Ok::<_, io::Error>(bytes)
        })
        .await;
        drop((client, server));
        drop(listener);
        assert_eq!(
            first,
            Ok(true),
            "private IPC creation must wait for descriptor copying: accepting={accepting}"
        );
        assert_eq!(exchanged.unwrap().unwrap(), *b"ipc");
        assert!(!private.join("generation.sock").exists());
    }

    #[tokio::test]
    async fn checked_ipc_connect_waits_for_descriptor_copy() {
        private_ipc_operation_waits_for_descriptor_copy(false).await;
    }

    #[tokio::test]
    async fn checked_ipc_accept_waits_for_descriptor_copy() {
        private_ipc_operation_waits_for_descriptor_copy(true).await;
    }

    #[test]
    fn refused_owned_spawns_release_admission_before_a_complete_piped_successor() {
        use std::{io::Write as _, os::unix::fs::PermissionsExt};

        let temporary = tempfile::tempdir().unwrap();
        let nonexecutable = temporary.path().join("nonexecutable");
        std::fs::write(&nonexecutable, b"#!/bin/sh\nprintf invoked > invoked\n").unwrap();
        std::fs::set_permissions(&nonexecutable, std::fs::Permissions::from_mode(0o600)).unwrap();
        let plan = StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Pipe);
        for (executable, expected) in [
            (temporary.path().join("missing"), io::ErrorKind::NotFound),
            (nonexecutable, io::ErrorKind::PermissionDenied),
        ] {
            let mut command = Command::new(executable);
            command.current_dir(temporary.path()).env_clear();
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            let error = OwnedProcessGroup::spawn(command, plan).err().unwrap();
            assert_eq!(error.kind(), expected);
            assert!(!temporary.path().join("invoked").exists());
        }

        // The successor must use the same owned admission and all three pipe
        // slots, then prove actual echo, independent EOFs and exact owner reap.
        let mut command = Command::new("/bin/cat");
        command.current_dir(temporary.path()).env_clear();
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut owner = OwnedProcessGroup::spawn(command, plan).unwrap();
        let mut input = owner.take_stdin().unwrap();
        let mut output = owner.take_stdout().unwrap();
        let mut errors = owner.take_stderr().unwrap();
        let (sent_output, received_output) = std::sync::mpsc::channel();
        let (sent_errors, received_errors) = std::sync::mpsc::channel();
        let output_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = sent_output.send(output.read_to_end(&mut bytes).map(|_| bytes));
        });
        let error_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = sent_errors.send(errors.read_to_end(&mut bytes).map(|_| bytes));
        });
        let written = input.write_all(b"exact successor echo\n");
        drop(input);
        let output = received_output.recv_timeout(TEST_BOUND);
        let errors = received_errors.recv_timeout(TEST_BOUND);
        let settled = settle_without_sleep(&mut owner, TEST_BOUND);
        output_reader.join().unwrap();
        error_reader.join().unwrap();
        settled.unwrap();
        written.unwrap();
        assert_eq!(output.unwrap().unwrap(), b"exact successor echo\n");
        assert!(errors.unwrap().unwrap().is_empty());
        assert!(matches!(owner.root_state(), RootState::Reaped(_)));
        assert!(!temporary.path().join("invoked").exists());
    }

    #[test]
    fn independent_spawn_waits_for_owned_pipe_creation() {
        let bound = Duration::from_secs(5);
        let (entered, in_window) = std::sync::mpsc::channel();
        let (event, events) = std::sync::mpsc::channel();
        let (seen, first_seen) = std::sync::mpsc::channel();
        let first = std::thread::spawn(move || {
            WINDOW.with(|window| {
                *window.borrow_mut() = Some(Box::new(move || {
                    let _ = entered.send(());
                    let _ = seen.send(events.recv_timeout(bound));
                }));
            });
            let mut command = Command::new("/bin/cat");
            command.env_clear();
            OwnedProcessGroup::spawn(
                command,
                StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Null),
            )
        });
        let second = std::thread::spawn(move || -> io::Result<ExitStatus> {
            in_window
                .recv_timeout(bound)
                .map_err(|error| io::Error::other(format!("owned pipe window absent: {error}")))?;
            let blocked = event.clone();
            BLOCKED.with(|seam| {
                *seam.borrow_mut() = Some(Box::new(move || {
                    let _ = blocked.send(Second::Blocked);
                }));
            });
            // The same stock cat used by the owned sibling exits on the
            // independent child's null stdin; /bin/true is absent on macOS.
            let mut command = Command::new("/bin/cat");
            command.env_clear();
            let child = spawn_independent(command, None);
            BLOCKED.with(|seam| seam.borrow_mut().take());
            let _ = event.send(if child.is_ok() {
                Second::Spawned
            } else {
                Second::Failed
            });
            // An independent child is observed/reaped by its actual caller,
            // outside the platform lock and without group termination.
            child?.wait()
        });
        let (first, second) = (first.join().unwrap(), second.join().unwrap());
        let mut first = first.unwrap();
        let first_event = first_seen.try_recv();
        drop(first.take_stdin().unwrap());
        let mut output = first.take_stdout().unwrap();
        let settled = settle_without_sleep(&mut first, bound);
        let mut bytes = Vec::new();
        let eof = output.read_to_end(&mut bytes);
        settled.unwrap();
        assert!(second.unwrap().success());
        assert_eq!(first_event, Ok(Ok(Second::Blocked)));
        assert_eq!(eof.unwrap(), 0);
    }

    #[test]
    fn piped_spawn_waits_for_owned_pipe_creation_and_keeps_lifetime_eof_isolated() {
        let bound = Duration::from_secs(5);
        let (entered, in_window) = std::sync::mpsc::channel();
        let (event, events) = std::sync::mpsc::channel();
        let (seen, first_seen) = std::sync::mpsc::channel();
        let first = std::thread::spawn(move || {
            WINDOW.with(|window| {
                *window.borrow_mut() = Some(Box::new(move || {
                    let _ = entered.send(());
                    let _ = seen.send(events.recv_timeout(bound));
                }));
            });
            let mut command = Command::new("/bin/cat");
            command.env_clear();
            OwnedProcessGroup::spawn(
                command,
                StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Null),
            )
        });
        let second = std::thread::spawn(move || -> io::Result<Child> {
            in_window
                .recv_timeout(bound)
                .map_err(|error| io::Error::other(format!("owned pipe window absent: {error}")))?;
            let blocked = event.clone();
            BLOCKED.with(|seam| {
                *seam.borrow_mut() = Some(Box::new(move || {
                    let _ = blocked.send(Second::Blocked);
                }));
            });
            let mut command = Command::new("/bin/cat");
            command.env_clear();
            let child = spawn_piped(command);
            BLOCKED.with(|seam| seam.borrow_mut().take());
            let _ = event.send(if child.is_ok() {
                Second::Spawned
            } else {
                Second::Failed
            });
            child
        });
        let (first, second) = (first.join().unwrap(), second.join().unwrap());
        let (mut first, mut second) = (first.unwrap(), second.unwrap());
        let first_event = first_seen.try_recv();
        drop(first.take_stdin().unwrap());
        let mut output = first.take_stdout().unwrap();
        let (complete, eof) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = complete.send(output.read_to_end(&mut bytes).map(|_| bytes.len()));
        });
        // The separately retained lifetime input is still open. An inherited
        // copy of the first pipe would prevent this EOF until it closed.
        let first_eof = eof.recv_timeout(bound);
        drop(second.stdin.take());
        let second_status = second.wait();
        let settled = settle_without_sleep(&mut first, bound);
        reader.join().unwrap();
        settled.unwrap();
        assert!(second_status.unwrap().success());
        assert_eq!(first_event, Ok(Ok(Second::Blocked)));
        assert_eq!(first_eof.unwrap().unwrap(), 0);
    }

    #[test]
    fn concurrent_owned_spawn_cannot_copy_a_pipe_in_its_window() {
        // The sibling owner tests' bound; nothing here sleeps.
        let bound = Duration::from_secs(5);
        let cat = || {
            let mut command = Command::new("/bin/cat");
            command.env_clear();
            command
        };
        let (entered, in_window) = std::sync::mpsc::channel::<()>();
        let (event, events) = std::sync::mpsc::channel::<Second>();
        let (seen, first_seen) = std::sync::mpsc::channel();
        // A pauses after creating its stdin pipe (on Apple before either end
        // is close-on-exec) until B reports what it did first. It never waits
        // for B's spawn alone, so a serialised B cannot deadlock it.
        let first = std::thread::spawn(move || {
            WINDOW.with(|window| {
                *window.borrow_mut() = Some(Box::new(move || {
                    let _ = entered.send(());
                    let _ = seen.send(events.recv_timeout(bound));
                }));
            });
            let plan = StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Null);
            OwnedProcessGroup::spawn(cat(), plan)
        });
        let second = std::thread::spawn(move || {
            in_window.recv_timeout(bound).map_err(|error| {
                io::Error::other(format!("A never reached its window: {error}"))
            })?;
            let blocked = event.clone();
            BLOCKED.with(|seam| {
                *seam.borrow_mut() = Some(Box::new(move || {
                    let _ = blocked.send(Second::Blocked);
                }));
            });
            let plan = StdioPlan::new(StdioSlot::Pipe, StdioSlot::Null, StdioSlot::Null);
            let spawned = OwnedProcessGroup::spawn(cat(), plan);
            BLOCKED.with(|seam| seam.borrow_mut().take());
            let _ = event.send(if spawned.is_ok() {
                Second::Spawned
            } else {
                Second::Failed
            });
            spawned
        });
        let (first, second) = (first.join().unwrap(), second.join().unwrap());
        let (mut first, mut second) = (first.unwrap(), second.unwrap());
        let first_event = first_seen.try_recv();
        let (first_input, first_output) = (first.take_stdin(), first.take_stdout());
        let second_input = second.take_stdin();
        // B's input stays open: a copy of A's input write end inherited by B's
        // cat would keep A's cat from reaching end of file.
        drop(first_input);
        let (finished, eof) = std::sync::mpsc::channel();
        let reader = first_output.map(|mut output| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = finished.send(output.read_to_end(&mut bytes).map(|_| bytes.len()));
            })
        });
        let first_eof = eof.recv_timeout(bound);
        drop(second_input);
        let settled = [
            settle_without_sleep(&mut first, bound),
            settle_without_sleep(&mut second, bound),
        ];
        // Join the reader only once it has reported; after both groups are
        // gone nothing else holds A's output write end.
        if (first_eof.is_ok() || eof.recv_timeout(bound).is_ok())
            && let Ok(reader) = reader
        {
            reader.join().unwrap();
        }
        assert!(
            matches!(first_eof, Ok(Ok(0))),
            "A's cat did not reach end of file within {bound:?} while B ran: {first_eof:?} \
             (B's first event: {first_event:?})"
        );
        assert_eq!(
            first_event,
            Ok(Ok(Second::Blocked)),
            "B must wait for the platform spawn lock while A is in its window"
        );
        for result in settled {
            result.unwrap();
        }
    }

    /// [`finish_test_owner`] without sleeping between bounded polls.
    fn settle_without_sleep(owner: &mut OwnedProcessGroup, bound: Duration) -> Result<(), String> {
        let deadline = Instant::now() + bound;
        loop {
            match owner.terminate_before_reap() {
                Termination::Signalled(_) | Termination::InvalidPhase => break,
                Termination::Interrupted if Instant::now() < deadline => {}
                result => return Err(format!("test transition cleanup: {result:?}")),
            }
            std::thread::yield_now();
        }
        loop {
            match owner.pre_reap_step(deadline) {
                PreReap::Ready | PreReap::Reaped => {}
                PreReap::Pending | PreReap::Unobserved(_) if Instant::now() < deadline => {
                    thread::yield_now();
                    continue;
                }
                other => return Err(format!("test pre-reap cleanup: {other:?}")),
            }
            match owner.reap_if_exited() {
                Reap::Reaped(_) => break,
                Reap::NotExited | Reap::Interrupted if Instant::now() < deadline => {}
                result => return Err(format!("test root reap cleanup: {result:?}")),
            }
            std::thread::yield_now();
        }
        let mut listing = owner.permission_listing(deadline);
        loop {
            match listing.resolve_blocking(owner.presence_after_reap()) {
                GroupPresence::Absent | GroupPresence::Recycled => return Ok(()),
                GroupPresence::Present | GroupPresence::PermissionDenied
                    if Instant::now() < deadline => {}
                result => return Err(format!("test group cleanup: {result:?}")),
            }
            std::thread::yield_now();
        }
    }

    pub(super) fn finish_test_owner(owner: &mut OwnedProcessGroup) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match owner.terminate_before_reap() {
                Termination::Signalled(_) | Termination::InvalidPhase => break,
                Termination::Interrupted if std::time::Instant::now() < deadline => {}
                result => return Err(format!("test transition cleanup: {result:?}")),
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        loop {
            match owner.pre_reap_step(deadline) {
                PreReap::Ready | PreReap::Reaped => {}
                PreReap::Pending | PreReap::Unobserved(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                other => return Err(format!("test pre-reap cleanup: {other:?}")),
            }
            match owner.reap_if_exited() {
                Reap::Reaped(_) => break,
                Reap::NotExited | Reap::Interrupted if std::time::Instant::now() < deadline => {}
                result => return Err(format!("test root reap cleanup: {result:?}")),
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut listing = owner.permission_listing(deadline);
        loop {
            match listing.resolve_blocking(owner.presence_after_reap()) {
                GroupPresence::Absent | GroupPresence::Recycled => return Ok(()),
                GroupPresence::Present | GroupPresence::PermissionDenied
                    if std::time::Instant::now() < deadline => {}
                result => return Err(format!("test group cleanup: {result:?}")),
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod native_disarm_contracts {
    use super::*;

    const BOUND: Duration = Duration::from_secs(5);

    pub(super) fn exited_root() -> OwnedProcessGroup {
        // Native fixture has no descendants and inherits LLVM_PROFILE_FILE.
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        OwnedProcessGroup::spawn(
            command,
            StdioPlan::new(StdioSlot::Null, StdioSlot::Null, StdioSlot::Null),
        )
        .unwrap()
    }

    pub(super) fn consume_exact_wait(owner: &mut OwnedProcessGroup) {
        let deadline = Instant::now() + BOUND;
        loop {
            // Test-only interference uses this owner's actual retained child.
            // It never signals or reaps an unrelated numeric identity.
            match waitid(
                WaitId::Pid(owner.group),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG,
            ) {
                Ok(Some(_)) => return,
                Ok(None) | Err(Errno::INTR) if Instant::now() < deadline => thread::yield_now(),
                outcome => {
                    let cleanup = tests::finish_test_owner(owner);
                    panic!("fixture child did not yield its exact wait: {outcome:?}; {cleanup:?}");
                }
            }
        }
    }

    pub(super) fn transition(owner: &mut OwnedProcessGroup) {
        let deadline = Instant::now() + BOUND;
        loop {
            match owner.terminate_before_reap() {
                Termination::Signalled(_) => return,
                Termination::Interrupted if Instant::now() < deadline => thread::yield_now(),
                outcome => {
                    let cleanup = tests::finish_test_owner(owner);
                    panic!("fixture transition failed: {outcome:?}; {cleanup:?}");
                }
            }
        }
    }

    fn assert_permanently_disarmed(owner: &mut OwnedProcessGroup) {
        let calls = owner.syscall_count();
        assert!(matches!(
            owner.root_state(),
            RootState::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(
            owner.terminate_before_reap(),
            Termination::Disarmed(DisarmReason::OwnershipLost)
        );
        assert_eq!(
            owner.pre_reap_step(Instant::now() + BOUND),
            PreReap::Disarmed(DisarmReason::OwnershipLost)
        );
        assert!(matches!(
            owner.reap_if_exited(),
            Reap::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(owner.presence_after_reap(), GroupPresence::InvalidPhase);
        assert_eq!(
            owner.syscall_count(),
            calls,
            "disarm must stop native calls"
        );
        assert!(
            matches!(
                waitid(
                    WaitId::Pid(owner.group),
                    WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
                ),
                Err(Errno::CHILD)
            ),
            "fixture child must already be reaped without claiming cleanup success"
        );
    }

    #[test]
    fn native_consumed_wait_disarms_an_anchored_owner_without_signalling() {
        let mut owner = exited_root();
        consume_exact_wait(&mut owner);
        let calls = owner.syscall_count();
        assert_eq!(
            owner.terminate_before_reap(),
            Termination::Disarmed(DisarmReason::OwnershipLost)
        );
        assert_eq!(owner.syscall_count(), calls + 1, "observation only");
        assert_permanently_disarmed(&mut owner);
    }

    #[test]
    fn native_consumed_wait_disarms_pre_reap_instead_of_starting_a_snapshot() {
        let mut owner = exited_root();
        transition(&mut owner);
        consume_exact_wait(&mut owner);
        assert_eq!(
            owner.pre_reap_step(Instant::now() + BOUND),
            PreReap::Disarmed(DisarmReason::OwnershipLost)
        );
        assert!(owner.membership.is_none());
        assert_eq!(owner.membership_jobs, 0, "no snapshot child was launched");
        assert_permanently_disarmed(&mut owner);
    }

    #[test]
    fn native_consumed_observed_exit_disarms_the_exact_child_reap() {
        let mut owner = exited_root();
        let deadline = Instant::now() + BOUND;
        loop {
            match owner.root_state() {
                RootState::Exited => break,
                RootState::Running | RootState::Interrupted if Instant::now() < deadline => {
                    thread::yield_now();
                }
                outcome => {
                    let cleanup = tests::finish_test_owner(&mut owner);
                    panic!("fixture exit was not observed: {outcome:?}; {cleanup:?}");
                }
            }
        }
        transition(&mut owner);
        consume_exact_wait(&mut owner);
        let calls = owner.syscall_count();
        assert!(matches!(
            owner.reap_if_exited(),
            Reap::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(owner.syscall_count(), calls + 1, "exact child wait only");
        assert_permanently_disarmed(&mut owner);
    }
}

#[cfg(test)]
mod native_failure_contracts {
    use super::*;

    const BOUND: Duration = Duration::from_secs(5);

    #[test]
    fn fresh_root_and_reap_observations_disarm_after_native_wait_loss() {
        let mut owner = native_disarm_contracts::exited_root();
        assert_eq!(
            owner.pre_reap_step(Instant::now() + BOUND),
            PreReap::InvalidPhase
        );
        assert_eq!(
            owner.resignal_before_reap(Instant::now() + BOUND),
            PreReap::InvalidPhase
        );
        native_disarm_contracts::consume_exact_wait(&mut owner);
        assert!(matches!(
            owner.root_state(),
            RootState::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert_eq!(
            owner.resignal_before_reap(Instant::now() + BOUND),
            PreReap::InvalidPhase
        );
        let calls = owner.syscall_count();
        assert!(tests::finish_test_owner(&mut owner).is_err());
        assert_eq!(
            owner.syscall_count(),
            calls,
            "cleanup cannot regain wait ownership"
        );

        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let mut running = OwnedProcessGroup::spawn(
            command,
            StdioPlan::new(StdioSlot::Null, StdioSlot::Null, StdioSlot::Null),
        )
        .unwrap();
        assert!(matches!(running.root_state(), RootState::Running));
        native_disarm_contracts::transition(&mut running);
        assert!(
            !running.observed_exit,
            "exit must require a fresh observation"
        );
        native_disarm_contracts::consume_exact_wait(&mut running);
        assert!(matches!(
            running.reap_if_exited(),
            Reap::Disarmed(DisarmReason::OwnershipLost)
        ));
        assert!(tests::finish_test_owner(&mut running).is_err());
        assert_eq!(running.presence_after_reap(), GroupPresence::InvalidPhase);
    }

    #[tokio::test]
    async fn completed_snapshot_faults_are_unobserved_and_never_cleanup_readiness() {
        for panic in [false, true] {
            let mut owner = native_disarm_contracts::exited_root();
            let limit = Instant::now() + BOUND;
            let observed = (|| -> Result<(), String> {
                while !matches!(owner.root_state(), RootState::Exited) {
                    if Instant::now() >= limit {
                        return Err("fixture exit not observed".into());
                    }
                    thread::yield_now();
                }
                Ok(())
            })();
            if let Err(error) = observed {
                let cleanup = tests::finish_test_owner(&mut owner);
                panic!("{error}; {cleanup:?}");
            }
            native_disarm_contracts::transition(&mut owner);
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let mut release = Some(release);
            let worker = thread::spawn(move || -> io::Result<Vec<snapshot::GroupMember>> {
                let _ = gate.recv();
                assert!(!panic, "controlled read-only snapshot worker failure");
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "controlled listing denial",
                ))
            });
            while !worker.is_finished() && Instant::now() < limit {
                // The worker cannot finish before this first pending probe.
                // Closing its private gate causes completion without a delay.
                drop(release.take());
                thread::yield_now();
            }
            drop(release);
            owner.membership = Some(MembershipJob {
                worker,
                completion: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                deadline: limit,
            });
            owner.wait_pre_reap(Duration::ZERO, limit).await;
            owner.wait_pre_reap(Duration::from_millis(1), limit).await;
            let outcome = owner.pre_reap_step(limit);
            let retained = matches!(owner.phase, Phase::PostSignal);
            let consumed = owner.membership.is_none();
            let cleanup = tests::finish_test_owner(&mut owner);
            cleanup.unwrap();
            assert_eq!(
                outcome,
                PreReap::Unobserved(if panic {
                    io::ErrorKind::Other
                } else {
                    io::ErrorKind::PermissionDenied
                })
            );
            assert!(retained, "failed listing must not reap the actual root");
            assert!(
                consumed,
                "completed failure must not be mistaken for a pending worker"
            );
        }
    }

    #[test]
    fn native_wait_loss_cancels_a_pending_snapshot_and_never_repeats_a_signal() {
        let mut owner = native_disarm_contracts::exited_root();
        native_disarm_contracts::transition(&mut owner);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let (entered, entry) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let worker = thread::spawn(move || {
            entered.send(()).unwrap();
            let _ = gate.recv();
            Ok(Vec::new())
        });
        entry.recv_timeout(BOUND).unwrap();
        owner.membership = Some(MembershipJob {
            worker,
            completion: None,
            cancelled: flag,
            deadline: Instant::now() + BOUND,
        });
        native_disarm_contracts::consume_exact_wait(&mut owner);
        assert_eq!(
            owner.resignal_before_reap(Instant::now() + BOUND),
            PreReap::Disarmed(DisarmReason::OwnershipLost)
        );
        let calls = owner.syscall_count();
        let outcome = owner.pre_reap_step(Instant::now() + BOUND);
        let was_pending = owner
            .membership
            .as_ref()
            .is_some_and(|job| !job.worker.is_finished());
        let was_cancelled = cancelled.load(Ordering::Acquire);
        let mut release = Some(release);
        let limit = Instant::now() + BOUND;
        while owner
            .membership
            .as_ref()
            .is_some_and(|job| !job.worker.is_finished())
            && Instant::now() < limit
        {
            // The observed pending worker owns its gate until this poll.
            drop(release.take());
            thread::yield_now();
        }
        drop(release);
        let finished = owner.pre_reap_step(limit);
        assert_eq!(outcome, PreReap::Disarmed(DisarmReason::OwnershipLost));
        assert!(was_pending && was_cancelled);
        assert_eq!(finished, PreReap::Disarmed(DisarmReason::OwnershipLost));
        assert!(
            owner.membership.is_none(),
            "completed cancelled worker must be joined"
        );
        assert_eq!(
            owner.syscall_count(),
            calls,
            "disarm prevents all further native calls"
        );
    }
}

#[cfg(test)]
mod observation_failure_contracts {
    use super::*;

    #[test]
    fn unobservable_native_outcomes_never_claim_absence_or_completed_termination() {
        let kind = Errno::IO.kind();
        assert_eq!(signal(Err(Errno::IO)), SignalOutcome::Failed(kind));
        assert_eq!(
            presence(Err(Errno::IO)),
            GroupPresence::ObservationError(kind)
        );
        for group in [0, 1, u32::MAX] {
            assert_eq!(
                group_presence_after_reap(group),
                GroupPresence::ObservationError(io::ErrorKind::InvalidInput)
            );
            let observed = observe_group_after_reap(group);
            assert!(!observed.none_of_ours());
            assert!(observed.to_string().contains("invalid process group"));
        }
        let denied = classify_group(Ok(()), 40, &own_uids(), || {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "controlled native snapshot refusal",
            ))
        });
        assert!(!denied.none_of_ours());
        assert_eq!(
            denied.to_string(),
            "survivors (listing unavailable: controlled native snapshot refusal)"
        );
    }

    #[test]
    fn read_only_numeric_presence_observes_a_physically_settled_owned_group() {
        let mut owner = native_disarm_contracts::exited_root();
        let group = owner.group.as_raw_nonzero().get().unsigned_abs();
        tests::finish_test_owner(&mut owner).unwrap();
        assert_eq!(group_presence_after_reap(group), GroupPresence::Absent);
        assert_eq!(observe_group_after_reap(group).to_string(), "absent");
        assert!(matches!(owner.root_state(), RootState::Reaped(_)));
        assert_eq!(owner.terminate_before_reap(), Termination::InvalidPhase);
    }
}

#[cfg(test)]
mod retained_worker_contracts {
    use super::*;

    #[tokio::test]
    async fn actual_root_reap_retains_cancelled_worker_without_regaining_signal_authority() {
        let mut owner = native_disarm_contracts::exited_root();
        native_disarm_contracts::transition(&mut owner);
        let limit = Instant::now() + tests::TEST_BOUND;
        let calls = owner.syscall_count();
        let poll = Duration::from_millis(2);
        let started = Instant::now();
        owner.wait_pre_reap(poll, limit).await;
        let elapsed = Instant::now().duration_since(started);
        let no_worker = owner.membership.is_none();
        let no_worker_calls = owner.syscall_count();
        let expired = owner.resignal_before_reap(Instant::now());
        let expired_calls = owner.syscall_count();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (release, gate) = std::sync::mpsc::channel();
        let (completed, completion) = tokio::sync::oneshot::channel();
        owner.membership = Some(MembershipJob {
            worker: thread::spawn(move || {
                let _ = gate.recv();
                let _ = completed.send(());
                Ok(Vec::new())
            }),
            completion: Some(completion),
            cancelled: Arc::clone(&cancelled),
            deadline: limit,
        });
        owner.wait_pre_reap(poll, limit).await;
        let wake_retained = owner
            .membership
            .as_ref()
            .is_some_and(|job| job.completion.is_some());
        let waiting_calls = owner.syscall_count();
        loop {
            match owner.reap_if_exited() {
                Reap::Reaped(_) => break,
                Reap::NotExited | Reap::Interrupted if Instant::now() < limit => {
                    tokio::task::yield_now().await;
                }
                outcome => {
                    drop(release);
                    let cleanup = tests::finish_test_owner(&mut owner);
                    panic!("native root reap failed: {outcome:?}; {cleanup:?}");
                }
            }
        }
        let reaped_calls = owner.syscall_count();
        let pending = owner.pre_reap_step(limit);
        let retained = owner.membership.is_some();
        let was_cancelled = cancelled.load(Ordering::Acquire);
        let cached_root = owner.root_state();
        let repeated_reap = owner.reap_if_exited();
        let termination = owner.terminate_before_reap();
        let still_calls = owner.syscall_count();
        release.send(()).unwrap();
        owner.wait_pre_reap(tests::TEST_BOUND, limit).await;
        let completed_state = owner.pre_reap_step(limit);
        let joined = owner.membership.is_none();
        let completed_calls = owner.syscall_count();
        tests::finish_test_owner(&mut owner).unwrap();
        assert!(elapsed >= poll);
        assert!(no_worker);
        assert_eq!(no_worker_calls, calls);
        assert_eq!(expired, PreReap::Expired);
        assert_eq!(expired_calls, calls, "expiry must not observe or signal");
        assert!(
            wake_retained,
            "poll expiry must retain the completion receiver"
        );
        assert_eq!(
            waiting_calls, calls,
            "waiting must not perform native observations"
        );
        assert_eq!(pending, PreReap::Pending);
        assert!(
            retained && was_cancelled,
            "reap must cancel and retain unfinished work"
        );
        assert!(matches!(cached_root, RootState::Reaped(_)));
        assert!(matches!(repeated_reap, Reap::Reaped(_)));
        assert_eq!(termination, Termination::InvalidPhase);
        assert_eq!(
            still_calls, reaped_calls,
            "cached states cannot regain syscall authority"
        );
        assert_eq!(completed_state, PreReap::Reaped);
        assert!(joined, "completed cancelled worker must be joined");
        assert_eq!(
            completed_calls, reaped_calls,
            "late membership cannot signal or reap again"
        );
    }
}
