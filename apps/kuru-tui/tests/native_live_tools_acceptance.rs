//! Explicitly paid native tool/permission acceptance; ordinary CI runs only
//! the synthetic provider and guard cases. The paid and offline routes share
//! the same real interactive UI and bounded child body.
#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use clap::Parser;
use kuru_connectors::{CheckpointStore, Provider, ProviderEvent, ProviderSink, ToolHost, provider};
use kuru_core::{
    Completion, CompletionRequest, Config, ContentBlock, ContextBudget, ContextEstimate, Mode,
    ModelInfo, Sourced, ToolCall, Usage,
};
use kuru_memory::MemoryStore;
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use kuru_runtime::Harness;
use serde_json::{Value, json};

#[path = "support/terminal.rs"]
#[allow(dead_code, reason = "shared helper also serves unrelated PTY fixtures")]
mod terminal;

const MODEL: &str = "gpt-5.6-luna";
const CHILD: &str = "KURU_NATIVE_LIVE_TOOLS_CHILD";
const PROJECT: &str = "KURU_NATIVE_LIVE_TOOLS_PROJECT";
const DEADLINE: Duration = Duration::from_secs(180);
const MAX_REQUESTS: usize = 8;
const MAX_INPUT: u64 = 16_000;
const MAX_TOTAL_INPUT: u64 = 64_000;
const MAX_PAYLOAD: usize = 16 * 1024;
const APPROVED: &str = "approved.txt";
const DENIED: &str = "denied.txt";
const CONTENT: &str = "SYNTHETIC_APPROVED\n";
const DENIED_CONTENT: &str = "SYNTHETIC_DENIED\n";
const FINAL: &str = "SYNTHETIC_TOOLS_ACCEPTED";
const PROMPT: &str = "Synthetic acceptance: in the speaking phase, first call file_write with path approved.txt and content SYNTHETIC_APPROVED followed by one newline. Wait for its result. Then call file_write with path denied.txt and content SYNTHETIC_DENIED followed by one newline. Wait for its result; do not retry a denial. Make each call separately. After both results, answer in one short sentence beginning SYNTHETIC_TOOLS_ACCEPTED describing the approved write and denied write. Do not call any other tools, including cognitive tools.";

#[derive(Default)]
struct Observations {
    requests: usize,
    deliberations: usize,
    speaking: usize,
    input_estimate: u64,
    measured_input: u64,
    output_bytes: usize,
    calls: Vec<String>,
    continuations: usize,
    final_answer: bool,
    usages: Vec<Usage>,
}

fn validate_catalog(models: &[ModelInfo]) -> Result<()> {
    let model = models
        .iter()
        .find(|model| model.id == MODEL)
        .context("required native model not advertised; no inference dispatched")?;
    ensure!(
        model.efforts.iter().any(|effort| effort == "low"),
        "low effort not advertised; no inference dispatched"
    );
    Ok(())
}

fn validate_request(request: &CompletionRequest) -> Result<bool> {
    ensure!(request.model == MODEL, "unexpected effective model");
    ensure!(
        request.effort.as_deref() == Some("low"),
        "unexpected effective effort"
    );
    let speaking = request.instructions.contains("Phase: speak and act");
    ensure!(
        speaking || request.instructions.contains("Phase: deliberate"),
        "unexpected inference phase"
    );
    Ok(speaking)
}

fn admit_request(observed: &mut Observations, input: u64, speaking: bool) -> Result<()> {
    ensure!(
        observed.requests < MAX_REQUESTS,
        "eight-stream dispatch limit"
    );
    ensure!(input <= MAX_INPUT, "per-request input estimate limit");
    ensure!(
        observed.input_estimate.saturating_add(input) <= MAX_TOTAL_INPUT,
        "total input estimate limit"
    );
    observed.requests += 1;
    observed.input_estimate += input;
    if speaking {
        observed.speaking += 1;
    } else {
        observed.deliberations += 1;
    }
    Ok(())
}

