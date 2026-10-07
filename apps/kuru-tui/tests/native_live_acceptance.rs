//! Explicitly paid, synthetic acceptance. Ordinary CI never selects this test.
#![cfg(unix)]

use std::{
    io,
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use clap::Parser;
use kuru::ui::{self, TerminalSession, View};
use kuru_connectors::{Provider, ProviderEvent, ProviderSink, ToolHost, provider};
use kuru_core::{
    CompletionRequest, Config, ContextEstimate, HookCommand, LifecycleHooks, Mode, ModelInfo, Usage,
};
use kuru_memory::{MemoryStore, StorageRecord};
use kuru_runtime::Harness;
use ratatui::{Terminal as Renderer, backend::CrosstermBackend};
use sha2::{Digest, Sha256};

#[path = "support/terminal.rs"]
#[allow(
    dead_code,
    reason = "shared PTY helper also supports unrelated offline fixtures"
)]
mod terminal;

const MODEL: &str = "gpt-5.6-luna";
const DEADLINE: Duration = Duration::from_secs(180);
const CHILD: &str = "KURU_NATIVE_LIVE_ACCEPTANCE_CHILD";

#[derive(Default)]
struct Observations {
    requests: usize,
    input_estimate: u64,
    measured_input: u64,
    output_bytes: usize,
    usages: Vec<Usage>,
    prefixes: Vec<[u8; 32]>,
    tools: Vec<[u8; 32]>,
    summaries: usize,
    tool_deltas: usize,
    rewritten: bool,
}

struct NativeObservation {
    native: Arc<dyn Provider>,
    models: Vec<ModelInfo>,
    effort: Option<String>,
    observed: Arc<Mutex<Observations>>,
}

// Keep provider diagnostics out of the PTY and libtest's panic report. Native
// transports may include request URLs or response bodies in their error chain.
fn safe_error(error: anyhow::Error) -> anyhow::Error {
    if let Some(incompatibility) = kuru_connectors::incompatibility(&error) {
        return anyhow::anyhow!(
            "live acceptance incompatible: {}",
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
            } else if let Some(status) = http.status() {
                if status.as_u16() == 401 || status.as_u16() == 403 {
                    "http-auth"
                } else {
                    "http-status"
                }
            } else {
                "http-transport"
            }
        });
    anyhow::anyhow!("live acceptance failed: {class}; raw diagnostic withheld")
}

#[async_trait]
impl Provider for NativeObservation {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }

    async fn estimate_context(&self, request: &CompletionRequest) -> Result<ContextEstimate> {
        self.native
            .estimate_context(request)
            .await
            .map_err(safe_error)
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        ensure!(request.model == MODEL, "unexpected effective model");
        ensure!(request.effort == self.effort, "unexpected effective effort");
        let estimate = self
            .native
            .estimate_context(&request)
            .await
            .map_err(safe_error)?;
        {
            let mut observed = self.observed.lock().unwrap();
            ensure!(observed.requests < 6, "six-request live acceptance limit");
            ensure!(
                estimate.estimated_input_tokens <= 16_000,
                "per-request input estimate limit"
            );
            ensure!(
                observed.input_estimate + estimate.estimated_input_tokens <= 64_000,
                "total input estimate limit"
            );
            observed.requests += 1;
            observed.input_estimate += estimate.estimated_input_tokens;
            // This is only a neutral CompletionRequest prefix, not final wire bytes.
            let prefix = request
                .instructions
                .split("\nYou are ")
                .next()
                .unwrap_or(&request.instructions);
            observed
                .prefixes
                .push(Sha256::digest(prefix.as_bytes()).into());
            observed
                .tools
                .push(Sha256::digest(serde_json::to_vec(&request.tools)?).into());
            observed.rewritten |= request.messages.iter().any(|message| {
                message
                    .plain_text()
                    .is_some_and(|text| text.contains("LIVE_REWRITTEN"))
            });
        }
        let mut bounded = BoundedSink {
            downstream: sink,
            observed: self.observed.clone(),
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
                            estimate.estimated_input_tokens <= 16_000,
                            "native measured input limit"
                        );
                        observed.measured_input += estimate.estimated_input_tokens;
                        ensure!(
                            observed.measured_input <= 64_000,
                            "total native measured input limit"
                        );
                    }
                    ProviderEvent::TextDelta { text, .. }
                    | ProviderEvent::ReasoningSummaryDelta { text, .. } => {
                        self.stream_bytes += text.len()
                    }
                    ProviderEvent::SettledReasoningSummaries(summaries) => {
                        observed.summaries += summaries.len();
                        self.settled_bytes += summaries
                            .iter()
                            .map(|summary| summary.text.len())
                            .sum::<usize>();
                    }
                    ProviderEvent::ToolCallDelta {
                        arguments_fragment, ..
                    } => {
                        observed.tool_deltas += 1;
                        self.stream_bytes += arguments_fragment.len();
                    }
                    ProviderEvent::Completed(completion) => {
                        self.settled_bytes += serde_json::to_vec(&completion.blocks)?.len();
                        observed.usages.push(completion.usage.clone())
                    }
                    _ => {}
                }
                // Bound both stream and terminal reconciliation, without
                // charging the same delivered text twice. This is a local
                // payload bound, not a server output-token generation cap.
                observed.output_bytes += self.stream_bytes.max(self.settled_bytes) - previous;
                ensure!(
                    observed.output_bytes <= 16 * 1024,
                    "observed output payload byte limit"
                );
            }
            self.downstream.emit(event).await
        })
    }
}

