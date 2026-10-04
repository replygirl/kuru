//! Safe ownership of one standard Unix child and its fresh process group.
//!
//! [`OwnedProcessGroup`] keeps the child's wait identity until it has consumed
//! its one group-then-root destructive transition. It deliberately exposes
//! neither a PID nor the child, so callers cannot reap the root and later
//! signal a recycled numeric group. This is process authority, not sandboxing:
//! processes that leave the fresh group are outside this boundary.

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
    sync::Arc,
    time::{Duration, Instant},
};

pub mod snapshot;

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
/// one transition with [`Self::terminate_before_reap`], reap the exact root,
/// and then use only [`Self::presence_after_reap`].
pub struct OwnedProcessGroup {
    child: Child,
    stdin: Option<OwnedFd>,
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
    group: Pid,
    phase: Phase,
    observed_exit: bool,
    #[cfg(test)]
    syscalls: Cell<usize>,
}

impl OwnedProcessGroup {
    /// Spawn a standard child in a fresh process group led by that child.
    ///
    /// `stdio` replaces any standard stream configured on `command`. The
    /// platform creates every declared pipe close-on-exec and hands std only
    /// the child's ends, so std creates no stdio pipe of its own.
    pub fn spawn(mut command: Command, stdio: StdioPlan) -> io::Result<Self> {
        command.process_group(0);
        let started = launch(&mut command, stdio);
        // The Command owns the child's pipe ends. Close them before returning:
        // a parent copy of its own child's stdout write end would keep that
        // pipe from ever reaching end of file.
        drop(command);
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
            #[cfg(test)]
            syscalls: Cell::new(0),
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
        let transition = take_transition(
            &mut self.phase,
            observation,
            || signal_group(group),
            || signal_root(group),
        );
        if matches!(transition, Termination::Signalled(_)) {
            self.record_syscalls(2);
        }
        transition
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
    }
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
/// copy them.
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
                // Consume authority before the first destructive syscall. A
                // failed group signal must not leave a later call able to
                // target a recycled numeric group.
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

    const INHERITED: StdioPlan =
        StdioPlan::new(StdioSlot::Inherit, StdioSlot::Inherit, StdioSlot::Inherit);

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

    fn finish_test_owner(owner: &mut OwnedProcessGroup) -> Result<(), String> {
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
