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
        Pid, Signal, WaitId, WaitIdOptions, kill_process, kill_process_group,
        test_kill_process_group, waitid,
    },
};
#[cfg(test)]
use std::cell::Cell;
use std::{
    io,
    os::unix::process::CommandExt,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus},
};

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
    PermissionDenied,
    ObservationError(io::ErrorKind),
    /// Group presence is only meaningful after the standard child is reaped.
    InvalidPhase,
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
    group: Pid,
    phase: Phase,
    observed_exit: bool,
    #[cfg(test)]
    syscalls: Cell<usize>,
}

impl OwnedProcessGroup {
    /// Spawn a standard child in a fresh process group led by that child.
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        command.process_group(0);
        let child = command.spawn()?;
        // rustix reads the standard Child's native identity infallibly. Keep
        // this immediately after spawn: an error return must never drop a
        // newly-created child before it is represented by this owner.
        let group = Pid::from_child(&child);
        Ok(Self {
            child,
            group,
            phase: Phase::Anchored,
            observed_exit: false,
            #[cfg(test)]
            syscalls: Cell::new(0),
        })
    }

    /// Take the owned standard-input pipe once.
    pub fn take_stdin(&mut self) -> io::Result<ChildStdin> {
        self.child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("owned child stdin was not piped"))
    }

    /// Take the owned standard-output pipe once.
    pub fn take_stdout(&mut self) -> io::Result<ChildStdout> {
        self.child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("owned child stdout was not piped"))
    }

    /// Take the owned standard-error pipe once.
    pub fn take_stderr(&mut self) -> io::Result<ChildStderr> {
        self.child
            .stderr
            .take()
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

    #[test]
    fn signal_and_post_reap_presence_keep_absence_and_permission_distinct() {
        assert_eq!(signal(Err(Errno::SRCH)), SignalOutcome::AlreadyAbsent);
        assert_eq!(signal(Err(Errno::PERM)), SignalOutcome::PermissionDenied);
        assert_eq!(presence(Err(Errno::SRCH)), GroupPresence::Absent);
        assert_eq!(presence(Err(Errno::PERM)), GroupPresence::PermissionDenied);
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
        let mut owner = OwnedProcessGroup::spawn(command).unwrap();
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
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut owner = OwnedProcessGroup::spawn(command).unwrap();
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
        loop {
            match owner.presence_after_reap() {
                GroupPresence::Absent => return Ok(()),
                GroupPresence::Present | GroupPresence::PermissionDenied
                    if std::time::Instant::now() < deadline => {}
                result => return Err(format!("test group cleanup: {result:?}")),
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
