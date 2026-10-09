//! Bounded one-shot lifecycle hook protocol and owned command execution.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use kuru_core::{HookCommand, HookEvent, LifecycleHooks};
use kuru_platform::fs::Directory;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{oneshot, watch},
};

use crate::redaction;

#[cfg(unix)]
use kuru_platform::unix::{
    GroupPresence, OwnedProcessGroup, PreReap, Reap, RootState, Termination,
};
#[cfg(windows)]
use kuru_platform::windows::process::{NativeChild, Stdio as NativeStdio};

const FORMAT: u16 = 1;
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ANNOTATION_BYTES: usize = 16 * 1024;
/// Largest pre-turn input a hook may return; equal to the admitted prompt bound.
pub const MAX_PRE_TURN_INPUT_BYTES: usize = 131_072;
const MAX_TOOL_NAME_BYTES: usize = 256;
/// One owned cleanup deadline: terminate, reap, group absence and pipe drains.
/// After the root exits, reaping and the output drain share one such deadline.
const CLEANUP: Duration = Duration::from_secs(5);
/// A worker observes caller loss at its next poll and then finishes within one
/// `CLEANUP` deadline (before or after root exit); the equal margin covers that
/// observation and thread scheduling. Exceeding it is reported as unconfirmed
/// cleanup.
const QUIESCE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(10);
/// Only an owned hook launch adds this marker to its finite child environment.
pub const HOOK_ORIGIN_ENV: &str = "KURU_INTERNAL_LIFECYCLE_HOOK_ORIGIN";
#[cfg(unix)]
type HookInput = tokio::process::ChildStdin;
#[cfg(unix)]
type HookOutput = tokio::process::ChildStdout;
#[cfg(unix)]
type HookError = tokio::process::ChildStderr;
#[cfg(unix)]
type HookOwner = OwnedProcessGroup;

#[cfg(windows)]
type HookInput = kuru_platform::windows::pipe::Pipe;
#[cfg(windows)]
type HookOutput = kuru_platform::windows::pipe::Pipe;
#[cfg(windows)]
type HookError = kuru_platform::windows::pipe::Pipe;
#[cfg(windows)]
type HookOwner = NativeChild;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct HookRequest<'a> {
    format: u16,
    event: HookEvent,
    invocation_id: &'a str,
    actor: &'a str,
    turn_id: Option<&'a str>,
    call_id: Option<&'a str>,
    payload: &'a Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
enum HookResponse {
    Allow {},
    Observe {},
    Deny { reason: Option<String> },
    Rewrite { value: Value },
    Stop { reason: Option<String> },
    Annotate { annotation: String },
}

/// The exact pre-turn value a hook receives and may return as a rewrite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreTurnValue {
    pub input: String,
}

/// The exact pre-tool value a hook receives and may return as a rewrite.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreToolValue {
    pub name: String,
    pub arguments: Value,
}

impl PreTurnValue {
    /// Decode and bound one pre-turn value. Connector chains and the runtime
    /// share this check so the limits cannot diverge.
    pub fn checked(value: Value) -> Result<Self> {
        let payload: Self =
            serde_json::from_value(value).context("pre-turn hook returned an invalid input")?;
        ensure!(
            !payload.input.trim().is_empty() && payload.input.len() <= MAX_PRE_TURN_INPUT_BYTES,
            "pre-turn hook returned an invalid input"
        );
        Ok(payload)
    }
}

impl PreToolValue {
    /// Decode and bound one pre-tool value for the proposed tool `name`.
    /// Hooks may rewrite arguments only; a changed tool name fails closed.
    pub fn checked(value: Value, name: &str) -> Result<Self> {
        let payload: Self =
            serde_json::from_value(value).context("pre-tool hook returned an invalid operation")?;
        ensure!(
            !payload.name.trim().is_empty()
                && payload.name.len() <= MAX_TOOL_NAME_BYTES
                && payload.arguments.is_object()
                && serde_json::to_vec(&payload.arguments)?.len() <= crate::MAX_BYTES,
            "pre-tool hook returned an invalid operation"
        );
        ensure!(
            payload.name == name,
            "pre-tool hook may rewrite only the proposed arguments"
        );
        Ok(payload)
    }
}

fn validate_pre_rewrite(event: HookEvent, current: &Value, rewritten: &Value) -> Result<()> {
    match event {
        HookEvent::PreTurn => {
            PreTurnValue::checked(rewritten.clone())?;
        }
        HookEvent::PreTool => {
            let name = current
                .get("name")
                .and_then(Value::as_str)
                .context("pre-tool hook input lacks its proposed tool name")?;
            PreToolValue::checked(rewritten.clone(), name)?;
        }
        _ => bail!("invalid pre-hook event"),
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookAnnotation {
    pub event: HookEvent,
    pub hook_index: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookOutcomeKind {
    Allowed,
    Rewritten,
    Denied,
    Observed,
    Stopped,
    Annotated,
    Failed,
    /// A configured hook that the internal hook-origin marker kept from running.
    Suppressed,
}

impl HookOutcomeKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Rewritten => "rewritten",
            Self::Denied => "denied",
            Self::Observed => "observed",
            Self::Stopped => "stopped",
            Self::Annotated => "annotated",
            Self::Failed => "failed",
            Self::Suppressed => "suppressed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookObservation {
    pub event: HookEvent,
    pub hook_index: usize,
    pub outcome: HookOutcomeKind,
}

#[derive(Debug)]
pub struct PreHookRun {
    pub outcome: Result<PreHookOutcome>,
    pub observations: Vec<HookObservation>,
}

#[derive(Debug)]
pub struct PostHookRun {
    pub annotations: Vec<HookAnnotation>,
    pub failures: Vec<String>,
    pub observations: Vec<HookObservation>,
}

#[derive(Debug)]
pub struct SpeakerHookRun {
    pub outcome: Result<SpeakerHookOutcome>,
    pub observations: Vec<HookObservation>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreHookOutcome {
    Allowed(Value),
    Denied(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeakerHookOutcome {
    Continue,
    Stop(String),
}

#[derive(Clone)]
pub struct HookHost {
    root: Arc<Directory>,
    hooks: LifecycleHooks,
    /// Configured hooks are reported, never run, below an owned hook launch.
    suppressed: bool,
    workers: Arc<HookWorkers>,
}

/// In-flight owned hook workers. A worker leaves only after its process tree
/// was reaped (or its cleanup failed), so awaiting zero awaits cleanup even
/// when the caller that started the worker was dropped.
struct HookWorkers {
    state: watch::Sender<WorkerState>,
}

#[derive(Clone, Copy, Default)]
struct WorkerState {
    active: usize,
    uncertain: bool,
}

struct WorkerSlot(Arc<HookWorkers>);

impl HookWorkers {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: watch::channel(WorkerState::default()).0,
        })
    }

    fn enter(self: &Arc<Self>) -> WorkerSlot {
        self.state.send_modify(|state| state.active += 1);
        WorkerSlot(self.clone())
    }
}

impl WorkerSlot {
    fn cleanup_unconfirmed(&self) {
        self.0.state.send_modify(|state| state.uncertain = true);
    }
}

impl Drop for WorkerSlot {
    fn drop(&mut self) {
        self.0.state.send_modify(|state| state.active -= 1);
    }
}

/// Cleanup of an owned hook tree could not be confirmed.
#[derive(Debug)]
struct CleanupUnconfirmed;

impl std::fmt::Display for CleanupUnconfirmed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("lifecycle hook cleanup could not be confirmed")
    }
}

impl std::error::Error for CleanupUnconfirmed {}

type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

struct BudgetState {
    remaining: Duration,
    active_since: Option<Instant>,
    active: usize,
    invocations: usize,
    annotation_bytes: usize,
}

/// One operation's shared execution budget. Overlapping commands spend wall time once.
pub struct HookBudget {
    state: Mutex<BudgetState>,
    max_invocations: usize,
    max_annotation_bytes: usize,
    clock: Clock,
    _hold: Option<crate::InvocationHold>,
}

struct HookLease(Arc<HookBudget>);

impl HookBudget {
    pub fn new(hooks: &LifecycleHooks) -> Arc<Self> {
        Self::with_clock(hooks, Arc::new(Instant::now))
    }

    fn with_clock(hooks: &LifecycleHooks, clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(BudgetState {
                remaining: Duration::from_millis(hooks.max_total_ms),
                active_since: None,
                active: 0,
                invocations: 0,
                annotation_bytes: 0,
            }),
            max_invocations: hooks.max_invocations,
            max_annotation_bytes: hooks.max_annotation_bytes,
            clock,
            _hold: None,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BudgetState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn tick(state: &mut BudgetState, now: Instant) {
        if let Some(since) = state.active_since {
            state.remaining = state
                .remaining
                .saturating_sub(now.saturating_duration_since(since));
            state.active_since = Some(now);
        }
    }

    fn claim(self: &Arc<Self>) -> Result<(HookLease, Instant)> {
        let mut state = self.lock();
        let now = (self.clock)();
        Self::tick(&mut state, now);
        ensure!(
            state.invocations < self.max_invocations,
            "hook invocation budget exhausted"
        );
        ensure!(
            !state.remaining.is_zero(),
            "hook execution time budget exhausted"
        );
        state.invocations += 1;
        state.active += 1;
        state.active_since = Some(now);
        Ok((HookLease(self.clone()), now + state.remaining))
    }

    fn reserve_annotation(&self, bytes: usize) -> Result<()> {
        let mut state = self.lock();
        ensure!(
            bytes
                <= self
                    .max_annotation_bytes
                    .saturating_sub(state.annotation_bytes),
            "hook annotation budget exhausted"
        );
        state.annotation_bytes += bytes;
        Ok(())
    }
}

impl Drop for HookLease {
    fn drop(&mut self) {
        let now = (self.0.clock)();
        let mut state = self.0.lock();
        HookBudget::tick(&mut state, now);
        state.active -= 1;
        if state.active == 0 {
            state.active_since = None;
        }
    }
}

impl HookHost {
    pub fn new(root: Arc<Directory>, hooks: LifecycleHooks) -> Self {
        Self::with_origin(root, hooks, std::env::var_os(HOOK_ORIGIN_ENV).as_deref())
    }

    fn with_origin(
        root: Arc<Directory>,
        hooks: LifecycleHooks,
        origin: Option<&std::ffi::OsStr>,
    ) -> Self {
        // Only the owned hook launch adds this marker to its finite child
        // environment. A Kuru process started by that command still performs
        // its ordinary admission, trust, and tool checks, but cannot start a
        // second lifecycle-hook chain from that nested operation. Its
        // configured hooks stay visible as suppressed outcomes instead of
        // silently disappearing.
        let suppressed = origin == Some(std::ffi::OsStr::new("1"));
        Self {
            root,
            hooks,
            suppressed,
            workers: HookWorkers::new(),
        }
    }

