//! Retained Unix built-in shell ownership.
//!
//! A caller future is not the process owner. Each accepted shell call has a
//! small OS-thread worker with its own Tokio runtime so cancellation or loss of
//! the parent runtime cannot drop the standard child before ordered cleanup.

use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use kuru_platform::{
    fs::Directory,
    unix::{GroupPresence, OwnedProcessGroup, Reap, RootState, Termination},
};
use serde_json::json;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::oneshot,
    time::{sleep, timeout_at},
};

use crate::MAX_BYTES;

const CLEANUP_ALLOWANCE: Duration = Duration::from_secs(5);
const OBSERVE_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) struct ShellRegistry {
    inner: Arc<RegistryInner>,
    #[cfg(test)]
    test_hooks: TestHooks,
}

#[cfg(test)]
#[derive(Clone)]
struct TestHooks {
    point: Arc<std::sync::atomic::AtomicU8>,
    start_gate: Arc<Mutex<Option<Arc<TestGate>>>>,
    retained_cleanup_gate: Arc<Mutex<Option<Arc<TestGate>>>>,
    interrupted_cleanup: Arc<AtomicBool>,
    transitions: Arc<std::sync::atomic::AtomicUsize>,
    cleanup_budget_ms: Arc<AtomicU64>,
}

#[cfg(test)]
struct InterruptedCleanupGuard(Arc<AtomicBool>);