fn expected_arguments(index: usize) -> Value {
    if index == 0 {
        json!({"path": APPROVED, "content": CONTENT})
    } else {
        json!({"path": DENIED, "content": DENIED_CONTENT})
    }
}

// A completed call is checked before its event reaches the runtime. This
// guard is additional fixture containment, not a shell sandbox or product rule.
fn validate_completion(
    observed: &mut Observations,
    completion: &Completion,
    speaking: bool,
) -> Result<()> {
    let calls = completion.calls();
    if !speaking {
        ensure!(calls.is_empty(), "unexpected deliberation tool call");
        return Ok(());
    }
    if observed.calls.len() == 2 {
        ensure!(calls.is_empty(), "unexpected call after both decisions");
        ensure!(
            observed.continuations == 2,
            "final answer lacks both continuations"
        );
        ensure!(
            completion.text_projection().contains(FINAL),
            "final acceptance marker absent"
        );
        observed.final_answer = true;
        return Ok(());
    }
    ensure!(calls.len() == 1, "expected one sequential synthetic write");
    let call = &calls[0];
    ensure!(
        call.name == "file_write" && call.arguments == expected_arguments(observed.calls.len()),
        "unexpected tool or synthetic write arguments"
    );
    ensure!(
        !call.id.is_empty()
            && call.id.len() <= 256
            && !call.id.chars().any(char::is_control)
            && !observed.calls.contains(&call.id),
        "invalid or repeated synthetic call identity"
    );
    ensure!(
        observed.calls.len() == observed.continuations,
        "next write lacks previous continuation"
    );
    observed.calls.push(call.id.clone());
    Ok(())
}

fn observe_continuation(observed: &mut Observations, request: &CompletionRequest) -> Result<()> {
    if observed.calls.len() == observed.continuations {
        return Ok(());
    }
    ensure!(
        observed.calls.len() == observed.continuations + 1,
        "ambiguous pending call"
    );
    let call = &observed.calls[observed.continuations];
    let current = request
        .current_message_count
        .context("current continuation boundary absent")?;
    ensure!(
        current <= request.messages.len(),
        "invalid current continuation boundary"
    );
    let results = request.messages[request.messages.len() - current..]
        .iter()
        .flat_map(|message| &message.blocks)
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                call_id,
                output,
                is_error,
            } if call_id == call => Some((output, *is_error)),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        results.len() == 1,
        "matching current tool-result continuation absent"
    );
    let (output, is_error) = results[0];
    let text = output
        .as_str()
        .context("unexpected tool-result receipt type")?;
    if observed.continuations == 0 {
        ensure!(
            !is_error && text.starts_with("file_write completed; checkpoint "),
            "approved write lacks successful checkpoint receipt"
        );
    } else {
        ensure!(
            is_error && text.starts_with("ERROR: tool permission denied"),
            "denied write lacks permission-denied receipt"
        );
    }
    observed.continuations += 1;
    Ok(())
}

// Native errors may carry response bodies or URLs. Only these fixed categories
// cross the fixture boundary; successful reports contain scalars only.
fn safe_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(incompatibility) = kuru_connectors::incompatibility(&error) {
        return anyhow::anyhow!(
            "native tool acceptance incompatible: {}",
            incompatibility.code().as_str()
        );
    }
    let class = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .map_or("native-provider", |http| {
            if http.is_timeout() {
                "network-timeout"
            } else if http.is_connect() {
                "network-connect"
            } else if http
                .status()
                .is_some_and(|status| matches!(status.as_u16(), 401 | 403))
            {
                "http-auth"
            } else {
                "http-transport"
            }
        });
    anyhow::anyhow!("native tool acceptance failed: {class}; raw diagnostic withheld")
}

struct BoundedProvider {
    native: Arc<dyn Provider>,
    models: Vec<ModelInfo>,
    observed: Arc<Mutex<Observations>>,
}