    pub fn configured(&self, event: HookEvent) -> bool {
        !self.hooks.event(event).is_empty()
    }

    /// Whether configured hooks are reported as suppressed instead of run.
    pub fn suppressed(&self) -> bool {
        self.suppressed
    }

    pub fn budget(&self) -> Arc<HookBudget> {
        HookBudget::new(&self.hooks)
    }

    /// One operation's immutable ownership is retained by each HookLease;
    /// neither this reusable host nor another session can replace it.
    pub fn budget_with_lifetime(&self, hold: crate::InvocationHold) -> Arc<HookBudget> {
        let mut budget = HookBudget::with_clock(&self.hooks, Arc::new(Instant::now));
        Arc::get_mut(&mut budget)
            .expect("new private hook budget")
            ._hold = Some(hold);
        budget
    }

    /// Await every in-flight owned hook worker, including one whose caller
    /// was cancelled or dropped, through its bounded tree cleanup. Reports
    /// cleanup that failed or did not finish within its derived bound; a
    /// reported uncertainty is cleared once returned.
    pub async fn quiesce(&self) -> Result<()> {
        let mut state = self.workers.state.subscribe();
        let settled = tokio::time::timeout(QUIESCE, state.wait_for(|state| state.active == 0))
            .await
            .map_err(|_| anyhow::Error::new(CleanupUnconfirmed))
            .context("lifecycle hook cleanup exceeded its bound")?
            .map(|_| ())
            .context("lifecycle hook worker tracking stopped");
        let mut uncertain = false;
        self.workers.state.send_if_modified(|state| {
            uncertain = std::mem::take(&mut state.uncertain);
            uncertain
        });
        settled?;
        if uncertain {
            return Err(anyhow::Error::new(CleanupUnconfirmed));
        }
        Ok(())
    }

    /// Owned hook workers whose process-tree cleanup has not finished.
    #[cfg(any(test, feature = "test-support"))]
    pub fn in_flight_hooks(&self) -> usize {
        self.workers.state.borrow().active
    }

    /// The bound [`HookHost::quiesce`] waits before reporting unconfirmed
    /// cleanup. A fixture that awaits an operation enclosing `quiesce` derives
    /// its own bound from this value instead of restating it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn quiesce_bound(&self) -> Duration {
        QUIESCE
    }

    fn suppressed_observations(&self, event: HookEvent) -> Vec<HookObservation> {
        (0..self.hooks.event(event).len())
            .map(|index| observation(event, index, HookOutcomeKind::Suppressed))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_pre(
        &self,
        budget: &Arc<HookBudget>,
        event: HookEvent,
        invocation_id: &str,
        actor: &str,
        turn_id: Option<&str>,
        call_id: Option<&str>,
        mut value: Value,
    ) -> PreHookRun {
        if !matches!(event, HookEvent::PreTurn | HookEvent::PreTool) {
            return PreHookRun {
                outcome: Err(anyhow::anyhow!("invalid pre-hook event")),
                observations: vec![],
            };
        }
        if self.suppressed {
            return PreHookRun {
                outcome: Ok(PreHookOutcome::Allowed(value)),
                observations: self.suppressed_observations(event),
            };
        }
        let mut observations = Vec::new();
        for (index, hook) in self.hooks.event(event).iter().enumerate() {
            let response = match invoke(
                self.root.clone(),
                self.workers.clone(),
                hook.clone(),
                budget.clone(),
                HookRequest {
                    format: FORMAT,
                    event,
                    invocation_id,
                    actor,
                    turn_id,
                    call_id,
                    payload: &value,
                },
            )
            .await
            .with_context(|| format!("{} hook {} failed", event.label(), index + 1))
            {
                Ok(response) => response,
                Err(error) => {
                    observations.push(observation(event, index, HookOutcomeKind::Failed));
                    return PreHookRun {
                        outcome: Err(error),
                        observations,
                    };
                }
            };
            match response {
                HookResponse::Allow {} => {
                    observations.push(observation(event, index, HookOutcomeKind::Allowed));
                }
                HookResponse::Rewrite { value: rewritten } => {
                    let checked = validate_pre_rewrite(event, &value, &rewritten).and_then(|()| {
                        let request = serde_json::to_vec(&HookRequest {
                            format: FORMAT,
                            event,
                            invocation_id,
                            actor,
                            turn_id,
                            call_id,
                            payload: &rewritten,
                        })?;
                        ensure!(
                            request.len() <= MAX_REQUEST_BYTES,
                            "pre-hook rewritten request exceeds its bound"
                        );
                        Ok(())
                    });
                    if let Err(error) = checked {
                        observations.push(observation(event, index, HookOutcomeKind::Failed));
                        return PreHookRun {
                            outcome: Err(error),
                            observations,
                        };
                    }
                    value = rewritten;
                    observations.push(observation(event, index, HookOutcomeKind::Rewritten));
                }
                HookResponse::Deny { reason } => {
                    let outcome =
                        project_reason(reason, "hook denied dispatch").map(PreHookOutcome::Denied);
                    observations.push(observation(
                        event,
                        index,
                        if outcome.is_ok() {
                            HookOutcomeKind::Denied
                        } else {
                            HookOutcomeKind::Failed
                        },
                    ));
                    return PreHookRun {
                        outcome,
                        observations,
                    };
                }
                HookResponse::Observe {}
                | HookResponse::Stop { .. }
                | HookResponse::Annotate { .. } => {
                    observations.push(observation(event, index, HookOutcomeKind::Failed));
                    return PreHookRun {
                        outcome: Err(anyhow::anyhow!("hook returned an invalid decision")),
                        observations,
                    };
                }
            }
        }
        PreHookRun {
            outcome: Ok(PreHookOutcome::Allowed(value)),
            observations,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_post(
        &self,
        budget: &Arc<HookBudget>,
        event: HookEvent,
        invocation_id: &str,
        actor: &str,
        turn_id: Option<&str>,
        call_id: Option<&str>,
        value: &Value,
    ) -> PostHookRun {
        if !matches!(event, HookEvent::PostTurn | HookEvent::PostTool) {
            return PostHookRun {
                annotations: vec![],
                failures: vec!["invalid post-hook event".into()],
                observations: vec![],
            };
        }
        if self.suppressed {
            return PostHookRun {
                annotations: vec![],
                failures: vec![],
                observations: self.suppressed_observations(event),
            };
        }
        let mut annotations = Vec::new();
        let mut failures = Vec::new();
        let mut observations = Vec::with_capacity(self.hooks.event(event).len());
        for (index, hook) in self.hooks.event(event).iter().enumerate() {
            let result = invoke(
                self.root.clone(),
                self.workers.clone(),
                hook.clone(),
                budget.clone(),
                HookRequest {
                    format: FORMAT,
                    event,
                    invocation_id,
                    actor,
                    turn_id,
                    call_id,
                    payload: value,
                },
            )
            .await
            .with_context(|| format!("{} hook {} failed", event.label(), index + 1))
            .and_then(|response| match response {
                HookResponse::Observe {} => Ok(HookAnnotation {
                    event,
                    hook_index: index,
                    text: String::new(),
                }),
                HookResponse::Annotate { annotation } => Ok(HookAnnotation {
                    event,
                    hook_index: index,
                    text: {
                        let projected = project_annotation(annotation)?;
                        budget.reserve_annotation(projected.len())?;
                        projected
                    },
                }),
                HookResponse::Allow {}
                | HookResponse::Deny { .. }
                | HookResponse::Rewrite { .. }
                | HookResponse::Stop { .. } => bail!("hook returned an invalid decision"),
            });
            match result {
                Ok(annotation) => {
                    let outcome = if annotation.text.is_empty() {
                        HookOutcomeKind::Observed
                    } else {
                        HookOutcomeKind::Annotated
                    };
                    observations.push(observation(event, index, outcome));
                    if !annotation.text.is_empty() {
                        annotations.push(annotation);
                    }
                }
                Err(error) => {
                    observations.push(observation(event, index, HookOutcomeKind::Failed));
                    failures.push(format!("{error:#}"));
                }
            }
        }
        PostHookRun {
            annotations,
            failures,
            observations,
        }
    }

    pub async fn run_speaker(
        &self,
        budget: &Arc<HookBudget>,
        invocation_id: &str,
        actor: &str,
        turn_id: Option<&str>,
        value: &Value,
    ) -> SpeakerHookRun {
        if self.suppressed {
            return SpeakerHookRun {
                outcome: Ok(SpeakerHookOutcome::Continue),
                observations: self.suppressed_observations(HookEvent::SpeakerSelected),
            };
        }
        let mut observations = Vec::new();
        for (index, hook) in self
            .hooks
            .event(HookEvent::SpeakerSelected)
            .iter()
            .enumerate()
        {
            let response = match invoke(
                self.root.clone(),
                self.workers.clone(),
                hook.clone(),
                budget.clone(),
                HookRequest {
                    format: FORMAT,
                    event: HookEvent::SpeakerSelected,
                    invocation_id,
                    actor,
                    turn_id,
                    call_id: None,
                    payload: value,
                },
            )
            .await
            .with_context(|| format!("speaker_selected hook {} failed", index + 1))
            {
                Ok(response) => response,
                Err(error) => {
                    observations.push(observation(
                        HookEvent::SpeakerSelected,
                        index,
                        HookOutcomeKind::Failed,
                    ));
                    return SpeakerHookRun {
                        outcome: Err(error),
                        observations,
                    };
                }
            };
            match response {
                HookResponse::Observe {} => observations.push(observation(
                    HookEvent::SpeakerSelected,
                    index,
                    HookOutcomeKind::Observed,
                )),
                HookResponse::Stop { reason } => {
                    let outcome = project_reason(reason, "hook stopped speaker dispatch")
                        .map(SpeakerHookOutcome::Stop);
                    observations.push(observation(
                        HookEvent::SpeakerSelected,
                        index,
                        if outcome.is_ok() {
                            HookOutcomeKind::Stopped
                        } else {
                            HookOutcomeKind::Failed
                        },
                    ));
                    return SpeakerHookRun {
                        outcome,
                        observations,
                    };
                }
                HookResponse::Allow {}
                | HookResponse::Deny { .. }
                | HookResponse::Rewrite { .. }
                | HookResponse::Annotate { .. } => {
                    observations.push(observation(
                        HookEvent::SpeakerSelected,
                        index,
                        HookOutcomeKind::Failed,
                    ));
                    return SpeakerHookRun {
                        outcome: Err(anyhow::anyhow!("hook returned an invalid decision")),
                        observations,
                    };
                }
            }
        }
        SpeakerHookRun {
            outcome: Ok(SpeakerHookOutcome::Continue),
            observations,
        }
    }
}

fn observation(event: HookEvent, hook_index: usize, outcome: HookOutcomeKind) -> HookObservation {
    HookObservation {
        event,
        hook_index,
        outcome,
    }
}

fn project_reason(reason: Option<String>, fallback: &str) -> Result<String> {
    let reason = reason.unwrap_or_else(|| fallback.to_owned());
    ensure!(reason.len() <= 4 * 1024, "hook reason exceeds its bound");
    let projected = project_hook_text(&reason).context("hook reason projection failed")?;
    ensure!(
        projected.len() <= 8 * 1024,
        "hook reason exceeds its projected bound"
    );
    Ok(projected)
}

fn project_annotation(annotation: String) -> Result<String> {
    ensure!(
        annotation.len() <= MAX_ANNOTATION_BYTES,
        "hook annotation exceeds its bound"
    );
    let projected = project_hook_text(&annotation).context("hook annotation projection failed")?;
    ensure!(
        projected.len() <= MAX_ANNOTATION_BYTES,
        "hook annotation exceeds its projected bound"
    );
    Ok(projected)
}

fn project_hook_text(value: &str) -> Result<String> {
    let redacted = redaction::text(value)?;
    let mut projected = String::new();
    for character in redacted.chars() {
        if character.is_control() {
            projected.extend(character.escape_default());
        } else {
            projected.push(character);
        }
    }
    Ok(projected)
}

async fn invoke(
    root: Arc<Directory>,
    workers: Arc<HookWorkers>,
    hook: HookCommand,
    budget: Arc<HookBudget>,
    request: HookRequest<'_>,
) -> Result<HookResponse> {
    let request = serde_json::to_vec(&request).context("serialize lifecycle hook request")?;
    ensure!(
        request.len() <= MAX_REQUEST_BYTES,
        "hook request exceeds its bound"
    );
    let (lease, aggregate_deadline) = budget.claim()?;
    let (reply, result) = oneshot::channel();
    // The worker owns cleanup so a dropped caller cannot abandon a live tree;
    // its slot lets the host await that cleanup before completion/shutdown.
    let slot = workers.enter();
    std::thread::Builder::new()
        .name("kuru-lifecycle-hook".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("lifecycle hook runtime failed");
            let mut owner = None;
            let outcome = match &runtime {
                Ok(runtime) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.block_on(run_owned(
                        root,
                        hook,
                        request,
                        &reply,
                        aggregate_deadline,
                        &mut owner,
                    ))
                })),
                Err(_) => Ok(Err(anyhow::anyhow!("lifecycle hook runtime failed"))),
            };
            // Only checked tree absence clears the retained owner. Generic
            // setup/wait errors and panics cannot authorize releasing it.
            let retaining = owner.is_some();
            if retaining {
                slot.cleanup_unconfirmed();
            }
            let outcome =
                outcome.unwrap_or_else(|_| Err(anyhow::anyhow!("lifecycle hook worker panicked")));
            if !retaining {
                // A confirmed outcome must expose settled budget and tracking
                // state, even when sending synchronously wakes its caller.
                drop(runtime);
                drop(lease);
                drop(slot);
                let _ = reply.send(outcome);
                return;
            }
            // Publish the existing bounded refusal before retained cleanup;
            // the same native worker and immutable HookLease remain owned.
            let _ = reply.send(outcome);
            if retaining && let (Ok(runtime), Some(mut owner)) = (runtime, owner) {
                runtime.block_on(async {
                    while cleanup_owned(&mut owner, Instant::now() + CLEANUP)
                        .await
                        .is_err()
                    {
                        tokio::time::sleep(POLL).await;
                    }
                });
            }
            drop(lease);
            drop(slot);
        })
        .context("lifecycle hook worker failed")?;
    result.await.context("lifecycle hook worker stopped")?
}

