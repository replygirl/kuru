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
    sync::oneshot,
};

use crate::redaction;

#[cfg(unix)]
use kuru_platform::unix::{GroupPresence, OwnedProcessGroup, Reap, RootState, Termination};
#[cfg(windows)]
use kuru_platform::windows::process::{NativeChild, Stdio as NativeStdio};

const FORMAT: u16 = 1;
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ANNOTATION_BYTES: usize = 16 * 1024;
const CLEANUP: Duration = Duration::from_secs(5);
const HOOK_ORIGIN_ENV: &str = "KURU_INTERNAL_LIFECYCLE_HOOK_ORIGIN";

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreTurnValue {
    input: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreToolValue {
    name: String,
    arguments: Value,
}

fn validate_pre_rewrite(event: HookEvent, value: &Value) -> Result<()> {
    match event {
        HookEvent::PreTurn => {
            let payload: PreTurnValue = serde_json::from_value(value.clone())
                .context("pre-turn hook returned an invalid input")?;
            ensure!(
                !payload.input.trim().is_empty() && payload.input.len() <= 131_072,
                "pre-turn hook returned an invalid input"
            );
        }
        HookEvent::PreTool => {
            let payload: PreToolValue = serde_json::from_value(value.clone())
                .context("pre-tool hook returned an invalid operation")?;
            ensure!(
                !payload.name.trim().is_empty()
                    && payload.name.len() <= 256
                    && payload.arguments.is_object()
                    && serde_json::to_vec(&payload.arguments)?.len() <= crate::MAX_BYTES,
                "pre-tool hook returned an invalid operation"
            );
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
}

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
}

struct HookLease(Arc<HookBudget>);

impl HookBudget {
    pub fn new(hooks: &LifecycleHooks) -> Arc<Self> {
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
        })
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
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
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
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        HookBudget::tick(&mut state, Instant::now());
        state.active -= 1;
        if state.active == 0 {
            state.active_since = None;
        }
    }
}

impl HookHost {
    pub fn new(root: Arc<Directory>, hooks: LifecycleHooks) -> Self {
        // Only the owned hook launch adds this marker to its finite child
        // environment. A Kuru process started by that command still performs
        // its ordinary admission, trust, and tool checks, but cannot start a
        // second lifecycle-hook chain from that nested operation.
        let hooks =
            if std::env::var_os(HOOK_ORIGIN_ENV).as_deref() == Some(std::ffi::OsStr::new("1")) {
                LifecycleHooks::default()
            } else {
                hooks
            };
        Self { root, hooks }
    }

    pub fn configured(&self, event: HookEvent) -> bool {
        !self.hooks.event(event).is_empty()
    }