#[async_trait]
impl Provider for BoundedProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }

    async fn estimate_context(&self, request: &CompletionRequest) -> Result<ContextEstimate> {
        validate_request(request)?;
        self.native
            .estimate_context(request)
            .await
            .map_err(safe_error)
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let speaking = validate_request(&request)?;
        let estimate = self
            .native
            .estimate_context(&request)
            .await
            .map_err(safe_error)?;
        {
            let mut observed = self.observed.lock().unwrap();
            if speaking {
                observe_continuation(&mut observed, &request)?;
            }
            admit_request(&mut observed, estimate.estimated_input_tokens, speaking)?;
        }
        let mut bounded = BoundedSink {
            downstream: sink,
            observed: self.observed.clone(),
            speaking,
            stream_bytes: 0,
            settled_bytes: 0,
        };
        self.native
            .stream(request, &mut bounded)
            .await
            .map_err(safe_error)
    }
}

struct BoundedSink<'a> {
    downstream: &'a mut dyn ProviderSink,
    observed: Arc<Mutex<Observations>>,
    speaking: bool,
    stream_bytes: usize,
    settled_bytes: usize,
}

impl ProviderSink for BoundedSink<'_> {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            {
                let mut observed = self.observed.lock().unwrap();
                let previous = self.stream_bytes.max(self.settled_bytes);
                match &event {
                    ProviderEvent::ContextMeasured(estimate) => {
                        ensure!(
                            estimate.estimated_input_tokens <= MAX_INPUT,
                            "native measured input limit"
                        );
                        observed.measured_input = observed
                            .measured_input
                            .saturating_add(estimate.estimated_input_tokens);
                        ensure!(
                            observed.measured_input <= MAX_TOTAL_INPUT,
                            "total native measured input limit"
                        );
                    }
                    ProviderEvent::TextDelta { text, .. }
                    | ProviderEvent::ReasoningSummaryDelta { text, .. } => {
                        self.stream_bytes = self.stream_bytes.saturating_add(text.len());
                    }
                    ProviderEvent::ToolCallDelta {
                        arguments_fragment, ..
                    } => {
                        self.stream_bytes =
                            self.stream_bytes.saturating_add(arguments_fragment.len());
                    }
                    ProviderEvent::SettledReasoningSummaries(summaries) => {
                        self.settled_bytes = self.settled_bytes.saturating_add(
                            summaries
                                .iter()
                                .map(|summary| summary.text.len())
                                .sum::<usize>(),
                        );
                    }
                    ProviderEvent::Completed(completion) => {
                        validate_completion(&mut observed, completion, self.speaking)?;
                        self.settled_bytes = self
                            .settled_bytes
                            .saturating_add(serde_json::to_vec(&completion.blocks)?.len());
                        observed.usages.push(completion.usage.clone());
                    }
                    _ => {}
                }
                observed.output_bytes = observed
                    .output_bytes
                    .saturating_add(self.stream_bytes.max(self.settled_bytes) - previous);
                ensure!(
                    observed.output_bytes <= MAX_PAYLOAD,
                    "observed output payload byte limit"
                );
            }
            self.downstream.emit(event).await
        })
    }
}

struct SyntheticProvider;

fn synthetic_models() -> Vec<ModelInfo> {
    vec![ModelInfo {
        id: MODEL.into(),
        name: MODEL.into(),
        efforts: vec!["low".into()],
        default_effort: Some("low".into()),
        metadata: Default::default(),
    }]
}