#[cfg(test)]
impl InterruptedCleanupGuard {
    fn release(&self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
impl Drop for InterruptedCleanupGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
struct TestGate {
    released: Mutex<bool>,
    wake: std::sync::Condvar,
    entered: AtomicBool,
}

#[cfg(test)]
impl TestGate {
    fn wait(&self) {
        self.entered.store(true, Ordering::Release);
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*released {
            released = self
                .wake
                .wait(released)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        self.wake.notify_all();
    }
}

#[cfg(test)]
pub(crate) struct TestGateGuard(Arc<TestGate>);

#[cfg(test)]
impl TestGateGuard {
    pub(crate) fn release(&self) {
        self.0.release();
    }

    pub(crate) fn entered(&self) -> bool {
        self.0.entered.load(Ordering::Acquire)
    }
}

#[cfg(test)]
impl Drop for TestGateGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
enum TestPoint {
    ThreadStartFailure = 1,
    RuntimeStartFailure = 2,
    PreSpawnPanic = 3,
    PartialPipeConversionFailure = 4,
    PostSpawnPanic = 5,
    PipeReadFailure = 6,
    CleanupPanic = 7,
}

#[cfg(test)]
impl TestHooks {
    fn new() -> Self {
        Self {
            point: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            start_gate: Arc::new(Mutex::new(None)),
            retained_cleanup_gate: Arc::new(Mutex::new(None)),
            interrupted_cleanup: Arc::new(AtomicBool::new(false)),
            transitions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            cleanup_budget_ms: Arc::new(AtomicU64::new(CLEANUP_ALLOWANCE.as_millis() as u64)),
        }
    }

    fn set_point(&self, point: TestPoint) {
        assert_eq!(self.point.swap(point as u8, Ordering::AcqRel), 0);
    }

    fn arm_retained_cleanup_gate(&self) -> TestGateGuard {
        let gate = Arc::new(TestGate {
            released: Mutex::new(false),
            wake: std::sync::Condvar::new(),
            entered: AtomicBool::new(false),
        });
        let mut retained_cleanup_gate = self
            .retained_cleanup_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(retained_cleanup_gate.replace(gate.clone()).is_none());
        TestGateGuard(gate)
    }

    fn arm_start_gate(&self) -> TestGateGuard {
        let gate = Arc::new(TestGate {
            released: Mutex::new(false),
            wake: std::sync::Condvar::new(),
            entered: AtomicBool::new(false),
        });
        let mut start_gate = self
            .start_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(start_gate.replace(gate.clone()).is_none());
        TestGateGuard(gate)
    }

    fn take(&self, point: TestPoint) -> bool {
        self.point
            .compare_exchange(point as u8, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn panic_if_armed(&self, point: TestPoint) {
        if self.take(point) {
            panic!("deterministic retained shell {point:?} test hook");
        }
    }

    fn await_start(&self) {
        let gate = self
            .start_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(gate) = gate {
            gate.wait();
        }
    }

    fn await_retained_cleanup(&self) {
        let gate = self
            .retained_cleanup_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(gate) = gate {
            gate.wait();
        }
    }

    fn arm_interrupted_cleanup(&self) -> InterruptedCleanupGuard {
        assert!(!self.interrupted_cleanup.swap(true, Ordering::AcqRel));
        InterruptedCleanupGuard(self.interrupted_cleanup.clone())
    }

    fn interrupted_cleanup(&self) -> bool {
        self.interrupted_cleanup.load(Ordering::Acquire)
    }

    fn record_transition(&self) {
        self.transitions.fetch_add(1, Ordering::AcqRel);
    }

    fn transitions(&self) -> usize {
        self.transitions.load(Ordering::Acquire)
    }

    fn set_cleanup_budget(&self, budget: Duration) {
        self.cleanup_budget_ms
            .store(budget.as_millis() as u64, Ordering::Release);
    }

    fn cleanup_budget(&self) -> Duration {
        Duration::from_millis(self.cleanup_budget_ms.load(Ordering::Acquire))
    }
}

struct RegistryInner {
    state: Mutex<RegistryState>,
    next: AtomicU64,
}

struct RegistryState {
    closing: bool,
    owners: BTreeMap<u64, Arc<Control>>,
}

struct Control {
    cancelled: AtomicBool,
    result: Mutex<Option<oneshot::Sender<Result<String>>>>,
    primary: Mutex<Option<String>>,
}

impl Control {
    fn cancelled_or_closed(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_ref()
                .is_some_and(oneshot::Sender::is_closed)
    }

    fn record_primary(&self, result: &Result<(Capture, Capture)>) {
        if let Err(error) = result {
            *self
                .primary
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(format!("{error:#}"));
        }
    }

    fn primary(&self) -> Option<String> {
        self.primary
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ShellRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                state: Mutex::new(RegistryState {
                    closing: false,
                    owners: BTreeMap::new(),
                }),
                next: AtomicU64::new(1),
            }),
            #[cfg(test)]
            test_hooks: TestHooks::new(),
        }
    }

    pub(crate) async fn execute<F>(
        &self,
        root_guard: Arc<Directory>,
        root: PathBuf,
        command: String,
        duration: Duration,
        environment: F,
    ) -> Result<String>
    where
        F: FnOnce() -> Vec<(std::ffi::OsString, std::ffi::OsString)>
            + Send
            + 'static
            + std::panic::UnwindSafe,
    {
        ensure!(!command.trim().is_empty(), "shell command is empty");
        let accepted = Instant::now();
        let deadline = accepted + duration;
        let cleanup_allowance = self.cleanup_allowance();
        let fallback = deadline + cleanup_allowance;
        let (sender, receiver) = oneshot::channel();
        let control = Arc::new(Control {
            cancelled: AtomicBool::new(false),
            result: Mutex::new(Some(sender)),
            primary: Mutex::new(None),
        });
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            ensure!(!state.closing, "shell host is shutting down");
            state.owners.insert(id, control.clone());
        }
        let weak = Arc::downgrade(&self.inner);
        let request = WorkerRequest {
            root_guard,
            root,
            command,
            environment: Box::new(environment),
            deadline,
            cleanup_allowance,
            #[cfg(test)]
            test_hooks: self.test_hooks.clone(),
        };
        let worker_control = control.clone();
        let start = move || worker(weak, id, worker_control, request);
        #[cfg(test)]
        if self.test_hooks.take(TestPoint::ThreadStartFailure) {
            remove(&self.inner, id);
            bail!("cannot start retained shell owner: injected thread-start failure");
        }
        if let Err(error) = thread::Builder::new()
            .name("kuru-unix-shell".into())
            .spawn(start)
        {
            remove(&self.inner, id);
            bail!("cannot start retained shell owner: {error}");
        }

        match timeout_at(tokio::time::Instant::from_std(fallback), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow::anyhow!("shell owner ended without a result")),
            Err(_) => {
                self.cancel(id);
                if let Some(primary) = control.primary() {
                    bail!(
                        "{primary}; shell cleanup was not confirmed by deadline; unconfirmed owners remain retained"
                    );
                }
                bail!(
                    "shell cleanup was not confirmed by deadline; unconfirmed owners remain retained"
                )
            }
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<()> {
        let controls = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.closing = true;
            state.owners.values().cloned().collect::<Vec<_>>()
        };
        for control in controls {
            control.cancelled.store(true, Ordering::Release);
        }
        let deadline = Instant::now() + self.cleanup_allowance();
        loop {
            if self
                .inner
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .owners
                .is_empty()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "shell cleanup was not confirmed by deadline; unconfirmed owners remain retained"
                );
            }
            sleep(OBSERVE_INTERVAL).await;
        }
    }

    pub(crate) fn cancel_all(&self) {
        let controls = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .owners
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for control in controls {
            control.cancelled.store(true, Ordering::Release);
        }
    }

    fn cancel(&self, id: u64) {
        if let Some(control) = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .owners
            .get(&id)
        {
            control.cancelled.store(true, Ordering::Release);
        }
    }

    fn cleanup_allowance(&self) -> Duration {
        #[cfg(test)]
        {
            self.test_hooks.cleanup_budget()
        }
        #[cfg(not(test))]
        {
            CLEANUP_ALLOWANCE
        }
    }

    #[cfg(test)]
    fn owner_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .owners
            .len()
    }

    #[cfg(test)]
    pub(crate) fn test_owner_count(&self) -> usize {
        self.owner_count()
    }

    #[cfg(test)]
    pub(crate) fn test_arm_start_gate(&self) -> TestGateGuard {
        self.test_hooks.arm_start_gate()
    }

    #[cfg(test)]
    fn test_arm_interrupted_cleanup(&self) -> InterruptedCleanupGuard {
        self.test_hooks.arm_interrupted_cleanup()
    }

    #[cfg(test)]
    fn test_set_cleanup_budget(&self, budget: Duration) {
        self.test_hooks.set_cleanup_budget(budget);
    }

    #[cfg(test)]
    fn test_transitions(&self) -> usize {
        self.test_hooks.transitions()
    }

    #[cfg(test)]
    pub(crate) fn test_is_closing(&self) -> bool {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .closing
    }
}