type Reader = tokio::task::JoinHandle<Result<Vec<u8>>>;

/// Stop the owned tree and its pipe readers under one cleanup deadline.
async fn stop(owner: &mut HookOwner, stdout: Reader, stderr: Reader) -> Result<()> {
    let limit = Instant::now() + CLEANUP;
    let cleaned = cleanup(owner, limit).await;
    finish_reader(stdout, limit).await;
    finish_reader(stderr, limit).await;
    cleaned
}

async fn run_owned(
    root: Arc<Directory>,
    hook: HookCommand,
    request: Vec<u8>,
    reply: &oneshot::Sender<Result<HookResponse>>,
    aggregate_deadline: Instant,
    retained: &mut Option<HookOwner>,
) -> Result<HookResponse> {
    ensure!(
        Instant::now() < aggregate_deadline,
        "hook execution time budget exhausted"
    );
    let (mut input, output, error) = spawn(&root, &hook, retained).await?;
    let owner = retained.as_mut().context("hook owner was not retained")?;
    let mut stdout = tokio::spawn(read_bounded(output, hook.max_output_bytes));
    let mut stderr = tokio::spawn(read_bounded(error, MAX_STDERR_BYTES));
    let deadline =
        (Instant::now() + Duration::from_millis(hook.timeout_ms)).min(aggregate_deadline);
    let write = async {
        input.write_all(&request).await?;
        input.write_all(b"\n").await?;
        input.shutdown().await
    };
    tokio::pin!(write);
    loop {
        if reply.is_closed() {
            stop(owner, stdout, stderr).await?;
            retained.take();
            bail!("lifecycle hook caller cancelled");
        }
        if Instant::now() >= deadline {
            stop(owner, stdout, stderr).await?;
            retained.take();
            bail!("lifecycle hook timed out");
        }
        tokio::select! {
            result = &mut write => {
                match result {
                    Ok(()) => break,
                    Err(_) => {
                        stop(owner, stdout, stderr).await?;
                        retained.take();
                        bail!("write lifecycle hook request failed");
                    }
                }
            }
            _ = tokio::time::sleep(POLL) => {}
        }
    }
    drop(input);
    let mut cleanup_confirmed = false;
    let exit = wait_for_exit(owner, deadline, reply, &mut cleanup_confirmed).await;
    if cleanup_confirmed {
        retained.take();
    }
    let (status, tail) = exit?;
    // The owned tree is gone, but a descendant that left it can still hold a
    // pipe. Drain only within the remaining deadline and the post-exit cleanup
    // bound already shared with reaping, stop at caller loss, and never parse
    // a partial response.
    let drain = deadline.min(tail);
    let drained = tokio::time::timeout_at(drain.into(), async {
        let pipes = async {
            let output = (&mut stdout)
                .await
                .context("lifecycle hook stdout task stopped")??;
            (&mut stderr)
                .await
                .context("lifecycle hook stderr task stopped")??;
            Ok::<_, anyhow::Error>(output)
        };
        tokio::pin!(pipes);
        loop {
            tokio::select! {
                output = &mut pipes => break Some(output),
                _ = tokio::time::sleep(POLL) => {
                    if reply.is_closed() {
                        break None;
                    }
                }
            }
        }
    })
    .await;
    let output = match drained {
        Ok(Some(output)) => output?,
        Ok(None) => {
            stdout.abort();
            stderr.abort();
            bail!("lifecycle hook caller cancelled");
        }
        Err(_) => {
            stdout.abort();
            stderr.abort();
            bail!("lifecycle hook output stayed open after its command exited");
        }
    };
    ensure!(status, "lifecycle hook command failed");
    parse_response(&output)
}

fn parse_response(bytes: &[u8]) -> Result<HookResponse> {
    ensure!(!bytes.is_empty(), "lifecycle hook returned no response");
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let response = HookResponse::deserialize(&mut deserializer)
        .context("lifecycle hook returned malformed output")?;
    deserializer
        .end()
        .context("lifecycle hook returned trailing output")?;
    Ok(response)
}

async fn read_bounded(mut pipe: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut total = 0_usize;
    loop {
        let count = pipe.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count);
        if retained.len() <= limit {
            let available = limit.saturating_add(1).saturating_sub(retained.len());
            retained.extend_from_slice(&buffer[..count.min(available)]);
        }
    }
    ensure!(total <= limit, "lifecycle hook output exceeded its bound");
    Ok(retained)
}

async fn finish_reader(mut task: Reader, limit: Instant) {
    if tokio::time::timeout_at(limit.into(), &mut task)
        .await
        .is_err()
    {
        task.abort();
    }
}

#[cfg(unix)]
async fn spawn(
    root: &Arc<Directory>,
    hook: &HookCommand,
    retained: &mut Option<HookOwner>,
) -> Result<(HookInput, HookOutput, HookError)> {
    use kuru_platform::unix::{StdioPlan, StdioSlot};
    use std::process::Command;

    let mut command = Command::new(&hook.command);
    command
        .args(&hook.args)
        .env_clear()
        .envs(crate::tools::unix_shell_environment(std::env::vars_os()))
        .env(HOOK_ORIGIN_ENV, "1")
        .current_dir(root.path());
    root.revalidate()
        .context("workspace changed before lifecycle hook launch")?;
    let piped = StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Pipe);
    *retained =
        Some(OwnedProcessGroup::spawn(command, piped).context("cannot start lifecycle hook")?);
    let owner = retained.as_mut().context("hook owner was not retained")?;
    let pipes = (|| -> Result<_> {
        let input = HookInput::from_std(owner.take_stdin()?)?;
        let output = HookOutput::from_std(owner.take_stdout()?)?;
        let error = HookError::from_std(owner.take_stderr()?)?;
        Ok((input, output, error))
    })();
    match pipes {
        Ok(pipes) => Ok(pipes),
        Err(error) => {
            cleanup(owner, Instant::now() + CLEANUP)
                .await
                .context("clean up lifecycle hook after pipe setup failure")?;
            retained.take();
            Err(error).context("prepare lifecycle hook pipes")
        }
    }
}