#[async_trait]
impl Provider for SyntheticProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(synthetic_models())
    }

    async fn estimate_context(&self, request: &CompletionRequest) -> Result<ContextEstimate> {
        Ok(ContextEstimate::for_final_body(
            request
                .context_budget
                .clone()
                .unwrap_or(ContextBudget::resolve(
                    Sourced::built_in(200_000),
                    None,
                    Some(1024),
                )?),
            serde_json::to_vec(request)?.len() as u64,
            false,
            vec![],
        ))
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let speaking = validate_request(&request)?;
        let results = request
            .messages
            .iter()
            .flat_map(|message| &message.blocks)
            .filter(|block| matches!(block, ContentBlock::ToolResult { .. }))
            .count();
        let completion = if !speaking {
            Completion::from_legacy(
                "Perform only the two synthetic writes separately in the speaking phase.",
                vec![],
                8,
                5,
            )
        } else if results < 2 {
            Completion::from_legacy(
                "",
                vec![ToolCall {
                    id: format!("synthetic-call-{results}"),
                    name: "file_write".into(),
                    arguments: expected_arguments(results),
                }],
                8,
                5,
            )
        } else {
            Completion::from_legacy(
                format!("{FINAL}: approved write succeeded and the second write was denied."),
                vec![],
                8,
                5,
            )
        };
        sink.emit(ProviderEvent::Completed(completion)).await
    }
}

async fn acceptance(
    project: &Path,
    offline: bool,
    observed: Arc<Mutex<Observations>>,
) -> Result<()> {
    let config = Config {
        mode: Mode::Freudian,
        provider: "codex".into(),
        model: MODEL.into(),
        effort: Some("low".into()),
        max_rounds: 1,
        max_tool_calls: 2,
        dream_every: 0,
        dream_on_exit: false,
        context_output_reserve_tokens: Some(1024),
        ..Config::default()
    };
    let native: Arc<dyn Provider> = if offline {
        Arc::new(SyntheticProvider)
    } else {
        // Only the native auth API opens Kuru's own store. This fixture never
        // reads, prints or copies credential bytes, or loads project config.
        let cli = kuru::cli::Cli::try_parse_from([
            std::ffi::OsStr::new("kuru"),
            std::ffi::OsStr::new("-C"),
            project.as_os_str(),
        ])?;
        let (_, auth, _) = kuru::cli::paths(&cli)?;
        provider(&config, project, &auth)
            .await
            .map_err(safe_error)?
    };
    let models = native.models().await.map_err(safe_error)?;
    validate_catalog(&models)?;
    println!(
        "LIVE_TOOLS_MODEL model={MODEL} effort=low offline={offline} hard_output_token_cap=false"
    );
    let wrapped = Arc::new(BoundedProvider {
        native,
        models: models.clone(),
        observed: observed.clone(),
    });
    let private = tempfile::tempdir()?;
    let root = Arc::new(Directory::open(
        project,
        Privacy::Inherited,
        NameRetention::Pinned,
    )?);
    let checkpoints = Arc::new(CheckpointStore::new(
        &private.path().join("checkpoints"),
        root.clone(),
    )?);
    let tools = ToolHost::with_retained_root(root, &config)?.with_checkpoint_store(checkpoints)?;
    let memory = MemoryStore::temporary().await?;
    let harness = Harness::with_tool_host_and_instructions(
        config, project,
        "This is an isolated synthetic acceptance fixture. In deliberate phases answer briefly without tools or peer routing. Only the speaking phase may perform the two sequential file_write calls requested by the user. Do not use cognitive tools. Never retry a permission denial.".into(),
        memory.clone(), wrapped, None, tools,
    ).await?;
    // Public UI submissions have no actor-target syntax. Three ordinary
    // Freudian deliberations plus three speaking requests fit the eight-stream
    // cap; one turn supplies both real approval decisions without a profile edit.
    println!(
        "LIVE_TOOLS_READY project={} approve_path={APPROVED} deny_path={DENIED}",
        project.display()
    );
    kuru::ui::run(harness, models).await?;
    ensure!(
        std::fs::read(project.join(APPROVED))? == CONTENT.as_bytes(),
        "approved file differs from exact synthetic bytes"
    );
    ensure!(
        !project.join(DENIED).try_exists()?,
        "denied write changed the project"
    );
    {
        let observed = observed.lock().unwrap();
        ensure!(
            observed.calls.len() == 2 && observed.continuations == 2 && observed.final_answer,
            "native tool chain did not complete both decisions"
        );
        ensure!(
            observed.deliberations == 3 && observed.speaking == 3,
            "unexpected public UI dispatch counts"
        );
        println!(
            "LIVE_TOOLS_RESULT requests={} deliberations={} speaking={} estimated_input={} native_measured_input={} observed_payload_bytes={} approved_exact=true denied_absent=true matching_continuations=2 final_answer=true",
            observed.requests,
            observed.deliberations,
            observed.speaking,
            observed.input_estimate,
            observed.measured_input,
            observed.output_bytes
        );
    }
    memory.close().await?;
    Ok(())
}