impl Drop for ShellRegistry {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

struct WorkerRequest {
    root_guard: Arc<Directory>,
    root: PathBuf,
    command: String,
    environment: Box<
        dyn FnOnce() -> Vec<(std::ffi::OsString, std::ffi::OsString)>
            + Send
            + std::panic::UnwindSafe,
    >,
    deadline: Instant,
    cleanup_allowance: Duration,
    #[cfg(test)]
    test_hooks: TestHooks,
}

fn worker(registry: Weak<RegistryInner>, id: u64, control: Arc<Control>, request: WorkerRequest) {
    let WorkerRequest {
        root_guard,
        root,
        command,
        environment,
        deadline,
        cleanup_allowance,
        #[cfg(test)]
        test_hooks,
    } = request;
    let finish = WorkerFinish {
        registry,
        id,
        control: control.clone(),
        confirmed: AtomicBool::new(false),
        spawned: AtomicBool::new(false),
    };
    #[cfg(test)]
    test_hooks.await_start();
    if let Some(error) = before_launch(&control, deadline) {
        finish.complete(Err(error));
        return;
    }
    let environment = match catch_unwind(environment) {
        Ok(environment) => environment,
        Err(_) => {
            finish.complete(Err(anyhow::anyhow!(
                "shell environment projection panicked before launch"
            )));
            return;
        }
    };
    if let Some(error) = before_launch(&control, deadline) {
        finish.complete(Err(error));
        return;
    }
    #[cfg(test)]
    if test_hooks.take(TestPoint::RuntimeStartFailure) {
        finish.complete(Err(anyhow::anyhow!(
            "create retained shell runtime: injected runtime-start failure"
        )));
        return;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create retained shell runtime")
    {
        Ok(runtime) => runtime,
        Err(error) => {
            finish.complete(Err(error));
            return;
        }
    };
    if let Some(error) = before_launch(&control, deadline) {
        finish.complete(Err(error));
        return;
    }
    if let Err(error) = root_guard.revalidate() {
        finish.complete(Err(error.into()));
        return;
    }

    let mut process = Command::new("sh");
    process
        .arg("-c")
        .arg(command)
        .env_clear()
        .envs(environment)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(error) = before_launch(&control, deadline) {
        finish.complete(Err(error));
        return;
    }
    if let Err(error) = root_guard.revalidate() {
        finish.complete(Err(error.into()));
        return;
    }
    #[cfg(test)]
    test_hooks.panic_if_armed(TestPoint::PreSpawnPanic);
    let mut group = match OwnedProcessGroup::spawn(process).context("cannot start shell") {
        Ok(group) => group,
        Err(error) => {
            finish.complete(Err(error));
            return;
        }
    };
    finish.spawned.store(true, Ordering::Release);

    // Keep `group` outside the panic boundary. A post-spawn panic can then use
    // the same signal-before-reap path instead of dropping the anchored child.
    // All setup and capture work remains inside one boundary while `group`
    // stays outside it. A conversion, read, or runtime panic therefore drops
    // any converted pipes before the common owner-preserving cleanup below.
    let session = catch_unwind(AssertUnwindSafe(|| {
        let _entered = runtime.enter();
        let mut stdout = group
            .take_stdout()
            .and_then(tokio::process::ChildStdout::from_std)
            .context("missing shell stdout")?;
        #[cfg(test)]
        if test_hooks.take(TestPoint::PartialPipeConversionFailure) {
            bail!("injected partial shell pipe conversion failure");
        }
        let mut stderr = group
            .take_stderr()
            .and_then(tokio::process::ChildStderr::from_std)
            .context("missing shell stderr")?;
        #[cfg(test)]
        test_hooks.panic_if_armed(TestPoint::PostSpawnPanic);
        let result = runtime.block_on(read_until_terminal(
            &mut group,
            &mut stdout,
            &mut stderr,
            &control,
            deadline,
            #[cfg(test)]
            &test_hooks,
        ));
        Ok::<_, anyhow::Error>((result, Some(stdout), Some(stderr)))
    }));
    let (result, stdout, stderr) = match session {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => (Err(error), None, None),
        Err(_) => (
            Err(anyhow::anyhow!(
                "retained shell worker panicked after spawn"
            )),
            None,
            None,
        ),
    };
    let primary_description = result.as_ref().err().map(|error| format!("{error:#}"));
    control.record_primary(&result);
    let finished = catch_unwind(AssertUnwindSafe(|| {
        finish_with_cleanup(
            &mut group,
            result,
            stdout,
            stderr,
            &finish,
            cleanup_allowance,
            #[cfg(test)]
            &test_hooks,
        )
    }));
    match finished {
        Ok(true) => {}
        Ok(false) => retain_until_confirmed(
            &mut group,
            &finish,
            cleanup_allowance,
            #[cfg(test)]
            &test_hooks,
        ),
        Err(_) => {
            let error = match primary_description {
                Some(primary) => anyhow::anyhow!(
                    "{primary}; shell cleanup was not confirmed; unconfirmed ownership is retained after cleanup panic"
                ),
                None => anyhow::anyhow!(
                    "shell cleanup was not confirmed; unconfirmed ownership is retained after cleanup panic"
                ),
            };
            finish.report(Err(error));
            retain_until_confirmed(
                &mut group,
                &finish,
                cleanup_allowance,
                #[cfg(test)]
                &test_hooks,
            );
        }
    }
}

struct WorkerFinish {
    registry: Weak<RegistryInner>,
    id: u64,
    control: Arc<Control>,
    confirmed: AtomicBool,
    spawned: AtomicBool,
}

impl WorkerFinish {
    fn report(&self, result: Result<String>) {
        if let Some(sender) = self
            .control
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = sender.send(result);
        }
    }

    fn complete(&self, result: Result<String>) {
        self.report(result);
        self.confirmed.store(true, Ordering::Release);
    }

    fn confirm(&self) {
        self.confirmed.store(true, Ordering::Release);
    }
}

impl Drop for WorkerFinish {
    fn drop(&mut self) {
        if !self.spawned.load(Ordering::Acquire) && !self.confirmed.load(Ordering::Acquire) {
            self.report(Err(anyhow::anyhow!(
                "retained shell worker panicked before launch"
            )));
            self.confirmed.store(true, Ordering::Release);
        }
        if (!self.spawned.load(Ordering::Acquire) || self.confirmed.load(Ordering::Acquire))
            && let Some(registry) = self.registry.upgrade()
        {
            remove(&registry, self.id);
        }
    }
}

async fn read_until_terminal(
    group: &mut OwnedProcessGroup,
    stdout: &mut tokio::process::ChildStdout,
    stderr: &mut tokio::process::ChildStderr,
    control: &Control,
    deadline: Instant,
    #[cfg(test)] test_hooks: &TestHooks,
) -> Result<(Capture, Capture)> {
    let mut out = Capture::new();
    let mut err = Capture::new();
    {
        let out_read = out.read(stdout);
        let err_read = err.read(stderr);
        tokio::pin!(out_read);
        tokio::pin!(err_read);
        let mut out_done = false;
        let mut err_done = false;
        let mut root_exited = false;
        loop {
            #[cfg(test)]
            if test_hooks.take(TestPoint::PipeReadFailure) {
                bail!("injected shell pipe read failure");
            }
            if control.cancelled_or_closed() {
                bail!("shell cancelled");
            }
            if Instant::now() >= deadline {
                bail!("shell timed out");
            }
            match group.root_state() {
                RootState::Disarmed(reason) => bail!("shell ownership lost: {reason:?}"),
                RootState::Reaped(_) => bail!("shell root was reaped before cleanup"),
                RootState::Exited => root_exited = true,
                RootState::Running | RootState::Interrupted => {}
            }
            if out_done && err_done && root_exited {
                break;
            }
            tokio::select! {
                result = &mut out_read, if !out_done => {
                    result.map_err(|error| anyhow::anyhow!("read shell stdout: {error:#}"))?;
                    out_done = true;
                }
                result = &mut err_read, if !err_done => {
                    result.map_err(|error| anyhow::anyhow!("read shell stderr: {error:#}"))?;
                    err_done = true;
                }
                _ = sleep(OBSERVE_INTERVAL) => {}
            }
        }
    }
    out.finish()?;
    err.finish()?;
    Ok((out, err))
}

fn finish_with_cleanup(
    group: &mut OwnedProcessGroup,
    primary: Result<(Capture, Capture)>,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    finish: &WorkerFinish,
    cleanup_allowance: Duration,
    #[cfg(test)] test_hooks: &TestHooks,
) -> bool {
    drop(stdout);
    drop(stderr);
    #[cfg(test)]
    test_hooks.panic_if_armed(TestPoint::CleanupPanic);
    let cleanup_deadline = Instant::now() + cleanup_allowance;
    match cleanup_until(
        group,
        cleanup_deadline,
        #[cfg(test)]
        test_hooks,
    ) {
        Ok(status) => {
            let result = primary.map(|(out, err)| {
                json!({
                    "exit_code": status.code(),
                    "success": status.success(),
                    "stdout": out.text,
                    "stderr": err.text,
                })
                .to_string()
            });
            finish.complete(result);
            true
        }
        Err(error) => {
            let result = match primary {
                Ok(_) => Err(anyhow::anyhow!(
                    "shell cleanup was not confirmed; unconfirmed ownership is retained: {error:#}"
                )),
                Err(primary) => Err(anyhow::anyhow!(
                    "{primary:#}; shell cleanup was not confirmed; unconfirmed ownership is retained: {error:#}"
                )),
            };
            finish.report(result);
            false
        }
    }
}

fn retain_until_confirmed(
    group: &mut OwnedProcessGroup,
    finish: &WorkerFinish,
    cleanup_allowance: Duration,
    #[cfg(test)] test_hooks: &TestHooks,
) {
    // Once the caller receives its bounded result the worker remains the owner.
    // A post-signal platform value cannot signal again; an anchored value may
    // still make its one initial transition after a later non-interrupted poll.
    loop {
        #[cfg(test)]
        test_hooks.await_retained_cleanup();
        if let Ok(Ok(_)) = catch_unwind(AssertUnwindSafe(|| {
            cleanup_until(
                group,
                Instant::now() + cleanup_allowance,
                #[cfg(test)]
                test_hooks,
            )
        })) {
            finish.confirm();
            return;
        }
        thread::sleep(OBSERVE_INTERVAL);
    }
}

fn before_launch(control: &Control, deadline: Instant) -> Option<anyhow::Error> {
    if control.cancelled_or_closed() {
        Some(anyhow::anyhow!("shell cancelled before launch"))
    } else if Instant::now() >= deadline {
        Some(anyhow::anyhow!("shell timed out before launch"))
    } else {
        None
    }
}

fn cleanup_until(
    group: &mut OwnedProcessGroup,
    deadline: Instant,
    #[cfg(test)] test_hooks: &TestHooks,
) -> Result<ExitStatus> {
    loop {
        #[cfg(test)]
        let termination = if test_hooks.interrupted_cleanup() {
            Termination::Interrupted
        } else {
            group.terminate_before_reap()
        };
        #[cfg(not(test))]
        let termination = group.terminate_before_reap();
        match termination {
            Termination::Signalled(_) => {
                #[cfg(test)]
                test_hooks.record_transition();
            }
            Termination::InvalidPhase => {}
            Termination::Interrupted => {
                if Instant::now() >= deadline {
                    bail!("ownership observation interrupted")
                }
                thread::sleep(OBSERVE_INTERVAL);
                continue;
            }
            Termination::Disarmed(reason) => bail!("shell ownership lost: {reason:?}"),
        }
        match group.reap_if_exited() {
            Reap::Reaped(status) => match group.presence_after_reap() {
                GroupPresence::Absent => return Ok(status),
                GroupPresence::Present | GroupPresence::PermissionDenied => {}
                GroupPresence::ObservationError(kind) => {
                    bail!("shell group observation failed: {kind}")
                }
                GroupPresence::InvalidPhase => {
                    bail!("shell group observation occurred before reap")
                }
            },
            Reap::NotExited | Reap::Interrupted => {}
            Reap::Disarmed(reason) => bail!("shell ownership lost: {reason:?}"),
            Reap::InvalidPhase => bail!("shell root cleanup phase is invalid"),
        }
        if Instant::now() >= deadline {
            bail!("cleanup confirmation timed out")
        }
        thread::sleep(OBSERVE_INTERVAL);
    }
}

fn remove(registry: &RegistryInner, id: u64) {
    registry
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .owners
        .remove(&id);
}

struct Capture {
    output: Option<crate::redaction::StreamingProjection>,
    text: String,
}

impl Capture {
    fn new() -> Self {
        Self {
            output: Some(crate::redaction::StreamingProjection::new(MAX_BYTES)),
            text: String::new(),
        }
    }