fn hook(script: &str) -> HookCommand {
    HookCommand {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        timeout_ms: 2_000,
        max_output_bytes: 1024,
    }
}

async fn private_summaries(memory: &MemoryStore) -> Result<Vec<(String, serde_json::Value)>> {
    let export = memory.begin_active_export().await?;
    let mut cursor = None;
    let mut summaries = Vec::new();
    for _ in 0..8 {
        let page = export.page(cursor).await?;
        for record in page.records {
            if let StorageRecord::State { key, value } = record
                && key.starts_with("kuru/private/reasoning-summary/")
            {
                summaries.push((key, value));
            }
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    ensure!(cursor.is_none(), "bounded synthetic export exhausted");
    summaries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(summaries)
}

async fn acceptance(observed: Arc<Mutex<Observations>>) -> Result<()> {
    // Existing CLI path resolution selects only factory authentication here;
    // no command, configuration activation or memory open is performed.
    let project = tempfile::tempdir()?;
    std::fs::write(
        project.path().join("fixture.txt"),
        "SYNTHETIC_FILE_VALUE=seventeen\n",
    )?;
    let cli = kuru::cli::Cli::try_parse_from([
        std::ffi::OsStr::new("kuru"),
        std::ffi::OsStr::new("-C"),
        project.path().as_os_str(),
    ])?;
    let (_, auth, _) = kuru::cli::paths(&cli)?;
    ensure!(auth.is_absolute(), "native auth directory must be absolute");
    let mut config = Config {
        mode: Mode::Freudian,
        provider: "codex".into(),
        model: MODEL.into(),
        max_rounds: 1,
        max_tool_calls: 2,
        dream_every: 0,
        dream_on_exit: false,
        context_output_reserve_tokens: Some(1024),
        ..Config::default()
    };
    config.hooks = LifecycleHooks {
        pre_turn: vec![hook(
            r#"input=$(cat); case "$input" in *LIVE_ORIGINAL*) printf '%s' '{"decision":"rewrite","value":{"input":"LIVE_REWRITTEN: Read fixture.txt using the existing read tool if useful, then answer in one sentence with SYNTHETIC_ACCEPTED and its value. Do not mutate files."}}' ;; *) printf '%s' '{"decision":"allow"}' ;; esac"#,
        )],
        post_turn: vec![hook(
            r#"cat >/dev/null; printf '%s' '{"decision":"annotate","annotation":"SYNTHETIC_POST_ANNOTATION"}'"#,
        )],
        pre_tool: vec![hook(
            r#"cat >/dev/null; printf '%s' '{"decision":"allow"}'"#,
        )],
        post_tool: vec![hook(
            r#"cat >/dev/null; printf '%s' '{"decision":"annotate","annotation":"SYNTHETIC_TOOL_ANNOTATION"}'"#,
        )],
        ..LifecycleHooks::default()
    };
    let native = provider(&config, project.path(), &auth)
        .await
        .map_err(safe_error)?;
    let models = native.models().await.map_err(safe_error)?;
    let selected = models
        .iter()
        .find(|model| model.id == MODEL)
        .ok_or_else(|| anyhow::anyhow!("required native model not advertised"))?;
    config.effort = selected
        .efforts
        .iter()
        .find(|effort| effort.as_str() == "low")
        .cloned();
    ensure!(
        config.effort.is_some(),
        "low effort not advertised; no inference dispatched"
    );
    println!(
        "LIVE_MODEL model={MODEL} effort=low capability_count={} hard_output_token_cap=false",
        selected.metadata.capabilities.len()
    );
    let wrapped = Arc::new(NativeObservation {
        native,
        models: models.clone(),
        effort: config.effort.clone(),
        observed: observed.clone(),
    });
    let memory = MemoryStore::temporary().await?;
    let tools = ToolHost::new(project.path(), &config)?;
    let mut instructions = String::from(
        "This is an entirely synthetic bounded acceptance project. Answer briefly. Never shell, mutate, contact peers or use external tools. The fixture file is safe to read.\n",
    );
    for index in 0..96 {
        instructions.push_str(&format!("Synthetic shared catalogue row {index:03}: amber square, indigo triangle, silver circle; values are invented and carry no real personal information. Preserve this fixed catalogue as shared reference, never recite it.\n"));
    }
    let mut harness = Harness::with_tool_host_and_instructions(
        config.clone(),
        project.path(),
        instructions.clone(),
        memory.clone(),
        wrapped.clone(),
        None,
        tools,
    )
    .await?;
    let session = harness.session.id.clone();
    let actors = harness
        .topology
        .parts
        .iter()
        .take(2)
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let mut guard = TerminalSession::enter(&mut io::stdout())?;
    let mut renderer = Renderer::new(CrosstermBackend::new(io::stdout()))?;
    let mut preview_frames = 0usize;
    let mut summary_frames = 0usize;
    let mut events = harness.subscribe();
    for (index, actor) in actors.iter().enumerate() {
        let mut view =
            View::from_initial(ui::project_initial_view(&harness).await?, models.clone());
        view.busy = true;
        view.speaker.clone_from(actor);
        let mut progress = harness.subscribe_progress();
        let mut progress_open = true;
        let input = if index == 0 {
            "LIVE_ORIGINAL"
        } else {
            "From your own perspective, answer in one short sentence beginning SYNTHETIC_ACCEPTED about the previous public answer. Do not use tools."
        };
        let turn = harness.run_for(input, Some(actor));
        tokio::pin!(turn);
        loop {
            tokio::select! {
                result = &mut turn => {
                    let output = result.map_err(safe_error)?;
                    ensure!(!output.limited, "live turn reached runtime limit");
                    ensure!(!output.text.is_empty(), "empty live public answer");
                    ensure!(progress.borrow().is_none(), "settled facing preview was retained");
                    view.preview = None;
                    view.busy = false;
                    renderer.draw(|frame| ui::draw(frame, &view))?;
                    break;
                }
                changed = progress.changed(), if progress_open => {
                    if changed.is_err() { progress_open = false; continue; }
                    view.preview = progress.borrow_and_update().clone();
                    if let Some(preview) = &view.preview {
                        ensure!(preview.text_tail.len() <= 8192 && preview.summary_tail.len() <= 2048, "facing preview bounds");
                        preview_frames += 1;
                        summary_frames += usize::from(!preview.summary_tail.is_empty());
                    }
                    renderer.draw(|frame| ui::draw(frame, &view))?;
                }
            }
        }
    }
    guard.restore()?;
    let hook_events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    ensure!(hook_events.iter().any(|event| matches!(event, kuru_runtime::Event::Hook { observation, .. } if observation.event == "pre_turn" && observation.outcome == "rewritten")), "native turn lacked observed rewrite hook");
    ensure!(hook_events.iter().filter(|event| matches!(event, kuru_runtime::Event::Hook { observation, .. } if observation.event == "post_turn" && observation.outcome == "annotated")).count() == 2, "native turns lacked observed annotation hooks");
    let history = harness.history().await?;
    ensure!(
        history.iter().any(|message| message
            .plain_text()
            .is_some_and(|text| text.contains("LIVE_ORIGINAL"))),
        "original public input lost"
    );
    let summaries = private_summaries(&memory).await?;
    ensure!(
        observed.lock().unwrap().summaries == 0 || !summaries.is_empty(),
        "delivered private summaries were not persisted"
    );
    for (_, value) in &summaries {
        ensure!(
            value["session_id"] == session,
            "summary belongs to wrong synthetic session"
        );
        ensure!(
            (value["turn_id"].as_str().is_some_and(|id| !id.is_empty())
                || value["operation_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty()))
                && value["invocation_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty()),
            "private summary identity absent"
        );
        let text = value["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("summary text absent"))?;
        ensure!(
            !history
                .iter()
                .any(|message| message.plain_text() == Some(text)),
            "private summary leaked into public history"
        );
        let actor = value["actor_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("summary actor absent"))?;
        ensure!(
            actors.iter().any(|expected| expected == actor),
            "summary actor outside selected synthetic actors"
        );
        ensure!(
            !harness
                .memory_for(actor)
                .await?
                .iter()
                .any(|message| message.plain_text() == Some(text)),
            "private summary entered fresh provider context"
        );
    }
    harness.shutdown(false).await?;
    drop(harness);
    let resumed = Harness::with_tool_host_and_instructions(
        config.clone(),
        project.path(),
        instructions,
        memory.clone(),
        wrapped,
        Some(&session),
        ToolHost::new(project.path(), &config)?,
    )
    .await?;
    let resumed_summaries = private_summaries(&memory).await?;
    ensure!(
        resumed_summaries == summaries,
        "exact private records changed across resume"
    );
    for (_, value) in &resumed_summaries {
        let actor = value["actor_id"].as_str().unwrap();
        let text = value["text"].as_str().unwrap();
        ensure!(
            !resumed
                .memory_for(actor)
                .await?
                .iter()
                .any(|message| message.plain_text() == Some(text)),
            "resumed fresh context disclosed private summary"
        );
    }
    let replay = ui::project_initial_view(&resumed).await?;
    ensure!(
        replay
            .transcript
            .iter()
            .any(|(_, text)| text.contains("SYNTHETIC_ACCEPTED")),
        "ordinary replay lost synthetic public reply"
    );
    for (_, value) in &resumed_summaries {
        let text = value["text"].as_str().unwrap();
        ensure!(
            !replay.transcript.iter().any(|(_, body)| body == text),
            "ordinary replay disclosed private summary"
        );
    }
    println!("LIVE_REPLAY_READY");
    ui::run(resumed, models).await.map_err(safe_error)?;
    memory.close().await?;
    let observed = observed.lock().unwrap();
    ensure!(observed.requests <= 6, "request bound exceeded");
    ensure!(
        observed.rewritten,
        "pre-turn rewrite not projected to native request"
    );
    let stable_prefix = observed.prefixes.windows(2).all(|pair| pair[0] == pair[1]);
    let stable_tools = observed.tools.windows(2).all(|pair| pair[0] == pair[1]);
    ensure!(
        stable_prefix,
        "shared CompletionRequest instruction prefix changed"
    );
    println!(
        "LIVE_RESULT requests={} estimated_input={} native_measured_input={} observed_payload_bytes={} preview_frames={preview_frames} summary_frames={summary_frames} private_summaries={} settled_summaries={} tool_deltas={} shared_request_prefix_stable={stable_prefix} tools_stable={stable_tools} final_wire_proof=false post_annotations=2 rewrite_observed=true cleared_settled_frames=2",
        observed.requests,
        observed.input_estimate,
        observed.measured_input,
        observed.output_bytes,
        summaries.len(),
        observed.summaries,
        observed.tool_deltas
    );
    Ok(())
}

#[test]
#[ignore = "paid native gpt-5.6-luna acceptance; requires Kuru's own existing login"]
fn native_live_acceptance() -> Result<()> {
    if std::env::var_os(CHILD).is_some() {
        let observed = Arc::new(Mutex::new(Observations::default()));
        return tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(kuru_memory::test_support::closing(async {
                let result = tokio::time::timeout(DEADLINE, acceptance(observed.clone()))
                    .await
                    .map_err(|_| anyhow::anyhow!("bounded native acceptance deadline expired"))
                    .and_then(|outcome| outcome);
                let observed = observed.lock().unwrap();
                println!("LIVE_ATTEMPT acceptance_succeeded={} delegated_streams={} terminal_completions={}", result.is_ok(), observed.requests, observed.usages.len());
                for (index, usage) in observed.usages.iter().enumerate() {
                    println!("LIVE_USAGE completion={index} input={:?} output={:?} cached={:?} reasoning={:?}", usage.input_tokens, usage.output_tokens, usage.cached_input_tokens, usage.reasoning_output_tokens);
                }
                result
            }));
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--ignored",
            "--exact",
            "native_live_acceptance",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("RUST_LOG", "off")
        .env("TERM", "xterm-256color");
    let mut terminal = terminal::Terminal::spawn(command, 36, 120)?;
    terminal.wait(
        "native turns finished before ordinary replay",
        DEADLINE,
        |terminal| Ok(String::from_utf8_lossy(&terminal.output).contains("LIVE_REPLAY_READY")),
    )?;
    terminal.wait_composer_frame(&["SYNTHETIC_ACCEPTED", "enter send"], DEADLINE)?;
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(DEADLINE)?;
    terminal.assert_restored()?;
    let output = String::from_utf8_lossy(&terminal.output);
    ensure!(
        output.contains("LIVE_RESULT"),
        "live result was not completed"
    );
    println!(
        "LIVE_TERMINAL thinking_label_wire_observed={} replay_public_reply=true restoration=true child_exit=0",
        output.contains("thinking")
    );
    for line in output.lines() {
        if let Some(start) = [
            "LIVE_MODEL ",
            "LIVE_RESULT ",
            "LIVE_USAGE ",
            "LIVE_ATTEMPT ",
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