fn child(offline: bool) -> Result<()> {
    // An optional explicit, nonexistent project lets the visible cmux driver
    // inspect only known synthetic paths. create_dir fails closed on reuse.
    let owned = tempfile::tempdir()?;
    let project =
        std::env::var_os(PROJECT).map_or_else(|| owned.path().join("project"), PathBuf::from);
    ensure!(
        project.is_absolute() && project.file_name() == Some(std::ffi::OsStr::new("project")),
        "synthetic project must be an absolute fresh project directory"
    );
    std::fs::create_dir(&project)?;
    let observed = Arc::new(Mutex::new(Observations::default()));
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?.block_on(kuru_memory::test_support::closing(async {
        let result = tokio::time::timeout(DEADLINE, acceptance(&project, offline, observed.clone())).await
            .map_err(|_| anyhow::anyhow!("bounded native tool acceptance deadline expired"))
            .and_then(|result| result);
        let observed = observed.lock().unwrap();
        println!("LIVE_TOOLS_ATTEMPT acceptance_succeeded={} dispatched_streams={} terminal_completions={} continuations={} estimated_input={} native_measured_input={} observed_payload_bytes={}",
            result.is_ok(), observed.requests, observed.usages.len(), observed.continuations, observed.input_estimate, observed.measured_input, observed.output_bytes);
        for (index, usage) in observed.usages.iter().enumerate() {
            println!("LIVE_TOOLS_USAGE completion={index} input={:?} output={:?} cached={:?} reasoning={:?}", usage.input_tokens, usage.output_tokens, usage.cached_input_tokens, usage.reasoning_output_tokens);
        }
        result
    }))
}

fn parent(offline: bool, name: &str) -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let project = scratch.path().join("project");
    let deadline = Instant::now() + DEADLINE;
    let remaining = || -> Result<Duration> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "parent native tool deadline expired");
        Ok(remaining)
    };
    let mut command = Command::new(std::env::current_exe()?);
    command.args(["--exact", name, "--nocapture"]);
    if !offline {
        command.arg("--ignored");
    }
    command
        .env(CHILD, "1")
        .env(PROJECT, &project)
        .env("RUST_LOG", "off")
        .env("TERM", "xterm-256color")
        .env("KURU_REDUCED_MOTION", "1");
    let mut terminal = terminal::Terminal::spawn(command, 36, 120)?;
    terminal.wait_composer_frame(&["KURU", "enter send"], remaining()?)?;
    terminal.send(format!("{PROMPT}\r").as_bytes())?;
    terminal.wait_composer_frame(
        &["Permission request", APPROVED, "1 once", "also Alt+digit"],
        remaining()?,
    )?;
    ensure!(
        !project.join(APPROVED).try_exists()? && !project.join(DENIED).try_exists()?,
        "write ran while first approval pending"
    );
    terminal.send(b"\x1b1")?;
    terminal.wait_composer_frame(&["Permission request", DENIED, "4 deny"], remaining()?)?;
    ensure!(
        std::fs::read(project.join(APPROVED))? == CONTENT.as_bytes(),
        "approved write did not settle exactly"
    );
    ensure!(
        !project.join(DENIED).try_exists()?,
        "write ran while denial pending"
    );
    terminal.send(b"\x1b4")?;
    terminal.wait_composer_frame(&[FINAL, "enter send"], remaining()?)?;
    ensure!(!project.join(DENIED).try_exists()?, "denied write ran");
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(remaining()?)?;
    terminal.assert_restored()?;
    let output = String::from_utf8_lossy(&terminal.output);
    ensure!(
        output.contains("LIVE_TOOLS_RESULT"),
        "child did not validate final tool outcomes"
    );
    println!(
        "LIVE_TOOLS_TERMINAL pending_no_effect=true approve_once=true deny=true restoration=true child_exit=0 offline={offline}"
    );
    for line in output.lines() {
        if let Some(start) = [
            "LIVE_TOOLS_MODEL ",
            "LIVE_TOOLS_RESULT ",
            "LIVE_TOOLS_ATTEMPT ",
            "LIVE_TOOLS_USAGE ",
        ]
        .into_iter()
        .find_map(|prefix| line.find(prefix))
        {
            let scalar = &line[start..];
            ensure!(
                scalar.len() <= 1024
                    && scalar
                        .chars()
                        .all(|character| character.is_ascii_graphic() || character == ' '),
                "bounded scalar report"
            );
            println!("{scalar}");
        }
    }
    Ok(())
}