    async fn read(&mut self, reader: &mut (impl AsyncRead + Unpin)) -> Result<()> {
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer).await?;
            if count == 0 {
                return Ok(());
            }
            self.output
                .as_mut()
                .expect("shell capture was already finished")
                .push(&buffer[..count])?;
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.text = self
            .output
            .take()
            .expect("shell capture was already finished")
            .finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use kuru_platform::fs::{NameRetention, Privacy};

    use super::*;

    fn retained_root(path: &Path) -> Arc<Directory> {
        Arc::new(Directory::open(path, Privacy::Inherited, NameRetention::Pinned).unwrap())
    }

    #[test]
    fn pre_spawn_panic_reclaims_only_its_registered_reservation() {
        let root = tempfile::tempdir().unwrap();
        let registry = ShellRegistry::new();
        registry.test_hooks.set_point(TestPoint::PreSpawnPanic);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let error = runtime
            .block_on(registry.execute(
                retained_root(root.path()),
                root.path().to_path_buf(),
                ": > launched-after-panic".into(),
                Duration::from_secs(1),
                Vec::new,
            ))
            .unwrap_err();

        assert!(error.to_string().contains("panicked before launch"));
        assert_eq!(registry.owner_count(), 0);
        assert!(!root.path().join("launched-after-panic").exists());
    }