#[cfg(windows)]
async fn spawn(
    root: &Arc<Directory>,
    hook: &HookCommand,
    retained: &mut Option<HookOwner>,
) -> Result<(HookInput, HookOutput, HookError)> {
    let system = kuru_platform::windows::process::system_directory()?;
    let mut environment = crate::tools::windows_shell_environment(std::env::vars_os(), &system)?;
    environment.push((HOOK_ORIGIN_ENV.into(), "1".into()));
    let mut spec =
        crate::process::configured_finite(&hook.command, &hook.args, environment, root.path())?;
    // A hook is a generic configured command: keep a deliberate inherited
    // module path, except when the command explicitly selects stock Windows
    // PowerShell, which must reconstruct its own standard module paths.
    if let Some(module_path) = hook_module_path(std::env::vars_os(), &spec.executable, &system) {
        spec.environment.push(module_path);
    }
    spec.stdin = NativeStdio::Pipe;
    spec.stdout = NativeStdio::Pipe;
    spec.stderr = NativeStdio::Pipe;
    root.revalidate()
        .context("workspace changed before lifecycle hook launch")?;
    *retained = Some(spec.spawn().await.context("cannot start lifecycle hook")?);
    let owner = retained.as_mut().context("hook owner was not retained")?;
    let pipes = (|| -> Result<_> {
        let input = owner.take_stdin().context("missing lifecycle hook stdin")?;
        let output = owner
            .take_stdout()
            .context("missing lifecycle hook stdout")?;
        let error = owner
            .take_stderr()
            .context("missing lifecycle hook stderr")?;
        Ok((input, output, error))
    })();
    match pipes {
        Ok(pipes) => Ok(pipes),
        Err(error) => {
            cleanup(owner, Instant::now() + CLEANUP)
                .await
                .context("clean up lifecycle hook after pipe setup failure")?;
            retained.take();
            Err(error).context("prepare lifecycle hook pipes")
        }
    }
}

/// The inherited `PSModulePath` a Windows hook keeps, unless its resolved
/// executable is the system's stock Windows PowerShell.
#[cfg(any(windows, test))]
fn hook_module_path(
    environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    executable: &std::path::Path,
    system_directory: &std::path::Path,
) -> Option<(std::ffi::OsString, std::ffi::OsString)> {
    let normalized = |path: &std::path::Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase()
    };
    let stock = system_directory
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if normalized(executable) == normalized(&stock) {
        return None;
    }
    environment
        .into_iter()
        .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PSModulePath"))
        .map(|(_, value)| ("PSModulePath".into(), value))
}

/// Signal-before-reap cleanup of the owned tree. Any failure leaves the
/// tree's absence unconfirmed and is tracked as such by the host.
async fn cleanup(owner: &mut HookOwner, deadline: Instant) -> Result<()> {
    cleanup_owned(owner, deadline)
        .await
        .map_err(|error| error.context(CleanupUnconfirmed))
}

/// Wait for the root to exit, then reap it. Returns its success and the single
/// post-exit cleanup deadline that the caller's output drain must share.
#[cfg(unix)]
async fn wait_for_exit(
    owner: &mut HookOwner,
    deadline: Instant,
    reply: &oneshot::Sender<Result<HookResponse>>,
    cleanup_confirmed: &mut bool,
) -> Result<(bool, Instant)> {
    loop {
        if reply.is_closed() {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            *cleanup_confirmed = true;
            bail!("lifecycle hook caller cancelled");
        }
        if Instant::now() >= deadline {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            *cleanup_confirmed = true;
            bail!("lifecycle hook timed out");
        }
        match owner.root_state() {
            RootState::Exited | RootState::Reaped(_) => break,
            RootState::Running | RootState::Interrupted => {}
            RootState::Disarmed(_) => {
                return Err(anyhow::anyhow!("lifecycle hook ownership was lost")
                    .context(CleanupUnconfirmed));
            }
        }
        tokio::time::sleep(POLL).await;
    }
    let tail = Instant::now() + CLEANUP;
    let status = reap_after_exit(owner, tail)
        .await
        .map_err(|error| error.context(CleanupUnconfirmed))?;
    *cleanup_confirmed = true;
    Ok((status, tail))
}

/// The root exited: signal the remaining group before reaping the root, then
/// confirm group absence, all under one cleanup deadline.
#[cfg(unix)]
async fn reap_after_exit(owner: &mut HookOwner, limit: Instant) -> Result<bool> {
    loop {
        match owner.terminate_before_reap() {
            Termination::Signalled(_) | Termination::InvalidPhase => break,
            Termination::Interrupted => {}
            Termination::Disarmed(_) => bail!("lifecycle hook ownership was lost"),
        }
        ensure!(Instant::now() < limit, "lifecycle hook cleanup timed out");
        tokio::time::sleep(POLL).await;
    }
    let status = loop {
        let step = owner.pre_reap_step(limit);
        if let Some(status) = hook_reap_step(owner, limit, step)? {
            break status.success();
        }
        owner.wait_pre_reap(POLL, limit).await;
    };
    ensure_group_absent(owner, limit).await?;
    Ok(status)
}