#[test]
#[ignore = "paid native gpt-5.6-luna tool acceptance; requires Kuru's own existing login"]
fn native_live_tools_acceptance() -> Result<()> {
    if std::env::var_os(CHILD).is_some() {
        child(false)
    } else {
        parent(false, "native_live_tools_acceptance")
    }
}

#[test]
fn offline_native_tools_acceptance() -> Result<()> {
    if std::env::var_os(CHILD).is_some() {
        child(true)
    } else {
        parent(true, "offline_native_tools_acceptance")
    }
}

#[test]
fn native_tool_guard_rejects_catalog_drift_and_dispatch_overruns() {
    assert!(validate_catalog(&[]).is_err());
    let mut models = synthetic_models();
    models[0].id = "other-model".into();
    assert!(validate_catalog(&models).is_err());
    models[0].id = MODEL.into();
    models[0].efforts = vec!["medium".into()];
    assert!(validate_catalog(&models).is_err());
    let mut observed = Observations::default();
    assert!(admit_request(&mut observed, MAX_INPUT + 1, true).is_err());
    assert_eq!(observed.requests, 0);
    for _ in 0..4 {
        admit_request(&mut observed, MAX_INPUT, true).unwrap();
    }
    assert!(admit_request(&mut observed, 1, true).is_err());
    assert_eq!(observed.requests, 4);
    let mut observed = Observations::default();
    for _ in 0..MAX_REQUESTS {
        admit_request(&mut observed, 1, false).unwrap();
    }
    assert!(admit_request(&mut observed, 1, false).is_err());
    assert_eq!(observed.requests, MAX_REQUESTS);
}

#[test]
fn native_tool_guard_rejects_malformed_unexpected_and_batched_calls_before_dispatch() {
    for calls in [
        vec![ToolCall {
            id: "call".into(),
            name: "shell".into(),
            arguments: json!({"command":"true"}),
        }],
        vec![ToolCall {
            id: "call".into(),
            name: "file_write".into(),
            arguments: json!({"path":"../outside", "content":CONTENT}),
        }],
        vec![ToolCall {
            id: "call".into(),
            name: "file_write".into(),
            arguments: json!({"path":APPROVED, "content":0}),
        }],
        vec![
            ToolCall {
                id: "call".into(),
                name: "file_write".into(),
                arguments: expected_arguments(0),
            },
            ToolCall {
                id: "second".into(),
                name: "file_write".into(),
                arguments: expected_arguments(1),
            },
        ],
    ] {
        let mut observed = Observations::default();
        assert!(
            validate_completion(
                &mut observed,
                &Completion::from_legacy("", calls, 0, 0),
                true
            )
            .is_err()
        );
        assert!(observed.calls.is_empty());
    }
    let mut observed = Observations::default();
    let valid = Completion::from_legacy(
        "",
        vec![ToolCall {
            id: "call".into(),
            name: "file_write".into(),
            arguments: expected_arguments(0),
        }],
        0,
        0,
    );
    assert!(validate_completion(&mut observed, &valid, false).is_err());
    assert!(observed.calls.is_empty());
}