    #[test]
    fn cleanup_panic_keeps_the_timeout_primary_and_retains_the_owner() {
        let root = tempfile::tempdir().unwrap();
        let registry = ShellRegistry::new();
        registry.test_hooks.set_point(TestPoint::CleanupPanic);
        let retained_cleanup = registry.test_hooks.arm_retained_cleanup_gate();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let error = runtime
            .block_on(registry.execute(
                retained_root(root.path()),
                root.path().to_path_buf(),
                "sleep 5".into(),
                Duration::from_millis(30),
                Vec::new,
            ))
            .unwrap_err()
            .to_string();

        assert!(error.contains("shell timed out"), "{error}");
        assert!(error.contains("cleanup panic"), "{error}");
        assert_eq!(registry.owner_count(), 1);
        retained_cleanup.release();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while registry.owner_count() != 0 {
                    sleep(OBSERVE_INTERVAL).await;
                }
            })
            .await
            .expect("retained owner did not confirm cleanup");
        });
    }

    #[test]
    fn before_spawn_failures_reclaim_their_reservation_without_launching() {
        for (point, expected) in [
            (TestPoint::ThreadStartFailure, "thread-start failure"),
            (TestPoint::RuntimeStartFailure, "runtime-start failure"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let registry = ShellRegistry::new();
            registry.test_hooks.set_point(point);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let error = runtime
                .block_on(registry.execute(
                    retained_root(root.path()),
                    root.path().to_path_buf(),
                    ": > launched-before-spawn-failure".into(),
                    Duration::from_secs(1),
                    Vec::new,
                ))
                .unwrap_err();

            assert!(error.to_string().contains(expected), "{error:#}");
            assert_eq!(registry.owner_count(), 0);
            assert!(!root.path().join("launched-before-spawn-failure").exists());
        }
    }

    #[test]
    fn post_spawn_failures_take_the_common_cleanup_path() {
        for (point, expected) in [
            (
                TestPoint::PartialPipeConversionFailure,
                "partial shell pipe conversion failure",
            ),
            (TestPoint::PostSpawnPanic, "worker panicked after spawn"),
            (TestPoint::PipeReadFailure, "pipe read failure"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let registry = ShellRegistry::new();
            registry.test_hooks.set_point(point);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let error = runtime
                .block_on(registry.execute(
                    retained_root(root.path()),
                    root.path().to_path_buf(),
                    "exec sleep 5".into(),
                    Duration::from_secs(1),
                    Vec::new,
                ))
                .unwrap_err();

            assert!(error.to_string().contains(expected), "{error:#}");
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while registry.owner_count() != 0 {
                        sleep(OBSERVE_INTERVAL).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{point:?} retained its owner: {error:#}"));
            });
        }
    }

    #[test]
    fn caller_fallback_prevents_a_delayed_worker_from_launching() {
        let root = tempfile::tempdir().unwrap();
        let registry = ShellRegistry::new();
        let start = registry.test_hooks.arm_start_gate();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let accepted = Instant::now();

        let error = runtime
            .block_on(registry.execute(
                retained_root(root.path()),
                root.path().to_path_buf(),
                ": > launched-after-caller-fallback".into(),
                Duration::from_millis(20),
                Vec::new,
            ))
            .unwrap_err();

        assert!(accepted.elapsed() >= CLEANUP_ALLOWANCE);
        assert!(error.to_string().contains("not confirmed by deadline"));
        assert_eq!(registry.owner_count(), 1);
        start.release();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while registry.owner_count() != 0 {
                    sleep(OBSERVE_INTERVAL).await;
                }
            })
            .await
            .expect("late worker did not reclaim its reservation");
        });
        assert!(!root.path().join("launched-after-caller-fallback").exists());
    }

    #[test]
    fn shutdown_cancels_a_starting_owner_and_closes_registration() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ShellRegistry::new());
        let start = registry.test_hooks.arm_start_gate();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async {
            let starting = tokio::spawn({
                let registry = registry.clone();
                let root = root.path().to_path_buf();
                async move {
                    registry
                        .execute(
                            retained_root(&root),
                            root,
                            ": > launched-during-shutdown".into(),
                            Duration::from_secs(1),
                            Vec::new,
                        )
                        .await
                }
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                while registry.owner_count() != 1 {
                    sleep(OBSERVE_INTERVAL).await;
                }
            })
            .await
            .expect("starting owner was not registered");

            let shutdown = tokio::spawn({
                let registry = registry.clone();
                async move { registry.shutdown().await }
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                while !registry.test_is_closing() {
                    sleep(OBSERVE_INTERVAL).await;
                }
            })
            .await
            .expect("shutdown did not close shell registration");
            let rejected = registry
                .execute(
                    retained_root(root.path()),
                    root.path().to_path_buf(),
                    ": > launched-after-shutdown".into(),
                    Duration::from_secs(1),
                    Vec::new,
                )
                .await
                .unwrap_err();
            assert!(rejected.to_string().contains("shutting down"));

            start.release();
            shutdown.await.unwrap().unwrap();
            assert!(
                starting
                    .await
                    .unwrap()
                    .unwrap_err()
                    .to_string()
                    .contains("cancelled")
            );
        });
        assert!(!root.path().join("launched-during-shutdown").exists());
        assert!(!root.path().join("launched-after-shutdown").exists());
    }

    #[test]
    fn delayed_interruption_retains_the_real_worker_then_confirms_once() {
        let root = tempfile::tempdir().unwrap();
        let registry = ShellRegistry::new();
        registry.test_set_cleanup_budget(Duration::from_millis(80));
        let interruption = registry.test_arm_interrupted_cleanup();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = Instant::now();

        let error = runtime
            .block_on(registry.execute(
                retained_root(root.path()),
                root.path().to_path_buf(),
                "exec sleep 5".into(),
                Duration::from_millis(25),
                Vec::new,
            ))
            .unwrap_err()
            .to_string();

        assert!(started.elapsed() >= Duration::from_millis(80));
        assert!(error.contains("shell timed out"), "{error}");
        assert!(error.contains("cleanup was not confirmed"), "{error}");
        assert_eq!(registry.owner_count(), 1);
        assert_eq!(registry.test_transitions(), 0);
        interruption.release();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while registry.owner_count() != 0 {
                    sleep(OBSERVE_INTERVAL).await;
                }
            })
            .await
            .expect("retained real worker did not confirm cleanup");
        });
        assert_eq!(registry.test_transitions(), 1);
    }

    #[test]
    fn successful_capture_never_reports_success_before_unconfirmed_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let registry = ShellRegistry::new();
        registry.test_set_cleanup_budget(Duration::from_millis(80));
        let interruption = registry.test_arm_interrupted_cleanup();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let error = runtime
            .block_on(registry.execute(
                retained_root(root.path()),
                root.path().to_path_buf(),
                "true".into(),
                Duration::from_secs(1),
                Vec::new,
            ))
            .unwrap_err()
            .to_string();

        assert!(error.contains("cleanup was not confirmed"), "{error}");
        assert!(!error.contains("exit_code"), "{error}");
        assert_eq!(registry.owner_count(), 1);
        assert_eq!(registry.test_transitions(), 0);
        interruption.release();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while registry.owner_count() != 0 {
                    sleep(OBSERVE_INTERVAL).await;
                }
            })
            .await
            .expect("successful shell owner did not later confirm cleanup");
        });
        assert_eq!(registry.test_transitions(), 1);
    }
}