/// Wait for the root to exit. Returns its success and the single post-exit
/// cleanup deadline that the caller's output drain must share.
#[cfg(windows)]
async fn wait_for_exit(
    owner: &mut HookOwner,
    deadline: Instant,
    reply: &oneshot::Sender<Result<HookResponse>>,
    cleanup_confirmed: &mut bool,
) -> Result<(bool, Instant)> {
    loop {
        if reply.is_closed() {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            *cleanup_confirmed = true;
            bail!("lifecycle hook caller cancelled");
        }
        if let Some(status) = owner.try_wait()? {
            *cleanup_confirmed = true;
            return Ok((status.success(), Instant::now() + CLEANUP));
        }
        if Instant::now() >= deadline {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            *cleanup_confirmed = true;
            bail!("lifecycle hook timed out");
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(unix)]
async fn cleanup_owned(owner: &mut HookOwner, deadline: Instant) -> Result<()> {
    loop {
        match owner.terminate_before_reap() {
            Termination::Signalled(_) | Termination::InvalidPhase => break,
            Termination::Interrupted => {}
            Termination::Disarmed(_) => bail!("lifecycle hook ownership was lost"),
        }
        ensure!(
            Instant::now() < deadline,
            "lifecycle hook cleanup timed out"
        );
        tokio::time::sleep(POLL).await;
    }
    loop {
        let step = owner.pre_reap_step(deadline);
        if hook_reap_step(owner, deadline, step)?.is_some() {
            break;
        }
        owner.wait_pre_reap(POLL, deadline).await;
    }
    ensure_group_absent(owner, deadline).await
}

#[cfg(unix)]
fn hook_reap_step(
    owner: &mut HookOwner,
    deadline: Instant,
    step: PreReap,
) -> Result<Option<std::process::ExitStatus>> {
    match step {
        PreReap::Ready | PreReap::Reaped | PreReap::Expired => match owner.reap_if_exited() {
            Reap::Reaped(status) => return Ok(Some(status)),
            Reap::NotExited | Reap::Interrupted => {}
            Reap::Disarmed(_) | Reap::InvalidPhase => bail!("lifecycle hook ownership was lost"),
        },
        PreReap::Pending | PreReap::ExpiredPending | PreReap::Unobserved(_) => {}
        PreReap::Disarmed(_) | PreReap::InvalidPhase => bail!("lifecycle hook ownership was lost"),
    }
    if Instant::now() >= deadline {
        // Membership starts only after root exit. Reap that exact root before
        // this hook owner is released, without waiting for the read-only
        // helper or claiming its cleanup finished. A still-running root is
        // observed nonblockingly, as in the existing expiry fallback.
        let _ = owner.reap_if_exited();
        bail!("lifecycle hook cleanup timed out");
    }
    Ok(None)
}

#[cfg(unix)]
async fn ensure_group_absent(owner: &HookOwner, deadline: Instant) -> Result<()> {
    // Signal zero is the poll; one bounded listing resolves EPERM at most once.
    let mut listing = owner.permission_listing(deadline);
    loop {
        match listing.resolve(owner.presence_after_reap()).await {
            GroupPresence::Absent | GroupPresence::Recycled => return Ok(()),
            GroupPresence::Present | GroupPresence::PermissionDenied => {}
            GroupPresence::ObservationError(_) | GroupPresence::InvalidPhase => {
                bail!("lifecycle hook cleanup could not be confirmed")
            }
        }
        ensure!(
            Instant::now() < deadline,
            "lifecycle hook cleanup timed out"
        );
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(windows)]
async fn cleanup_owned(owner: &mut HookOwner, deadline: Instant) -> Result<()> {
    owner.terminate().context("terminate lifecycle hook")?;
    owner
        .wait(deadline.saturating_duration_since(Instant::now()))
        .await
        .context("lifecycle hook cleanup timed out")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::HookCommand;
    use kuru_platform::fs::{NameRetention, Privacy};

    #[cfg(unix)]
    #[tokio::test]
    async fn pending_membership_expiry_reaps_root_and_reports_unconfirmed() {
        for step in [PreReap::Pending, PreReap::ExpiredPending] {
            let root = tempfile::tempdir().unwrap();
            let marker = root.path().join("root-id");
            let mut command = std::process::Command::new("/bin/sh");
            command
                .args(["-c", "printf '%s' \"$$\" > \"$1\"", "kuru-hook-expiry"])
                .arg(&marker);
            let mut owner = OwnedProcessGroup::spawn(
                command,
                kuru_platform::unix::StdioPlan::new(
                    kuru_platform::unix::StdioSlot::Null,
                    kuru_platform::unix::StdioSlot::Null,
                    kuru_platform::unix::StdioSlot::Null,
                ),
            )
            .unwrap();
            let limit = Instant::now() + CLEANUP;
            while !matches!(owner.root_state(), RootState::Exited) && Instant::now() < limit {
                tokio::time::sleep(POLL).await;
            }
            let exited = matches!(owner.root_state(), RootState::Exited);
            if !exited {
                let _ = cleanup_owned(&mut owner, Instant::now() + CLEANUP).await;
            }
            assert!(exited, "hook fixture did not exit");
            owner.terminate_before_reap();
            let result = hook_reap_step(&mut owner, Instant::now(), step);
            assert_eq!(
                result.unwrap_err().to_string(),
                "lifecycle hook cleanup timed out"
            );
            assert!(matches!(owner.root_state(), RootState::Reaped(_)));
            let id: u32 = std::fs::read_to_string(marker).unwrap().parse().unwrap();
            assert!(
                !kuru_platform::unix::snapshot::processes()
                    .unwrap()
                    .iter()
                    .any(|row| row.pid == id && row.ppid == std::process::id()),
                "expired hook root remained unreaped"
            );
        }
    }

    fn host(root: &std::path::Path, event: HookEvent, command: HookCommand) -> HookHost {
        let mut hooks = LifecycleHooks::default();
        match event {
            HookEvent::PreTurn => hooks.pre_turn.push(command),
            HookEvent::PostTurn => hooks.post_turn.push(command),
            HookEvent::PreTool => hooks.pre_tool.push(command),
            HookEvent::PostTool => hooks.post_tool.push(command),
            HookEvent::SpeakerSelected => hooks.speaker_selected.push(command),
        }
        HookHost::new(
            Arc::new(Directory::open(root, Privacy::Inherited, NameRetention::Pinned).unwrap()),
            hooks,
        )
    }

    #[cfg(unix)]
    fn command(script: &str) -> HookCommand {
        HookCommand {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            timeout_ms: 2_000,
            max_output_bytes: 1024,
        }
    }

    /// Warm the exact owned hook launch path once per test process, outside
    /// every timed hook budget; see `crate::shell_warmup`.
    #[cfg(windows)]
    async fn warm_hook_launch() {
        crate::shell_warmup::warm_up_stock_powershell_hook_launch()
            .await
            .unwrap();
    }

    #[cfg(windows)]
    fn windows_command(script: &str) -> HookCommand {
        let powershell = kuru_platform::windows::process::system_directory()
            .unwrap()
            .join("WindowsPowerShell/v1.0/powershell.exe");
        HookCommand {
            command: powershell.to_string_lossy().into_owned(),
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script.into(),
            ],
            max_output_bytes: 1024,
            // The product default; cold engine start is absorbed by
            // `warm_hook_launch` outside this budget.
            ..HookCommand::default()
        }
    }

    fn response_command(response: Value) -> HookCommand {
        let text = response.to_string();
        #[cfg(unix)]
        return command(&format!(
            "cat >/dev/null; printf '%s' '{}'",
            text.replace('\'', "'\\''")
        ));
        #[cfg(windows)]
        return windows_command(&format!(
            "$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{}')",
            text.replace('\'', "''")
        ));
    }

    #[tokio::test]
    async fn speaker_hooks_observe_stop_and_reject_invalid_decisions_before_later_launch() {
        #[cfg(windows)]
        warm_hook_launch().await;
        for response in [
            serde_json::json!({"decision":"observe"}),
            serde_json::json!({"decision":"stop"}),
            serde_json::json!({"decision":"stop","reason":"api_key=fixture-secret safe stop"}),
            serde_json::json!({"decision":"allow"}),
            serde_json::json!({"decision":"deny"}),
            serde_json::json!({"decision":"rewrite","value":{"input":"wrong event"}}),
            serde_json::json!({"decision":"annotate","annotation":"wrong event"}),
        ] {
            let root = tempfile::tempdir().unwrap();
            #[cfg(unix)]
            let later = command(
                "cat >/dev/null; : > speaker-later; printf '%s' '{\"decision\":\"observe\"}'",
            );
            #[cfg(windows)]
            let later = windows_command(
                r#"$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('speaker-later', ''); [Console]::Out.Write('{"decision":"observe"}')"#,
            );
            let host = host_with_hooks(
                root.path(),
                LifecycleHooks {
                    speaker_selected: vec![response_command(response.clone()), later],
                    ..LifecycleHooks::default()
                },
            );
            let budget = host.budget();
            let run = host
                .run_speaker(
                    &budget,
                    "invocation",
                    "actor",
                    Some("turn"),
                    &serde_json::json!({"speaker":"actor"}),
                )
                .await;
            host.quiesce().await.unwrap();
            match response["decision"].as_str().unwrap() {
                "observe" => {
                    assert_eq!(run.outcome.unwrap(), SpeakerHookOutcome::Continue);
                    assert_eq!(
                        run.observations,
                        vec![
                            observation(HookEvent::SpeakerSelected, 0, HookOutcomeKind::Observed),
                            observation(HookEvent::SpeakerSelected, 1, HookOutcomeKind::Observed)
                        ]
                    );
                    assert!(root.path().join("speaker-later").is_file());
                    assert_eq!(budget.snapshot().2, 2);
                }
                "stop" => {
                    let SpeakerHookOutcome::Stop(reason) = run.outcome.unwrap() else {
                        panic!("stop hook continued")
                    };
                    assert!(!reason.contains("fixture-secret"));
                    if response.get("reason").is_none() {
                        assert_eq!(reason, "hook stopped speaker dispatch");
                    }
                    assert_eq!(
                        run.observations,
                        vec![observation(
                            HookEvent::SpeakerSelected,
                            0,
                            HookOutcomeKind::Stopped
                        )]
                    );
                    assert!(!root.path().join("speaker-later").exists());
                    assert_eq!(budget.snapshot().2, 1);
                }
                _ => {
                    assert_eq!(
                        run.outcome.unwrap_err().to_string(),
                        "hook returned an invalid decision"
                    );
                    assert_eq!(
                        run.observations,
                        vec![observation(
                            HookEvent::SpeakerSelected,
                            0,
                            HookOutcomeKind::Failed
                        )]
                    );
                    assert!(!root.path().join("speaker-later").exists());
                    assert_eq!(budget.snapshot().2, 1);
                }
            }
            assert_eq!(budget.snapshot().1, 0, "speaker hook retained a lease");
        }
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let failed = command("cat >/dev/null; exit 7");
        #[cfg(windows)]
        let failed = windows_command("$null = [Console]::In.ReadToEnd(); exit 7");
        let host = host(root.path(), HookEvent::SpeakerSelected, failed);
        let budget = host.budget();
        let run = host
            .run_speaker(&budget, "invocation", "actor", None, &serde_json::json!({}))
            .await;
        host.quiesce().await.unwrap();
        assert!(run.outcome.is_err());
        assert_eq!(
            run.observations,
            vec![observation(
                HookEvent::SpeakerSelected,
                0,
                HookOutcomeKind::Failed
            )]
        );
        assert_eq!((budget.snapshot().1, budget.snapshot().2), (0, 1));
    }

    #[cfg(windows)]
    fn aggregate_for(hooks: &[&HookCommand]) -> u64 {
        hooks.iter().map(|hook| hook.timeout_ms).sum()
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_owned_hooks_rewrite_annotate_and_stop_after_timeout() {
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        let rewrite = windows_command(
            r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"rewrite","value":{"input":"windows-rewrite"}}')"#,
        );
        let check = windows_command(
            r#"$request = [Console]::In.ReadToEnd(); if ($request -notlike '*windows-rewrite*') { exit 9 }; [Console]::Out.Write('{"decision":"allow"}')"#,
        );
        let annotate = windows_command(
            r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"annotate","annotation":"separate windows note"}')"#,
        );
        let hooks = LifecycleHooks {
            max_total_ms: aggregate_for(&[&rewrite, &check, &annotate]),
            pre_turn: vec![rewrite, check],
            post_turn: vec![annotate],
            ..LifecycleHooks::default()
        };
        let directory = Arc::new(
            Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let host = HookHost::new(Arc::clone(&directory), hooks);
        let budget = host.budget();
        let pre = host
            .run_pre(
                &budget,
                HookEvent::PreTurn,
                "windows-invocation",
                "actor",
                Some("turn"),
                None,
                serde_json::json!({"input":"original"}),
            )
            .await;
        assert_eq!(
            pre.observations
                .iter()
                .map(|observation| observation.outcome)
                .collect::<Vec<_>>(),
            [HookOutcomeKind::Rewritten, HookOutcomeKind::Allowed],
            "{:?}",
            pre.outcome
        );
        assert_eq!(
            pre.outcome.unwrap(),
            PreHookOutcome::Allowed(serde_json::json!({"input":"windows-rewrite"}))
        );
        let post = host
            .run_post(
                &budget,
                HookEvent::PostTurn,
                "windows-invocation",
                "actor",
                Some("turn"),
                None,
                &serde_json::json!({"answer":"settled"}),
            )
            .await;
        assert_eq!(post.annotations.len(), 1);
        assert_eq!(post.annotations[0].text, "separate windows note");

        let marker = root.path().join("later-hook-ran");
        let started = root.path().join("slow-hook-started");
        let lock = root.path().join("slow-hook-lock");
        let slow = windows_command(&held_lock_script(&lock, &started));
        let later = windows_command(&format!(
            "$null = [Console]::In.ReadToEnd(); [System.IO.File]::WriteAllText('{}', 'x'); [Console]::Out.Write('{{\"decision\":\"allow\"}}')",
            quoted(&marker)
        ));
        let slow_budget = Duration::from_millis(slow.timeout_ms);
        let timeout_hooks = LifecycleHooks {
            max_total_ms: aggregate_for(&[&slow, &later]),
            pre_turn: vec![slow, later],
            ..LifecycleHooks::default()
        };
        let timed = Arc::new(HookHost::new(directory, timeout_hooks));
        let run = {
            let timed = Arc::clone(&timed);
            async move {
                let budget = timed.budget();
                timed
                    .run_pre(
                        &budget,
                        HookEvent::PreTurn,
                        "windows-timeout",
                        "actor",
                        Some("turn"),
                        None,
                        serde_json::json!({"input":"original"}),
                    )
                    .await
            }
        };
        let observe = async {
            // The hook writes its marker before its own timeout or not at all.
            let observed_start = wait_for_file(&started, slow_budget).await;
            let held_lock = observed_start
                && std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&lock)
                    .is_err();
            (observed_start, held_lock)
        };
        let (refused, (observed_start, held_lock)) = tokio::join!(run, observe);
        assert!(
            observed_start,
            "owned hook did not start before its timeout"
        );
        assert!(
            held_lock,
            "owned hook did not hold its exclusive file handle"
        );
        assert!(refused.outcome.is_err());
        assert_eq!(refused.observations.len(), 1);
        assert_eq!(refused.observations[0].outcome, HookOutcomeKind::Failed);
        timed.quiesce().await.unwrap();
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .expect("timed-out hook retained its exclusive file handle");
        assert!(
            !marker.exists(),
            "a timed-out pre hook admitted its successor"
        );
    }

    #[cfg(windows)]
    fn quoted(path: &std::path::Path) -> String {
        path.to_string_lossy().replace('\'', "''")
    }

    #[cfg(windows)]
    fn held_lock_script(lock: &std::path::Path, started: &std::path::Path) -> String {
        format!(
            "$null = [Console]::In.ReadToEnd(); $held = [System.IO.File]::Open('{}', [System.IO.FileMode]::Create, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None); try {{ [System.IO.File]::WriteAllText('{}', 'started'); [Threading.Thread]::Sleep(30000) }} finally {{ $held.Dispose() }}",
            quoted(lock),
            quoted(started)
        )
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_cancellation_awaits_reaping_the_started_hook() {
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        let started = root.path().join("hook-started");
        let lock = root.path().join("hook-lock");
        let hook = windows_command(&held_lock_script(&lock, &started));
        let budget_wait = Duration::from_millis(hook.timeout_ms);
        let host = Arc::new(host(root.path(), HookEvent::PreTurn, hook));
        let task = tokio::spawn({
            let host = host.clone();
            async move {
                host.run_pre(
                    &host.budget(),
                    HookEvent::PreTurn,
                    "invocation",
                    "actor",
                    Some("turn"),
                    None,
                    serde_json::json!({"input":"hello"}),
                )
                .await
            }
        });
        assert!(
            wait_for_file(&started, budget_wait).await,
            "owned hook did not start before its timeout"
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        host.quiesce().await.unwrap();
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .expect("cancelled hook retained its exclusive file handle");
    }

    #[test]
    fn protocol_rejects_unframed_unknown_and_oversized_results() {
        assert!(matches!(
            parse_response(br#"{"decision":"allow"}"#).unwrap(),
            HookResponse::Allow {}
        ));
        for invalid in [
            b"".as_slice(),
            b"{",
            b"{\"decision\":\"unknown\"}",
            b"{\"decision\":\"allow\",\"extra\":true}",
            b"{\"decision\":\"observe\",\"extra\":true}",
            b"{\"decision\":\"deny\",\"reason\":\"no\",\"extra\":true}",
            b"{\"decision\":\"rewrite\",\"value\":{},\"extra\":true}",
            b"{\"decision\":\"annotate\",\"annotation\":\"safe\",\"extra\":true}",
            b"{\"decision\":\"allow\",\"decision\":\"allow\"}",
            b"{\"decision\":\"allow\"} trailing",
            b"\xff",
        ] {
            assert!(parse_response(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(project_annotation("x".repeat(MAX_ANNOTATION_BYTES + 1)).is_err());
        assert!(project_reason(Some("x".repeat(4 * 1024 + 1)), "unused").is_err());
    }

    #[tokio::test]
    async fn pressured_stdout_and_stderr_fail_closed_before_the_next_pre_hook() {
        #[cfg(windows)]
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("later-hook-ran");
        #[cfg(unix)]
        let pressured = command(
            "cat >/dev/null; head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; printf '%s' '{\"decision\":\"allow\"}'",
        );
        #[cfg(windows)]
        let pressured = windows_command(
            r#"$null = [Console]::In.ReadToEnd(); $bytes = [byte[]]::new(131072); [Console]::OpenStandardOutput().Write($bytes, 0, $bytes.Length); [Console]::OpenStandardError().Write($bytes, 0, $bytes.Length); [Console]::Out.Write('{"decision":"allow"}')"#,
        );
        #[cfg(unix)]
        let later =
            command("cat >/dev/null; : > later-hook-ran; printf '%s' '{\"decision\":\"allow\"}'");
        #[cfg(windows)]
        let later = windows_command(
            r#"$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('later-hook-ran', ''); [Console]::Out.Write('{"decision":"allow"}')"#,
        );
        let hooks = LifecycleHooks {
            pre_turn: vec![pressured, later],
            ..LifecycleHooks::default()
        };
        let host = host_with_hooks(root.path(), hooks);
        let run = host
            .run_pre(
                &host.budget(),
                HookEvent::PreTurn,
                "invocation",
                "actor",
                Some("turn"),
                None,
                serde_json::json!({"input":"original"}),
            )
            .await;
        host.quiesce().await.unwrap();
        assert!(run.outcome.is_err());
        assert_eq!(run.observations.len(), 1);
        assert_eq!(run.observations[0].outcome, HookOutcomeKind::Failed);
        assert!(!marker.exists(), "failed pre hook admitted a later command");
    }

    #[tokio::test]
    async fn invalid_intermediate_rewrite_never_reaches_a_later_hook() {
        #[cfg(windows)]
        warm_hook_launch().await;
        for (event, invalid, input) in [
            (
                HookEvent::PreTurn,
                serde_json::json!({"input":""}),
                serde_json::json!({"input":"original"}),
            ),
            (
                HookEvent::PreTool,
                serde_json::json!({"name":"","arguments":{}}),
                serde_json::json!({"name":"file_read","arguments":{"path":"readme.txt"}}),
            ),
            // A pre-tool hook may rewrite only arguments, never the tool.
            (
                HookEvent::PreTool,
                serde_json::json!({"name":"shell","arguments":{"command":"true"}}),
                serde_json::json!({"name":"file_read","arguments":{"path":"readme.txt"}}),
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let marker = root.path().join("later-hook-ran");
            #[cfg(unix)]
            let later = command(
                "cat >/dev/null; : > later-hook-ran; printf '%s' '{\"decision\":\"allow\"}'",
            );
            #[cfg(windows)]
            let later = windows_command(
                r#"$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('later-hook-ran', ''); [Console]::Out.Write('{"decision":"allow"}')"#,
            );
            let chain = vec![
                response_command(serde_json::json!({"decision":"rewrite","value":invalid})),
                later,
            ];
            let hooks = if event == HookEvent::PreTurn {
                LifecycleHooks {
                    pre_turn: chain,
                    ..LifecycleHooks::default()
                }
            } else {
                LifecycleHooks {
                    pre_tool: chain,
                    ..LifecycleHooks::default()
                }
            };
            let host = host_with_hooks(root.path(), hooks);
            let run = host
                .run_pre(
                    &host.budget(),
                    event,
                    "invocation",
                    "actor",
                    Some("turn"),
                    (event == HookEvent::PreTool).then_some("call"),
                    input,
                )
                .await;
            host.quiesce().await.unwrap();
            assert!(run.outcome.is_err(), "{event:?}");
            assert_eq!(run.observations.len(), 1, "{event:?}");
            assert_eq!(run.observations[0].outcome, HookOutcomeKind::Failed);
            assert!(!marker.exists(), "{event:?} admitted later hook");
        }
    }

    #[tokio::test]
    async fn hostile_hook_diagnostics_stay_out_of_failures_and_annotations_are_projected() {
        #[cfg(windows)]
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let hostile = command(
            "cat >/dev/null; printf 'api_key=fixture-secret /private/fixture TOOL_RESULT_BODY \\033[31m' >&2; printf '\\377'",
        );
        #[cfg(windows)]
        let hostile = windows_command(
            r#"$null = [Console]::In.ReadToEnd(); [Console]::Error.Write('api_key=fixture-secret /private/fixture TOOL_RESULT_BODY ' + [char]27 + '[31m'); [Console]::OpenStandardOutput().WriteByte(255)"#,
        );
        let hooks = LifecycleHooks {
            pre_turn: vec![hostile],
            post_turn: vec![response_command(
                serde_json::json!({"decision":"annotate","annotation":"api_key=fixture-secret \u{1b}[31m"}),
            )],
            ..LifecycleHooks::default()
        };
        let host = host_with_hooks(root.path(), hooks);
        let budget = host.budget();
        let pre = host
            .run_pre(
                &budget,
                HookEvent::PreTurn,
                "invocation",
                "actor",
                Some("turn"),
                None,
                serde_json::json!({"input":"private original input"}),
            )
            .await;
        let post = host
            .run_post(
                &budget,
                HookEvent::PostTurn,
                "invocation",
                "actor",
                Some("turn"),
                None,
                &serde_json::json!({"result":"TOOL_RESULT_BODY"}),
            )
            .await;
        host.quiesce().await.unwrap();
        let failure = format!("{:#}", pre.outcome.unwrap_err());
        assert_eq!(pre.observations[0].outcome, HookOutcomeKind::Failed);
        for forbidden in [
            "fixture-secret",
            "/private/fixture",
            "TOOL_RESULT_BODY",
            "private original input",
            "\u{1b}",
        ] {
            assert!(
                !failure.contains(forbidden),
                "hook failure leaked {forbidden:?}"
            );
        }
        assert!(post.failures.is_empty(), "{:?}", post.failures);
        assert_eq!(post.annotations.len(), 1);
        assert_eq!(post.observations[0].outcome, HookOutcomeKind::Annotated);
        assert!(!post.annotations[0].text.contains("fixture-secret"));
        assert!(!post.annotations[0].text.contains('\u{1b}'));
        assert!(post.annotations[0].text.contains("\\u{1b}"));
    }

    #[tokio::test]
    async fn protocol_orders_rewrites_rejects_wrong_variants_and_continues_post_failures() {
        #[cfg(windows)]
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let check_rewrite = command(
            "read value; case \"$value\" in *first*) printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"name\":\"file_read\",\"arguments\":{\"path\":\"second\"}}}';; *) exit 9;; esac",
        );
        #[cfg(windows)]
        let check_rewrite = windows_command(
            r#"$request = [Console]::In.ReadToEnd(); if (-not $request.Contains('first')) { exit 9 }; [Console]::Out.Write('{"decision":"rewrite","value":{"name":"file_read","arguments":{"path":"second"}}}')"#,
        );
        #[cfg(unix)]
        let malformed = command("cat >/dev/null; printf trailing");
        #[cfg(windows)]
        let malformed =
            windows_command("$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('trailing')");
        let hooks = LifecycleHooks {
            pre_tool: vec![
                response_command(
                    serde_json::json!({"decision":"rewrite","value":{"name":"file_read","arguments":{"path":"first"}}}),
                ),
                check_rewrite,
            ],
            post_tool: vec![
                malformed,
                response_command(
                    serde_json::json!({"decision":"annotate","annotation":"safe note"}),
                ),
            ],
            ..LifecycleHooks::default()
        };
        let host = HookHost::new(
            Arc::new(
                Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
            ),
            hooks,
        );
        let budget = host.budget();
        let outcome = host
            .run_pre(
                &budget,
                HookEvent::PreTool,
                "invocation",
                "actor",
                Some("turn"),
                Some("call"),
                serde_json::json!({"name":"file_read","arguments":{"path":"original"}}),
            )
            .await
            .outcome
            .unwrap();
        assert_eq!(
            outcome,
            PreHookOutcome::Allowed(
                serde_json::json!({"name":"file_read","arguments":{"path":"second"}})
            )
        );
        let post = host
            .run_post(
                &budget,
                HookEvent::PostTool,
                "invocation",
                "actor",
                Some("turn"),
                Some("call"),
                &serde_json::json!({"result":"unchanged"}),
            )
            .await;
        host.quiesce().await.unwrap();
        assert_eq!(post.failures.len(), 1);
        assert_eq!(post.annotations[0].text, "safe note");
        assert_eq!(post.observations[0].outcome, HookOutcomeKind::Failed);
        assert_eq!(post.observations[1].outcome, HookOutcomeKind::Annotated);
    }

    async fn wait_for_file(path: &std::path::Path, bound: Duration) -> bool {
        tokio::time::timeout(bound, async {
            while !path.exists() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .is_ok()
    }

    #[cfg(unix)]
    fn group_present(group: i32) -> bool {
        // Signal zero, and a listing when it is answered; it never terminates
        // anything. A group recycled by another user is not ours.
        !kuru_platform::unix::observe_group_after_reap(group.unsigned_abs()).none_of_ours()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_awaits_reaping_the_started_owned_hook_tree() {
        let root = tempfile::tempdir().unwrap();
        let ready = root.path().join("hook-ready");
        // The root publishes its process-group id only after its descendant
        // is running, so cancellation always lands on a started tree.
        let mut hook = command(
            "sleep 30 & cat >/dev/null; printf '%s' $$ > hook-ready.tmp; mv hook-ready.tmp hook-ready; wait",
        );
        hook.timeout_ms = HookCommand::default().timeout_ms;
        let budget_wait = Duration::from_millis(hook.timeout_ms);
        let host = Arc::new(host(root.path(), HookEvent::PreTurn, hook));
        let task = tokio::spawn({
            let host = host.clone();
            async move {
                host.run_pre(
                    &host.budget(),
                    HookEvent::PreTurn,
                    "invocation",
                    "actor",
                    Some("turn"),
                    None,
                    serde_json::json!({"input":"hello"}),
                )
                .await
            }
        });
        assert!(
            wait_for_file(&ready, budget_wait).await,
            "owned hook did not start before its timeout"
        );
        let group: i32 = std::fs::read_to_string(&ready)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(group_present(group), "started hook group was not observed");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        host.quiesce().await.unwrap();
        assert!(
            !group_present(group),
            "owned hook group survived awaited cancellation cleanup"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn escaped_descendant_holding_stdout_fails_the_hook_within_its_deadline() {
        let root = tempfile::tempdir().unwrap();
        // The descendant leaves the owned process group with setsid, keeps the
        // inherited stdout/stderr open, and exits only when the test releases it.
        let hook = command(
            "cat >/dev/null; /usr/bin/perl -MPOSIX -e 'POSIX::setsid() or die; open(my $f, \">\", \"escaped.tmp\") or die; print $f $$; close $f; rename(\"escaped.tmp\", \"escaped\"); for (1..6000) { last if -e \"release\"; select(undef, undef, undef, 0.01) }' & while [ ! -e escaped ]; do sleep 0.01; done; printf '%s' '{\"decision\":\"allow\"}'",
        );
        let host = host(root.path(), HookEvent::PreTurn, hook);
        let run = host
            .run_pre(
                &host.budget(),
                HookEvent::PreTurn,
                "invocation",
                "actor",
                Some("turn"),
                None,
                serde_json::json!({"input":"hello"}),
            )
            .await;
        std::fs::write(root.path().join("release"), b"").unwrap();
        assert!(
            root.path().join("escaped").exists(),
            "descendant never escaped"
        );
        assert!(
            run.outcome.is_err(),
            "held stdout produced {:?}",
            run.outcome
        );
        assert_eq!(run.observations.len(), 1);
        assert_eq!(run.observations[0].outcome, HookOutcomeKind::Failed);
        host.quiesce().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_after_root_exit_stops_the_held_output_drain() {
        let root = tempfile::tempdir().unwrap();
        // The descendant escapes the owned group, keeps stdout open, and
        // publishes `orphaned` only after the hook root has exited, so the
        // caller is cancelled in the post-exit tail rather than before exit.
        let mut hook = command(
            "cat >/dev/null; /usr/bin/perl -MPOSIX -e 'POSIX::setsid() or die; my $p = getppid(); open(my $f, \">\", \"escaped.tmp\") or die; print $f $$; close $f; rename(\"escaped.tmp\", \"escaped\"); for (1..6000) { last if getppid() != $p; select(undef, undef, undef, 0.01) } open($f, \">\", \"orphaned.tmp\") or die; close $f; rename(\"orphaned.tmp\", \"orphaned\"); for (1..6000) { last if -e \"release\"; select(undef, undef, undef, 0.01) }' & while [ ! -e escaped ]; do sleep 0.01; done",
        );
        // A hook deadline well beyond the post-exit bound, so only caller loss
        // (not the deadline) can end the held drain early.
        hook.timeout_ms = 30_000;
        let budget_wait = Duration::from_millis(hook.timeout_ms);
        let host = Arc::new(host(root.path(), HookEvent::PreTurn, hook));
        let task = tokio::spawn({
            let host = host.clone();
            async move {
                host.run_pre(
                    &host.budget(),
                    HookEvent::PreTurn,
                    "invocation",
                    "actor",
                    Some("turn"),
                    None,
                    serde_json::json!({"input":"hello"}),
                )
                .await
            }
        });
        let orphaned = wait_for_file(&root.path().join("orphaned"), budget_wait).await;
        assert!(orphaned, "hook root did not exit before its timeout");
        // Give the worker many polls to observe the root exit and enter the
        // post-exit output drain before the caller goes away.
        tokio::time::sleep(POLL * 20).await;
        let cancelled = Instant::now();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        // The drain stops at its next poll after caller loss while the escaped
        // descendant still holds the output open; it does not run out the
        // post-exit `CLEANUP` bound.
        host.quiesce().await.unwrap();
        let settled = cancelled.elapsed();
        assert_eq!(host.in_flight_hooks(), 0);
        assert!(
            settled < CLEANUP / 2,
            "held output drain outlived caller loss for {settled:?}"
        );
        let escaped: i32 = std::fs::read_to_string(root.path().join("escaped"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let held = nix::sys::signal::kill(nix::unistd::Pid::from_raw(escaped), None).is_ok();
        std::fs::write(root.path().join("release"), b"").unwrap();
        assert!(
            held,
            "descendant released the output before cancellation settled"
        );
    }

    fn manual_clock() -> (Clock, Arc<Mutex<Instant>>) {
        let now = Arc::new(Mutex::new(Instant::now()));
        let read = now.clone();
        (Arc::new(move || *read.lock().unwrap()), now)
    }

    fn advance(clock: &Mutex<Instant>, millis: u64) {
        *clock.lock().unwrap() += Duration::from_millis(millis);
    }

    impl HookBudget {
        fn snapshot(&self) -> (Duration, usize, usize) {
            let state = self.lock();
            (state.remaining, state.active, state.invocations)
        }
    }

    #[test]
    fn budget_counts_overlap_once_and_leaves_idle_time_uncharged() {
        let (clock, now) = manual_clock();
        let budget = HookBudget::with_clock(
            &LifecycleHooks {
                max_total_ms: 1_000,
                max_invocations: 3,
                ..LifecycleHooks::default()
            },
            clock,
        );
        let (first, _) = budget.claim().unwrap();
        advance(&now, 50);
        let (second, _) = budget.claim().unwrap();
        assert_eq!(budget.snapshot().1, 2);
        advance(&now, 50);
        drop(first);
        advance(&now, 50);
        drop(second);
        // Two overlapping commands spanning 150 ms spend 150 ms, not 200 ms.
        assert_eq!(budget.snapshot(), (Duration::from_millis(850), 0, 2));
        advance(&now, 10_000);
        let (third, deadline) = budget.claim().unwrap();
        assert_eq!(deadline, *now.lock().unwrap() + Duration::from_millis(850));
        drop(third);
        assert_eq!(budget.snapshot(), (Duration::from_millis(850), 0, 3));
        assert!(
            budget
                .claim()
                .err()
                .expect("a fourth claim exceeded the invocation cap")
                .to_string()
                .contains("invocation")
        );
        assert_eq!(budget.snapshot().2, 3);
    }

    #[test]
    fn budget_refuses_new_work_once_time_reaches_zero() {
        let (clock, now) = manual_clock();
        let budget = HookBudget::with_clock(
            &LifecycleHooks {
                max_total_ms: 100,
                max_invocations: 8,
                ..LifecycleHooks::default()
            },
            clock,
        );
        let (lease, _) = budget.claim().unwrap();
        advance(&now, 150);
        drop(lease);
        assert_eq!(budget.snapshot(), (Duration::ZERO, 0, 1));
        assert!(budget.claim().is_err());
        assert_eq!(
            budget.snapshot().2,
            1,
            "a refused claim spent an invocation"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shared_budget_holds_leases_through_owned_reap_and_caps_invocations() {
        let root = tempfile::tempdir().unwrap();
        let mut hooks = LifecycleHooks {
            max_invocations: 3,
            ..LifecycleHooks::default()
        };
        // Each named invocation marks its start, then waits for the test's
        // release file; no assertion depends on elapsed time.
        hooks.pre_turn.push(command(
            r#"request=$(cat); case "$request" in *'"invocation_id":"one"'*) id=one;; *'"invocation_id":"two"'*) id=two;; *) id=;; esac; if [ -n "$id" ]; then : > "started-$id"; while [ ! -e "release-$id" ]; do sleep 0.01; done; fi; printf '%s' '{"decision":"allow"}'"#,
        ));
        hooks.pre_turn[0].timeout_ms = HookCommand::default().timeout_ms;
        let budget_wait = Duration::from_millis(hooks.pre_turn[0].timeout_ms);
        let host = Arc::new(host_with_hooks(root.path(), hooks));
        let budget = host.budget();
        let start = |id: &'static str| {
            let host = host.clone();
            let budget = budget.clone();
            tokio::spawn(async move {
                host.run_pre(
                    &budget,
                    HookEvent::PreTurn,
                    id,
                    "actor",
                    None,
                    None,
                    serde_json::json!({"input":id}),
                )
                .await
            })
        };
        let first = start("one");
        let second = start("two");
        for id in ["one", "two"] {
            assert!(
                wait_for_file(&root.path().join(format!("started-{id}")), budget_wait).await,
                "hook {id} did not start before its timeout"
            );
        }
        assert_eq!(
            budget.snapshot().1,
            2,
            "overlapping hooks hold one lease each"
        );
        for id in ["one", "two"] {
            std::fs::write(root.path().join(format!("release-{id}")), b"").unwrap();
        }
        assert!(first.await.unwrap().outcome.is_ok());
        assert!(second.await.unwrap().outcome.is_ok());
        assert_eq!(budget.snapshot().1, 0, "a lease outlived its reaped hook");
        let third = start("three").await.unwrap();
        assert!(third.outcome.is_ok(), "{third:?}");
        let (remaining, active, invocations) = budget.snapshot();
        assert_eq!((active, invocations), (0, 3));
        assert!(!remaining.is_zero());
        let fourth = start("four").await.unwrap();
        assert!(fourth.outcome.is_err());
        assert_eq!(fourth.observations[0].outcome, HookOutcomeKind::Failed);
        assert_eq!(budget.snapshot().2, 3, "the fourth hook launched");
    }

    struct CompletionWake {
        budget: Arc<HookBudget>,
        workers: Arc<HookWorkers>,
        observed: Mutex<Option<(usize, usize)>>,
        changed: tokio::sync::Notify,
    }

    impl std::task::Wake for CompletionWake {
        fn wake(self: Arc<Self>) {
            *self.observed.lock().unwrap() =
                Some((self.budget.snapshot().1, self.workers.state.borrow().active));
            self.changed.notify_one();
        }
    }

    #[tokio::test]
    async fn confirmed_hook_releases_lease_and_worker_before_result_wake() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };

        #[cfg(windows)]
        warm_hook_launch().await;
        for (response, succeeds) in [(r#"{"decision":"allow"}"#, true), ("invalid", false)] {
            let root = tempfile::tempdir().unwrap();
            #[cfg(unix)]
            let hook = command(&format!(
                "cat >/dev/null; while [ ! -e release ]; do sleep 0.01; done; printf '%s' '{response}'"
            ));
            #[cfg(windows)]
            let hook = windows_command(&format!(
                "$null = [Console]::In.ReadLine(); while (-not [IO.File]::Exists('release')) {{ [Threading.Thread]::Sleep(10) }}; [Console]::Write('{response}')"
            ));
            let bound = Duration::from_millis(hook.timeout_ms) + QUIESCE;
            let host = host(root.path(), HookEvent::PreTurn, hook);
            let budget = host.budget();
            let wake = Arc::new(CompletionWake {
                budget: budget.clone(),
                workers: host.workers.clone(),
                observed: Mutex::new(None),
                changed: tokio::sync::Notify::new(),
            });
            let waker = Waker::from(wake.clone());
            let mut context = Context::from_waker(&waker);
            let mut run = Box::pin(host.run_pre(
                &budget,
                HookEvent::PreTurn,
                "invocation",
                "actor",
                None,
                None,
                serde_json::json!({"input":"hello"}),
            ));
            assert!(matches!(run.as_mut().poll(&mut context), Poll::Pending));
            std::fs::write(root.path().join("release"), b"").unwrap();
            tokio::time::timeout(bound, wake.changed.notified())
                .await
                .unwrap();
            let observed = *wake.observed.lock().unwrap();
            let outcome = run.await;
            host.quiesce().await.unwrap();
            assert_eq!(outcome.outcome.is_ok(), succeeds, "{outcome:?}");
            assert_eq!(
                observed,
                Some((0, 0)),
                "completion woke before ownership settled"
            );
        }
    }

    #[tokio::test]
    async fn suppressed_hosts_report_every_configured_hook_without_running_it() {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let ran = command("cat >/dev/null; : > hook-ran; printf '%s' '{\"decision\":\"deny\"}'");
        #[cfg(windows)]
        let ran = windows_command(
            r#"$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('hook-ran', ''); [Console]::Out.Write('{"decision":"deny"}')"#,
        );
        let hooks = LifecycleHooks {
            pre_tool: vec![ran.clone(), ran.clone()],
            post_turn: vec![ran.clone()],
            speaker_selected: vec![ran],
            ..LifecycleHooks::default()
        };
        let directory = Arc::new(
            Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        assert!(!HookHost::with_origin(directory.clone(), hooks.clone(), None).suppressed());
        let host = HookHost::with_origin(directory, hooks, Some(std::ffi::OsStr::new("1")));
        assert!(host.suppressed() && host.configured(HookEvent::PreTool));
        let budget = host.budget();
        let value = serde_json::json!({"name":"file_read","arguments":{"path":"a"}});
        let pre = host
            .run_pre(
                &budget,
                HookEvent::PreTool,
                "invocation",
                "actor",
                None,
                Some("call"),
                value.clone(),
            )
            .await;
        assert_eq!(pre.outcome.unwrap(), PreHookOutcome::Allowed(value));
        assert_eq!(
            pre.observations,
            [
                observation(HookEvent::PreTool, 0, HookOutcomeKind::Suppressed),
                observation(HookEvent::PreTool, 1, HookOutcomeKind::Suppressed)
            ]
        );
        let post = host
            .run_post(
                &budget,
                HookEvent::PostTurn,
                "invocation",
                "actor",
                None,
                None,
                &serde_json::json!({}),
            )
            .await;
        assert!(post.annotations.is_empty() && post.failures.is_empty());
        assert_eq!(post.observations[0].outcome, HookOutcomeKind::Suppressed);
        let speaker = host
            .run_speaker(&budget, "invocation", "actor", None, &serde_json::json!({}))
            .await;
        assert_eq!(speaker.outcome.unwrap(), SpeakerHookOutcome::Continue);
        assert_eq!(speaker.observations[0].outcome, HookOutcomeKind::Suppressed);
        assert!(!root.path().join("hook-ran").exists());
        assert_eq!(budget.snapshot().2, 0);
    }

    #[test]
    fn windows_hooks_keep_deliberate_module_paths_except_for_stock_powershell() {
        let system = std::path::Path::new("C:\\Windows\\System32");
        let environment = || {
            vec![
                ("Path".into(), "C:\\bin".into()),
                ("psmodulepath".into(), "C:\\modules".into()),
            ]
        };
        assert_eq!(
            hook_module_path(
                environment(),
                std::path::Path::new("c:/windows/system32/WindowsPowerShell/v1.0/POWERSHELL.EXE"),
                system
            ),
            None
        );
        assert_eq!(
            hook_module_path(
                environment(),
                std::path::Path::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
                system
            ),
            Some(("PSModulePath".into(), "C:\\modules".into()))
        );
        assert_eq!(
            hook_module_path(vec![], std::path::Path::new("C:\\tools\\hook.exe"), system),
            None
        );
    }

    #[tokio::test]
    async fn annotation_budget_fails_only_the_excess_post_hook() {
        #[cfg(windows)]
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        let mut hooks = LifecycleHooks {
            max_annotation_bytes: 5,
            ..LifecycleHooks::default()
        };
        hooks.post_tool = vec![
            response_command(serde_json::json!({"decision":"annotate","annotation":"four"})),
            response_command(serde_json::json!({"decision":"annotate","annotation":"more"})),
        ];
        let host = host_with_hooks(root.path(), hooks);
        let run = host
            .run_post(
                &host.budget(),
                HookEvent::PostTool,
                "invocation",
                "actor",
                None,
                Some("call"),
                &serde_json::json!({"result":"settled"}),
            )
            .await;
        host.quiesce().await.unwrap();
        assert_eq!(run.annotations.len(), 1);
        assert_eq!(run.annotations[0].text, "four");
        assert_eq!(run.observations[1].outcome, HookOutcomeKind::Failed);
        assert_eq!(run.failures.len(), 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn caller_loss_during_undrained_native_request_reaps_before_releasing_its_lease() {
        warm_hook_launch().await;
        let root = tempfile::tempdir().unwrap();
        let started = root.path().join("request-byte-acknowledged");
        let lock = root.path().join("request-owner-lock");
        let later_marker = root.path().join("later-hook-ran");
        std::fs::write(&lock, b"owned fixture witness").unwrap();
        let original = kuru_platform::fs::regular_file_info(&std::fs::File::open(&lock).unwrap())
            .unwrap()
            .identity;
        let mut hook = windows_command(&format!(
            "$held = [IO.File]::Open('{}', [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); try {{ $inputPipe = [Console]::OpenStandardInput(); if ($inputPipe.ReadByte() -ne 123) {{ exit 9 }}; [IO.File]::WriteAllText('{}', 'first request byte'); [Threading.Thread]::Sleep(30000) }} finally {{ $held.Dispose() }}",
            quoted(&lock),
            quoted(&started)
        ));
        // Separate the held peer's deadline from the unchanged quiesce bound:
        // ordinary timeout must never satisfy the caller-loss cleanup oracle.
        hook.timeout_ms = 30_000;
        let bound = Duration::from_millis(hook.timeout_ms);
        let later = windows_command(&format!(
            "$null = [Console]::In.ReadToEnd(); [IO.File]::WriteAllText('{}', 'unexpected dispatch'); [Console]::Out.Write('{{\"decision\":\"allow\"}}')",
            quoted(&later_marker)
        ));
        // windows/pipe.rs::server_with_instances supplies 65536-byte input
        // and output buffers. One byte acknowledges actual write admission;
        // the peer never drains or parses the remaining larger request.
        let payload = serde_json::json!({"input":"x".repeat(192 * 1024)});
        let serialized = serde_json::to_vec(&HookRequest {
            format: FORMAT,
            event: HookEvent::PreTurn,
            invocation_id: "blocked-native-request",
            actor: "actor",
            turn_id: Some("turn"),
            call_id: None,
            payload: &payload,
        })
        .unwrap();
        assert!(serialized.len() > 65536 && serialized.len() < MAX_REQUEST_BYTES);
        let host = Arc::new(host_with_hooks(
            root.path(),
            LifecycleHooks {
                pre_turn: vec![hook, later],
                ..LifecycleHooks::default()
            },
        ));
        let budget = host.budget();
        let launched = Instant::now();
        let task = tokio::spawn({
            let host = host.clone();
            let budget = budget.clone();
            async move {
                host.run_pre(
                    &budget,
                    HookEvent::PreTurn,
                    "blocked-native-request",
                    "actor",
                    Some("turn"),
                    None,
                    payload,
                )
                .await
            }
        });
        let acknowledged = wait_for_file(&started, bound).await;
        let active = budget.snapshot().1;
        let in_flight = host.in_flight_hooks();
        let pending = !task.is_finished();
        let exclusively_held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .is_err();
        task.abort();
        let cancelled = task.await;
        let settled = host.quiesce().await;
        let reopened = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock);
        settled.unwrap();
        let reopened = reopened.expect("cancelled native hook retained its exclusive lock");
        let observed = kuru_platform::fs::regular_file_info(&reopened)
            .unwrap()
            .identity;
        drop(reopened);
        assert!(
            launched.elapsed() < bound,
            "native request ended at its hook deadline rather than caller loss"
        );
        assert!(
            acknowledged && pending && exclusively_held,
            "request was not admitted into the held native peer"
        );
        assert_eq!((active, in_flight), (1, 1));
        assert!(cancelled.unwrap_err().is_cancelled());
        assert_eq!((budget.snapshot().1, host.in_flight_hooks()), (0, 0));
        assert_eq!(
            budget.snapshot().2,
            1,
            "caller loss admitted the later hook"
        );
        assert_eq!(observed, original);
        assert_eq!(std::fs::read(&lock).unwrap(), b"owned fixture witness");
        assert!(!later_marker.exists());
    }

    fn host_with_hooks(root: &std::path::Path, hooks: LifecycleHooks) -> HookHost {
        HookHost::new(
            Arc::new(Directory::open(root, Privacy::Inherited, NameRetention::Pinned).unwrap()),
            hooks,
        )
    }
}