fn synthetic_request() -> CompletionRequest {
    CompletionRequest {
        actor: "synthetic-actor".into(),
        instructions: "Phase: speak and act".into(),
        shared_instruction_prefix_bytes: None,
        messages: vec![],
        current_message_count: Some(0),
        context_budget: None,
        model: MODEL.into(),
        effort: Some("low".into()),
        tools: vec![],
    }
}

#[test]
fn native_tool_guard_rejects_effective_model_effort_and_unmatched_continuation() {
    let mut request = synthetic_request();
    request.model = "other-model".into();
    assert!(validate_request(&request).is_err());
    request.model = MODEL.into();
    request.effort = Some("medium".into());
    assert!(validate_request(&request).is_err());
    request.effort = Some("low".into());
    request.instructions = "Phase: dream".into();
    assert!(validate_request(&request).is_err());
    let mut observed = Observations {
        calls: vec!["approved-call".into()],
        ..Default::default()
    };
    request = synthetic_request();
    request.messages = vec![kuru_core::Message::tool_result(
        "wrong-call",
        json!("file_write completed; checkpoint synthetic"),
        false,
    )];
    request.current_message_count = Some(1);
    assert!(observe_continuation(&mut observed, &request).is_err());
    assert_eq!(observed.continuations, 0);
    request.messages[0] = kuru_core::Message::tool_result(
        "approved-call",
        json!("ERROR: tool permission denied"),
        true,
    );
    assert!(observe_continuation(&mut observed, &request).is_err());
    request.messages[0] = kuru_core::Message::tool_result(
        "approved-call",
        json!("file_write completed; checkpoint synthetic"),
        false,
    );
    request.current_message_count = Some(0);
    assert!(observe_continuation(&mut observed, &request).is_err());
    request.current_message_count = Some(1);
    observe_continuation(&mut observed, &request).unwrap();
    assert_eq!(observed.continuations, 1);
    observed.calls.push("denied-call".into());
    request.messages[0] =
        kuru_core::Message::tool_result("denied-call", json!("ERROR: other failure"), true);
    assert!(observe_continuation(&mut observed, &request).is_err());
    assert_eq!(observed.continuations, 1);
    request.messages[0] = kuru_core::Message::tool_result(
        "denied-call",
        json!("ERROR: tool permission denied: tool permission was denied"),
        true,
    );
    observe_continuation(&mut observed, &request).unwrap();
    assert_eq!(observed.continuations, 2);
}

struct CountingSink(Arc<Mutex<usize>>);

impl ProviderSink for CountingSink {
    fn emit<'a>(
        &'a mut self,
        _: ProviderEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            *self.0.lock().unwrap() += 1;
            Ok(())
        })
    }
}

#[test]
fn native_tool_guard_withholds_unsafe_completions_and_over_cap_payload() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for (completion, speaking) in [
        (
            Completion::from_legacy(
                "",
                vec![ToolCall {
                    id: "unsafe-call".into(),
                    name: "shell".into(),
                    arguments: json!({"command":"true"}),
                }],
                0,
                0,
            ),
            true,
        ),
        (
            Completion::from_legacy("x".repeat(MAX_PAYLOAD + 1), vec![], 0, 0),
            false,
        ),
    ] {
        let forwarded = Arc::new(Mutex::new(0));
        let mut downstream = CountingSink(forwarded.clone());
        let mut bounded = BoundedSink {
            downstream: &mut downstream,
            observed: Default::default(),
            speaking,
            stream_bytes: 0,
            settled_bytes: 0,
        };
        assert!(
            runtime
                .block_on(bounded.emit(ProviderEvent::Completed(completion)))
                .is_err()
        );
        assert_eq!(
            *forwarded.lock().unwrap(),
            0,
            "unsafe terminal event reached runtime dispatch"
        );
    }
    Ok(())
}