    pub fn budget(&self) -> Arc<HookBudget> {
        HookBudget::new(&self.hooks)
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
        let mut observations = Vec::new();
        for (index, hook) in self.hooks.event(event).iter().enumerate() {
            let response = match invoke(
                self.root.clone(),
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
                    let checked = validate_pre_rewrite(event, &rewritten).and_then(|()| {
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
        let mut annotations = Vec::new();
        let mut failures = Vec::new();
        let mut observations = Vec::with_capacity(self.hooks.event(event).len());
        for (index, hook) in self.hooks.event(event).iter().enumerate() {
            let result = invoke(
                self.root.clone(),
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
        let mut observations = Vec::new();
        for (index, hook) in self
            .hooks
            .event(HookEvent::SpeakerSelected)
            .iter()
            .enumerate()
        {
            let response = match invoke(
                self.root.clone(),
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
    std::thread::Builder::new()
        .name("kuru-lifecycle-hook".into())
        .spawn(move || {
            let outcome = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("lifecycle hook runtime failed")
                .and_then(|runtime| {
                    runtime.block_on(run_owned(root, hook, request, &reply, aggregate_deadline))
                });
            drop(lease);
            let _ = reply.send(outcome);
        })
        .context("lifecycle hook worker failed")?;
    result.await.context("lifecycle hook worker stopped")?
}

async fn run_owned(
    root: Arc<Directory>,
    hook: HookCommand,
    request: Vec<u8>,
    reply: &oneshot::Sender<Result<HookResponse>>,
    aggregate_deadline: Instant,
) -> Result<HookResponse> {
    ensure!(
        Instant::now() < aggregate_deadline,
        "hook execution time budget exhausted"
    );
    let (mut owner, mut input, output, error) = spawn(&root, &hook).await?;
    let stdout = tokio::spawn(read_bounded(output, hook.max_output_bytes));
    let stderr = tokio::spawn(read_bounded(error, MAX_STDERR_BYTES));
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
            cleanup(&mut owner, Instant::now() + CLEANUP).await?;
            finish_reader(stdout).await;
            finish_reader(stderr).await;
            bail!("lifecycle hook caller cancelled");
        }
        if Instant::now() >= deadline {
            cleanup(&mut owner, Instant::now() + CLEANUP).await?;
            finish_reader(stdout).await;
            finish_reader(stderr).await;
            bail!("lifecycle hook timed out");
        }
        tokio::select! {
            result = &mut write => {
                match result {
                    Ok(()) => break,
                    Err(_) => {
                        cleanup(&mut owner, Instant::now() + CLEANUP).await?;
                        finish_reader(stdout).await;
                        finish_reader(stderr).await;
                        bail!("write lifecycle hook request failed");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
    drop(input);
    let status = wait_for_exit(&mut owner, deadline, reply).await?;
    let output = stdout
        .await
        .context("lifecycle hook stdout task stopped")??;
    let _ = stderr
        .await
        .context("lifecycle hook stderr task stopped")??;
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

async fn finish_reader(task: tokio::task::JoinHandle<Result<Vec<u8>>>) {
    let _ = tokio::time::timeout(CLEANUP, task).await;
}

#[cfg(unix)]
async fn spawn(
    root: &Arc<Directory>,
    hook: &HookCommand,
) -> Result<(HookOwner, HookInput, HookOutput, HookError)> {
    use std::process::{Command, Stdio};

    let mut command = Command::new(&hook.command);
    command
        .args(&hook.args)
        .env_clear()
        .envs(crate::tools::unix_shell_environment(std::env::vars_os()))
        .env(HOOK_ORIGIN_ENV, "1")
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    root.revalidate()
        .context("workspace changed before lifecycle hook launch")?;
    let mut owner = OwnedProcessGroup::spawn(command).context("cannot start lifecycle hook")?;
    let pipes = (|| -> Result<_> {
        let input = HookInput::from_std(owner.take_stdin()?)?;
        let output = HookOutput::from_std(owner.take_stdout()?)?;
        let error = HookError::from_std(owner.take_stderr()?)?;
        Ok((input, output, error))
    })();
    match pipes {
        Ok((input, output, error)) => Ok((owner, input, output, error)),
        Err(error) => {
            cleanup(&mut owner, Instant::now() + CLEANUP)
                .await
                .context("clean up lifecycle hook after pipe setup failure")?;
            Err(error).context("prepare lifecycle hook pipes")
        }
    }
}

#[cfg(windows)]
async fn spawn(
    root: &Arc<Directory>,
    hook: &HookCommand,
) -> Result<(HookOwner, HookInput, HookOutput, HookError)> {
    let mut environment = crate::tools::windows_shell_environment(
        std::env::vars_os(),
        &kuru_platform::windows::process::system_directory()?,
    )?;
    environment.push((HOOK_ORIGIN_ENV.into(), "1".into()));
    let mut spec =
        crate::process::configured_finite(&hook.command, &hook.args, environment, root.path())?;
    spec.stdin = NativeStdio::Pipe;
    spec.stdout = NativeStdio::Pipe;
    spec.stderr = NativeStdio::Pipe;
    root.revalidate()
        .context("workspace changed before lifecycle hook launch")?;
    let mut owner = spec.spawn().await.context("cannot start lifecycle hook")?;
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
        Ok((input, output, error)) => Ok((owner, input, output, error)),
        Err(error) => {
            cleanup(&mut owner, Instant::now() + CLEANUP)
                .await
                .context("clean up lifecycle hook after pipe setup failure")?;
            Err(error).context("prepare lifecycle hook pipes")
        }
    }
}

#[cfg(unix)]
async fn wait_for_exit(
    owner: &mut HookOwner,
    deadline: Instant,
    reply: &oneshot::Sender<Result<HookResponse>>,
) -> Result<bool> {
    loop {
        if reply.is_closed() {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            bail!("lifecycle hook caller cancelled");
        }
        if Instant::now() >= deadline {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            bail!("lifecycle hook timed out");
        }
        match owner.root_state() {
            RootState::Exited | RootState::Reaped(_) => break,
            RootState::Running | RootState::Interrupted => {}
            RootState::Disarmed(_) => bail!("lifecycle hook ownership was lost"),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    loop {
        match owner.terminate_before_reap() {
            Termination::Signalled(_) | Termination::InvalidPhase => break,
            Termination::Interrupted => {}
            Termination::Disarmed(_) => bail!("lifecycle hook ownership was lost"),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let status = loop {
        match owner.reap_if_exited() {
            Reap::Reaped(status) => break status.success(),
            Reap::NotExited | Reap::Interrupted => {}
            Reap::Disarmed(_) | Reap::InvalidPhase => {
                bail!("lifecycle hook ownership was lost")
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    ensure_group_absent(owner, Instant::now() + CLEANUP).await?;
    Ok(status)
}

#[cfg(windows)]
async fn wait_for_exit(
    owner: &mut HookOwner,
    deadline: Instant,
    reply: &oneshot::Sender<Result<HookResponse>>,
) -> Result<bool> {
    loop {
        if reply.is_closed() {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            bail!("lifecycle hook caller cancelled");
        }
        if let Some(status) = owner.try_wait()? {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            cleanup(owner, Instant::now() + CLEANUP).await?;
            bail!("lifecycle hook timed out");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(unix)]
async fn cleanup(owner: &mut HookOwner, deadline: Instant) -> Result<()> {
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
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    loop {
        match owner.reap_if_exited() {
            Reap::Reaped(_) => break,
            Reap::NotExited | Reap::Interrupted => {}
            Reap::Disarmed(_) | Reap::InvalidPhase => {
                bail!("lifecycle hook ownership was lost")
            }
        }
        ensure!(
            Instant::now() < deadline,
            "lifecycle hook cleanup timed out"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ensure_group_absent(owner, deadline).await
}

#[cfg(unix)]
async fn ensure_group_absent(owner: &HookOwner, deadline: Instant) -> Result<()> {
    loop {
        match owner.presence_after_reap() {
            GroupPresence::Absent => return Ok(()),
            GroupPresence::Present | GroupPresence::PermissionDenied => {}
            GroupPresence::ObservationError(_) | GroupPresence::InvalidPhase => {
                bail!("lifecycle hook cleanup could not be confirmed")
            }
        }
        ensure!(
            Instant::now() < deadline,
            "lifecycle hook cleanup timed out"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
async fn cleanup(owner: &mut HookOwner, deadline: Instant) -> Result<()> {
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

    #[cfg(windows)]
    const WINDOWS_HOOK_STARTUP_MS: u64 = 120_000;

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
            // Stock PowerShell's first runspace can take longer than a short
            // hook assertion under instrumented Windows CI; see shell_warmup.
            timeout_ms: WINDOWS_HOOK_STARTUP_MS,
            max_output_bytes: 1024,
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_owned_hooks_rewrite_annotate_and_stop_after_timeout() {
        let root = tempfile::tempdir().unwrap();
        let hooks = LifecycleHooks {
            max_total_ms: 3 * WINDOWS_HOOK_STARTUP_MS,
            pre_turn: vec![
                windows_command(
                    r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"rewrite","value":{"input":"windows-rewrite"}}')"#,
                ),
                windows_command(
                    r#"$request = [Console]::In.ReadToEnd(); if ($request -notlike '*windows-rewrite*') { exit 9 }; [Console]::Out.Write('{"decision":"allow"}')"#,
                ),
            ],
            post_turn: vec![windows_command(
                r#"$null = [Console]::In.ReadToEnd(); [Console]::Out.Write('{"decision":"annotate","annotation":"separate windows note"}')"#,
            )],
            ..LifecycleHooks::default()
        };
        let directory = Arc::new(
            Directory::open(root.path(), Privacy::Inherited, NameRetention::Pinned).unwrap(),
        );
        let host = HookHost::new(Arc::clone(&directory), hooks);
        let budget = host.budget();
        let pre_started = Instant::now();
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
        if let Err(error) = &pre.outcome {
            panic!(
                "first Windows hook failed after {:?}; observations: {:?}; cause: {error:#}",
                pre_started.elapsed(),
                pre.observations
                    .iter()
                    .map(|observation| observation.outcome)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            pre.outcome.unwrap(),
            PreHookOutcome::Allowed(serde_json::json!({"input":"windows-rewrite"}))
        );
        assert_eq!(pre.observations[0].outcome, HookOutcomeKind::Rewritten);
        assert_eq!(pre.observations[1].outcome, HookOutcomeKind::Allowed);
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
        let quoted_marker = marker.to_string_lossy().replace('\'', "''");
        let quoted_started = started.to_string_lossy().replace('\'', "''");
        let quoted_lock = lock.to_string_lossy().replace('\'', "''");
        let mut slow = windows_command(&format!(
            "$null = [Console]::In.ReadToEnd(); $held = [System.IO.File]::Open('{quoted_lock}', [System.IO.FileMode]::Create, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None); try {{ [System.IO.File]::WriteAllText('{quoted_started}', 'started'); [Threading.Thread]::Sleep(30000) }} finally {{ $held.Dispose() }}"
        ));
        slow.timeout_ms = 10_000;
        let timeout_hooks = LifecycleHooks {
            pre_turn: vec![
                slow,
                windows_command(&format!(
                    "$null = [Console]::In.ReadToEnd(); [System.IO.File]::WriteAllText('{quoted_marker}', 'x'); [Console]::Out.Write('{{\"decision\":\"allow\"}}')"
                )),
            ],
            ..LifecycleHooks::default()
        };
        let timed = Arc::new(HookHost::new(directory, timeout_hooks));
        let pending = tokio::spawn({
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
        });
        let observed_start = tokio::time::timeout(Duration::from_secs(8), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            observed_start.is_ok(),
            "owned hook did not start before its timeout"
        );
        assert!(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock)
                .is_err(),
            "owned hook did not hold its exclusive file handle"
        );
        let refused = pending.await.unwrap();
        assert!(refused.outcome.is_err());
        assert_eq!(refused.observations.len(), 1);
        assert_eq!(refused.observations[0].outcome, HookOutcomeKind::Failed);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn pressured_stdout_and_stderr_fail_closed_before_the_next_pre_hook() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("later-hook-ran");
        let hooks = LifecycleHooks {
            pre_turn: vec![
                command(
                    "cat >/dev/null; head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; printf '%s' '{\"decision\":\"allow\"}'",
                ),
                command(
                    "cat >/dev/null; : > later-hook-ran; printf '%s' '{\"decision\":\"allow\"}'",
                ),
            ],
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
        assert!(run.outcome.is_err());
        assert_eq!(run.observations.len(), 1);
        assert_eq!(run.observations[0].outcome, HookOutcomeKind::Failed);
        assert!(!marker.exists(), "failed pre hook admitted a later command");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn invalid_intermediate_rewrite_never_reaches_a_later_hook() {
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
        ] {
            let root = tempfile::tempdir().unwrap();
            let marker = root.path().join("later-hook-ran");
            let chain = vec![
                command(&format!(
                    "cat >/dev/null; printf '%s' '{}'",
                    serde_json::json!({"decision":"rewrite","value":invalid})
                )),
                command(
                    "cat >/dev/null; : > later-hook-ran; printf '%s' '{\"decision\":\"allow\"}'",
                ),
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
            assert!(run.outcome.is_err(), "{event:?}");
            assert_eq!(run.observations.len(), 1, "{event:?}");
            assert_eq!(run.observations[0].outcome, HookOutcomeKind::Failed);
            assert!(!marker.exists(), "{event:?} admitted later hook");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hostile_hook_diagnostics_stay_out_of_failures_and_annotations_are_projected() {
        let root = tempfile::tempdir().unwrap();
        let hooks = LifecycleHooks {
            pre_turn: vec![command(
                "cat >/dev/null; printf 'api_key=fixture-secret /private/fixture TOOL_RESULT_BODY \\033[31m' >&2; printf '\\377'",
            )],
            post_turn: vec![command(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"api_key=fixture-secret \\u001b[31m\"}'",
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
        assert!(post.failures.is_empty(), "{:?}", post.failures);
        assert_eq!(post.annotations.len(), 1);
        assert_eq!(post.observations[0].outcome, HookOutcomeKind::Annotated);
        assert!(!post.annotations[0].text.contains("fixture-secret"));
        assert!(!post.annotations[0].text.contains('\u{1b}'));
        assert!(post.annotations[0].text.contains("\\u{1b}"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn protocol_orders_rewrites_rejects_wrong_variants_and_continues_post_failures() {
        let root = tempfile::tempdir().unwrap();
        let hooks = LifecycleHooks {
            pre_tool: vec![
                command(
                    "cat >/dev/null; printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"name\":\"file_read\",\"arguments\":{\"path\":\"first\"}}}'",
                ),
                command(
                    "read value; case \"$value\" in *first*) printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"name\":\"file_read\",\"arguments\":{\"path\":\"second\"}}}';; *) exit 9;; esac",
                ),
            ],
            post_tool: vec![
                command("cat >/dev/null; printf trailing"),
                command(
                    "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"safe note\"}'",
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
        assert_eq!(post.failures.len(), 1);
        assert_eq!(post.annotations[0].text, "safe note");
        assert_eq!(post.observations[0].outcome, HookOutcomeKind::Failed);
        assert_eq!(post.observations[1].outcome, HookOutcomeKind::Annotated);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_reaps_the_owned_hook_tree() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("child-survived");
        let script = format!(
            "(sleep 1; printf survived > '{}') & cat >/dev/null; sleep 30",
            marker.display()
        );
        let host = Arc::new(host(root.path(), HookEvent::PreTurn, command(&script)));
        let budget = host.budget();
        let task = tokio::spawn({
            let host = host.clone();
            let budget = budget.clone();
            async move {
                host.run_pre(
                    &budget,
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
        tokio::time::sleep(Duration::from_millis(100)).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        assert!(
            !marker.exists(),
            "owned hook descendant survived cancellation"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shared_budget_counts_overlap_once_and_pauses_during_idle_time() {
        let root = tempfile::tempdir().unwrap();
        let mut hooks = LifecycleHooks {
            max_invocations: 3,
            max_total_ms: 260,
            ..LifecycleHooks::default()
        };
        hooks.pre_turn.push(command("request=$(cat); case \"$request\" in *three*) ;; *) sleep 0.1;; esac; printf '%s' '{\"decision\":\"allow\"}'"));
        let host = host_with_hooks(root.path(), hooks);
        let budget = host.budget();
        let first = host.run_pre(
            &budget,
            HookEvent::PreTurn,
            "one",
            "actor",
            None,
            None,
            serde_json::json!({"input":"one"}),
        );
        let second = host.run_pre(
            &budget,
            HookEvent::PreTurn,
            "two",
            "actor",
            None,
            None,
            serde_json::json!({"input":"two"}),
        );
        let (first, second) = tokio::join!(first, second);
        assert!(first.outcome.is_ok(), "{first:?}");
        assert!(second.outcome.is_ok(), "{second:?}");
        tokio::time::sleep(Duration::from_millis(280)).await;
        let third = host
            .run_pre(
                &budget,
                HookEvent::PreTurn,
                "three",
                "actor",
                None,
                None,
                serde_json::json!({"input":"three"}),
            )
            .await;
        assert!(
            third.outcome.is_ok(),
            "idle time consumed hook budget: {third:?}"
        );
        let fourth = host
            .run_pre(
                &budget,
                HookEvent::PreTurn,
                "four",
                "actor",
                None,
                None,
                serde_json::json!({"input":"four"}),
            )
            .await;
        assert!(fourth.outcome.is_err());
        assert_eq!(fourth.observations[0].outcome, HookOutcomeKind::Failed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn annotation_budget_fails_only_the_excess_post_hook() {
        let root = tempfile::tempdir().unwrap();
        let mut hooks = LifecycleHooks {
            max_annotation_bytes: 5,
            ..LifecycleHooks::default()
        };
        hooks.post_tool = vec![
            command(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"four\"}'",
            ),
            command(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"more\"}'",
            ),
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
        assert_eq!(run.annotations.len(), 1);
        assert_eq!(run.annotations[0].text, "four");
        assert_eq!(run.observations[1].outcome, HookOutcomeKind::Failed);
        assert_eq!(run.failures.len(), 1);
    }

    #[cfg(unix)]
    fn host_with_hooks(root: &std::path::Path, hooks: LifecycleHooks) -> HookHost {
        HookHost::new(
            Arc::new(Directory::open(root, Privacy::Inherited, NameRetention::Pinned).unwrap()),
            hooks,
        )
    }
}
