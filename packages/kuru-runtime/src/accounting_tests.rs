use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::{future::Future, pin::Pin};

use anyhow::{Result, bail};
use async_trait::async_trait;
use kuru_connectors::{DemoProvider, Provider, ProviderEvent, ProviderSink};
use kuru_core::{
    Completion, CompletionRequest, Config, ContentBlock, ContextEstimate, ContextTooLarge, Message,
    Mode, ModelInfo, ToolSpec, Usage, UsagePhase, estimated_tokens_for_bytes,
};
use kuru_memory::MemoryStore;
use serde_json::json;
use tokio::sync::Notify;

use crate::{CancellationToken, Harness};

fn config() -> Config {
    Config {
        mode: Mode::Freudian,
        provider: "demo".into(),
        model: "demo".into(),
        max_rounds: 1,
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

#[derive(Default)]
struct MeteredProvider {
    requests: AtomicUsize,
    fail_deliberation: bool,
}

#[async_trait]
impl Provider for MeteredProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        assert!(
            request.context_budget.is_some(),
            "runtime request bypassed fit policy"
        );
        sink.emit(ProviderEvent::ContextMeasured(
            ContextEstimate::for_final_body(
                request.context_budget.clone().unwrap(),
                100,
                false,
                vec![],
            ),
        ))
        .await?;
        self.requests.fetch_add(1, Ordering::SeqCst);
        let deliberate = request.instructions.contains("Phase: deliberate");
        sink.emit(ProviderEvent::Usage(Usage {
            input_tokens: Some(9),
            output_tokens: Some(2),
            cached_input_tokens: None,
            reasoning_output_tokens: None,
        }))
        .await?;
        if deliberate && self.fail_deliberation {
            bail!("scripted provider failure after observed usage");
        }
        sink.emit(ProviderEvent::Completed(Completion {
            blocks: vec![ContentBlock::Text {
                text: "answer".into(),
            }],
            usage: Usage {
                input_tokens: Some(10),
                output_tokens: Some(3),
                cached_input_tokens: Some(0),
                reasoning_output_tokens: None,
            },
            stop_reason: None,
        }))
        .await
    }
}

#[tokio::test]
async fn real_dolt_accounts_deliberation_and_speaking_once_across_exact_retry() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(MeteredProvider::default());
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let session = harness.session.id.clone();
    let output = harness
        .run_controlled(
            "hello",
            Some(&target),
            "accounted-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(output.text, "answer");
    let first = harness.session_usage().await.unwrap();
    assert!(first.historical_complete);
    assert_eq!(first.invocation_count, 2);
    assert_eq!(first.known_usage.input_tokens, Some(20));
    assert_eq!(first.known_usage.output_tokens, Some(6));
    assert_eq!(first.known_usage.cached_input_tokens, Some(0));
    assert!(!first.component_complete.reasoning_output_tokens);
    assert_eq!(provider.requests.load(Ordering::SeqCst), 2);
    let retried = harness
        .run_controlled(
            "hello",
            Some(&target),
            "accounted-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(retried.text, output.text);
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 2);
    assert_eq!(provider.requests.load(Ordering::SeqCst), 2);
    harness.shutdown(false).await.unwrap();
    let ledger = memory.usage_ledger().unwrap();
    assert_eq!(
        ledger
            .session(&session)
            .await
            .unwrap()
            .known_usage
            .input_tokens,
        Some(20)
    );
    memory.close().await.unwrap();
}

#[tokio::test]
async fn resumed_preledger_session_remains_historically_incomplete_after_new_usage() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut original = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        Arc::new(MeteredProvider::default()),
        None,
    )
    .await
    .unwrap();
    let scope = original.scope.clone();
    let mut old_session = original.session.clone();
    old_session.id = "pre-ledger-session".into();
    memory
        .put(
            &format!("{scope}/session/{}", old_session.id),
            &serde_json::to_value(&old_session).unwrap(),
        )
        .await
        .unwrap();
    original.shutdown(false).await.unwrap();
    let mut resumed = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        Arc::new(MeteredProvider::default()),
        Some(&old_session.id),
    )
    .await
    .unwrap();
    let before = resumed.session_usage().await.unwrap();
    assert!(!before.historical_complete);
    assert_eq!(before.invocation_count, 0);
    assert_eq!(before.known_usage.input_tokens, None);
    let target = resumed.topology.parts[0].id.clone();
    resumed
        .run_controlled(
            "new usage",
            Some(&target),
            "pre-ledger-new-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let after = resumed.session_usage().await.unwrap();
    assert!(!after.historical_complete);
    assert_eq!(after.invocation_count, 2);
    assert_eq!(after.known_usage.input_tokens, Some(20));
    resumed.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn provider_failure_keeps_partial_observation_and_unknown_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(MeteredProvider {
        fail_deliberation: true,
        ..MeteredProvider::default()
    });
    let mut harness = Harness::new(config(), directory.path(), memory.clone(), provider, None)
        .await
        .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let _ = harness
        .run_controlled(
            "hello",
            Some(&target),
            "partial-turn",
            &CancellationToken::new(),
        )
        .await;
    let usage = harness.session_usage().await.unwrap();
    assert_eq!(usage.invocation_count, 1);
    assert_eq!(usage.incomplete_invocations, 1);
    assert_eq!(usage.known_usage.input_tokens, Some(9));
    assert_eq!(usage.known_usage.output_tokens, Some(2));
    assert_eq!(usage.known_usage.cached_input_tokens, None);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct FailBeforeUsage;

#[async_trait]
impl Provider for FailBeforeUsage {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
        _sink: &mut dyn ProviderSink,
    ) -> Result<()> {
        bail!("scripted provider failure before usage")
    }
}

#[tokio::test]
async fn admitted_without_observation_remains_incomplete_and_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        Arc::new(FailBeforeUsage),
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    assert!(
        harness
            .ask(
                &actor,
                vec![Message::text("user", "fail before usage")],
                "speak and act: fail before usage",
                vec![],
            )
            .await
            .is_err()
    );
    let usage = harness.session_usage().await.unwrap();
    assert_eq!(usage.invocation_count, 1);
    assert_eq!(usage.incomplete_invocations, 1);
    assert_eq!(usage.known_usage.input_tokens, None);
    assert_eq!(usage.known_usage.output_tokens, None);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct MissingTerminalProvider;

#[async_trait]
impl Provider for MissingTerminalProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::Usage(Usage {
            input_tokens: Some(11),
            cached_input_tokens: Some(0),
            ..Usage::default()
        }))
        .await?;
        sink.emit(ProviderEvent::Completed(Completion {
            blocks: vec![ContentBlock::Text {
                text: "terminal".into(),
            }],
            usage: Usage {
                output_tokens: Some(4),
                ..Usage::default()
            },
            stop_reason: None,
        }))
        .await
    }
}

#[tokio::test]
async fn terminal_missing_component_retains_progress_without_fabricating_zero() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        Arc::new(MissingTerminalProvider),
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    harness
        .ask(
            &actor,
            vec![Message::text("user", "report")],
            "speak and act: report",
            vec![],
        )
        .await
        .unwrap();
    let usage = harness.session_usage().await.unwrap();
    assert_eq!(usage.invocation_count, 1);
    assert_eq!(usage.known_usage.input_tokens, Some(11));
    assert_eq!(usage.known_usage.output_tokens, Some(4));
    assert_eq!(usage.known_usage.cached_input_tokens, Some(0));
    assert_eq!(usage.known_usage.reasoning_output_tokens, None);
    assert_eq!(usage.incomplete_invocations, 1);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct FitProvider {
    attempts: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for FitProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        self.attempts.lock().unwrap().push(request.clone());
        let budget = request.context_budget.clone().expect("runtime fit budget");
        let measured = ContextEstimate::for_final_body(budget.clone(), 100, false, vec![]);
        sink.emit(ProviderEvent::ContextMeasured(measured)).await?;
        if request.messages.len() > 2 {
            return Err(ContextTooLarge {
                estimated_input_tokens: budget.window.value,
                output_reserve_tokens: budget.output_reserve_tokens,
                window_tokens: budget.window.value,
                native_continuation_mandatory: false,
            }
            .into());
        }
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "fit",
            vec![],
            1,
            1,
        )))
        .await
    }
}

#[tokio::test]
async fn fit_retries_same_invocation_with_whole_optional_rows_and_exact_current_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(FitProvider {
        attempts: Mutex::new(vec![]),
    });
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    let namespace = harness.namespace(&actor);
    for index in 0..5 {
        memory
            .append_message(
                &namespace,
                &Message::text("user", format!("old-{index}-🪶")),
            )
            .await
            .unwrap();
    }
    let receipt = Message::tool_result(
        "required-call",
        json!({"result":"must remain complete"}),
        false,
    );
    let completion = harness
        .ask(&actor, vec![receipt.clone()], "speak and act: fit", vec![])
        .await
        .unwrap();
    assert_eq!(completion.text_projection(), "fit");
    {
        let attempts = provider.attempts.lock().unwrap();
        assert_eq!(attempts.len(), 5);
        assert!(
            attempts
                .iter()
                .all(|request| request.current_message_count == Some(1))
        );
        assert!(
            attempts
                .iter()
                .all(|request| request.messages.last() == Some(&receipt))
        );
        assert_eq!(attempts.last().unwrap().messages.len(), 2);
    }
    let context = harness.subscribe_context().borrow().latest.clone().unwrap();
    assert_eq!(context.phase, UsagePhase::Speak);
    assert_eq!(context.omitted_private_rows, 4);
    assert_eq!(
        memory
            .history_window(&namespace, 100)
            .await
            .unwrap()
            .total_rows,
        7
    );
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 1);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct InstructionFitProvider {
    attempts: Mutex<Vec<String>>,
}

#[async_trait]
impl Provider for InstructionFitProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        self.attempts
            .lock()
            .unwrap()
            .push(request.instructions.clone());
        let budget = request.context_budget.clone().unwrap();
        sink.emit(ProviderEvent::ContextMeasured(
            ContextEstimate::for_final_body(budget.clone(), 100, false, vec![]),
        ))
        .await?;
        if request.instructions.contains("PUBLIC-OLD") || request.instructions.contains("NOTE-OLD")
        {
            return Err(ContextTooLarge {
                estimated_input_tokens: budget.window.value,
                output_reserve_tokens: budget.output_reserve_tokens,
                window_tokens: budget.window.value,
                native_continuation_mandatory: false,
            }
            .into());
        }
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "fit",
            vec![],
            1,
            1,
        )))
        .await
    }
}

#[tokio::test]
async fn fit_omits_public_and_note_rows_whole_without_deleting_them() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(InstructionFitProvider {
        attempts: Mutex::new(vec![]),
    });
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    let transcript = format!("{}/transcript/{}", harness.scope, harness.session.id);
    let notes = format!("{}/notes", harness.namespace(&actor));
    memory
        .append_message(&transcript, &Message::text("user", "PUBLIC-OLD 🪶"))
        .await
        .unwrap();
    memory
        .append_message(
            &transcript,
            &Message::text(crate::INTERRUPTION_ROLE, crate::INTERRUPTION_TEXT),
        )
        .await
        .unwrap();
    memory.append(&notes, "note", "NOTE-OLD 🪶").await.unwrap();
    harness
        .ask(
            &actor,
            vec![Message::text("user", "current")],
            "speak and act: fit",
            vec![],
        )
        .await
        .unwrap();
    let attempts = provider.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 3);
    assert!(attempts[0].contains("PUBLIC-OLD") && attempts[0].contains("NOTE-OLD"));
    assert!(!attempts[0].contains(crate::INTERRUPTION_TEXT));
    assert!(!attempts[1].contains("PUBLIC-OLD") && attempts[1].contains("NOTE-OLD"));
    assert!(!attempts[2].contains("PUBLIC-OLD") && !attempts[2].contains("NOTE-OLD"));
    let context = harness.subscribe_context().borrow().latest.clone().unwrap();
    assert_eq!(context.omitted_public_rows, 1);
    assert_eq!(context.omitted_note_rows, 1);
    assert!(context.runtime_sources.iter().any(|source| source.kind
        == kuru_core::ContextSourceKind::PublicTranscript
        && source.units == 0));
    assert!(
        context
            .runtime_sources
            .iter()
            .any(|source| source.kind == kuru_core::ContextSourceKind::Notes && source.units == 0)
    );
    assert_eq!(
        memory
            .history_window(&transcript, 16)
            .await
            .unwrap()
            .total_rows,
        2
    );
    assert_eq!(
        memory.history_window(&notes, 16).await.unwrap().total_rows,
        1
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct DemoAttempt {
    request: CompletionRequest,
    measured: ContextEstimate,
    accepted: bool,
}

#[derive(Default)]
struct RecordingDemo {
    attempts: Mutex<Vec<DemoAttempt>>,
}

struct MeasurementSink<'a> {
    downstream: &'a mut dyn ProviderSink,
    measured: &'a mut Option<ContextEstimate>,
}

impl ProviderSink for MeasurementSink<'_> {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let ProviderEvent::ContextMeasured(context) = &event {
                *self.measured = Some(context.clone());
            }
            self.downstream.emit(event).await
        })
    }
}

#[async_trait]
impl Provider for RecordingDemo {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        DemoProvider.models().await
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let mut measured = None;
        let result = DemoProvider
            .stream(
                request.clone(),
                &mut MeasurementSink {
                    downstream: sink,
                    measured: &mut measured,
                },
            )
            .await;
        self.attempts.lock().unwrap().push(DemoAttempt {
            request,
            measured: measured.expect("Demo emitted its actual prompt measurement"),
            accepted: result.is_ok(),
        });
        result
    }
}

#[tokio::test]
async fn demo_effective_prompt_retries_whole_multilingual_rows_under_one_admission() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(RecordingDemo::default());
    let mut settings = config();
    settings.assumed_context_window_tokens = Some(3_500);
    settings.context_output_reserve_tokens = Some(200);
    let mut harness = Harness::new(
        settings,
        directory.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    let private_key = harness.namespace(&actor);
    let transcript_key = format!("{}/transcript/{}", harness.scope, harness.session.id);
    let notes_key = format!("{private_key}/notes");
    let older_private = (0..10)
        .map(|index| format!("older-private-{index}-{}", "🪶漢字".repeat(180)))
        .collect::<Vec<_>>();
    for row in &older_private {
        memory
            .append_message(&private_key, &Message::text("user", row))
            .await
            .unwrap();
    }
    let public = format!("PUBLIC-{}", "🌊".repeat(80));
    let note = format!("NOTE-{}", "é".repeat(120));
    memory
        .append_message(&transcript_key, &Message::text("user", &public))
        .await
        .unwrap();
    memory.append(&notes_key, "note", &note).await.unwrap();
    let required = vec![
        Message::text("user", "current user 🦉"),
        Message::tool_result("required-tool", json!({"value":"current exact 🦉"}), false),
    ];
    let tools = vec![ToolSpec {
        name: "required_tool".into(),
        description: "Tool schema remains available during fit".into(),
        parameters: json!({"type":"object","properties":{"value":{"type":"string"}}}),
    }];
    harness
        .ask(
            &actor,
            required.clone(),
            "speak and act: real fit",
            tools.clone(),
        )
        .await
        .unwrap();

    {
        let attempts = provider.attempts.lock().unwrap();
        assert!(
            attempts.len() > 1,
            "optional history must actually overflow"
        );
        assert!(
            attempts[..attempts.len() - 1]
                .iter()
                .all(|attempt| !attempt.accepted)
        );
        assert!(attempts.last().unwrap().accepted);
        for attempt in attempts.iter() {
            let body = json!({
                "instructions": attempt.request.instructions,
                "messages": attempt.request.messages,
                "tools": attempt.request.tools,
            });
            let bytes = serde_json::to_vec(&body).unwrap().len() as u64;
            assert_eq!(attempt.measured.final_body_bytes, bytes);
            assert_eq!(
                attempt.measured.estimated_input_tokens,
                estimated_tokens_for_bytes(bytes)
            );
            assert_eq!(attempt.measured.budget.window.value, 3_500);
            assert_eq!(attempt.measured.budget.output_reserve_tokens, 200);
            assert_eq!(attempt.measured.ensure_fits().is_ok(), attempt.accepted);
            assert_eq!(attempt.request.current_message_count, Some(required.len()));
            assert_eq!(
                attempt.request.messages[attempt.request.messages.len() - required.len()..],
                required
            );
            assert_eq!(attempt.request.tools, tools);
        }
        let final_request = &attempts.last().unwrap().request;
        let retained_private =
            &final_request.messages[..final_request.messages.len() - required.len()];
        let omitted_private = older_private.len() - retained_private.len();
        assert!(omitted_private > 0);
        for (row, expected) in retained_private
            .iter()
            .zip(older_private[omitted_private..].iter())
        {
            assert_eq!(row.plain_text(), Some(expected.as_str()));
        }
        let public_retained = final_request.instructions.contains(&public);
        let note_retained = final_request.instructions.contains(&note);
        let context = harness.subscribe_context().borrow().latest.clone().unwrap();
        assert_eq!(context.omitted_private_rows, omitted_private as u64);
        assert_eq!(context.omitted_public_rows, u64::from(!public_retained));
        assert_eq!(context.omitted_note_rows, u64::from(!note_retained));
        for (kind, units) in [
            (
                kuru_core::ContextSourceKind::PrivateHistory,
                retained_private.len() as u64,
            ),
            (
                kuru_core::ContextSourceKind::PublicTranscript,
                u64::from(public_retained),
            ),
            (
                kuru_core::ContextSourceKind::Notes,
                u64::from(note_retained),
            ),
            (kuru_core::ContextSourceKind::CurrentInput, 1),
            (kuru_core::ContextSourceKind::RequiredReceipts, 1),
            (kuru_core::ContextSourceKind::ToolSchemas, 1),
        ] {
            assert_eq!(
                context
                    .runtime_sources
                    .iter()
                    .find(|source| source.kind == kind)
                    .unwrap()
                    .units,
                units,
                "source {kind:?} did not match the sent request"
            );
        }
    }
    assert_eq!(
        memory
            .history_window(&private_key, 100)
            .await
            .unwrap()
            .total_rows,
        13
    );
    assert_eq!(
        memory
            .history_window(&transcript_key, 100)
            .await
            .unwrap()
            .total_rows,
        1
    );
    assert_eq!(
        memory
            .history_window(&notes_key, 100)
            .await
            .unwrap()
            .total_rows,
        1
    );
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 1);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct PhaseProvider {
    recipient: String,
    speaking: AtomicUsize,
    phases: Mutex<Vec<UsagePhase>>,
}

#[async_trait]
impl Provider for PhaseProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let phase = if request.instructions.contains("Phase: dream:") {
            UsagePhase::Dream
        } else if request.instructions.contains("Phase: peer consultation:") {
            UsagePhase::Consult
        } else if request.instructions.contains("Phase: speak and act:") {
            UsagePhase::Speak
        } else {
            UsagePhase::Deliberate
        };
        self.phases.lock().unwrap().push(phase);
        let calls =
            if phase == UsagePhase::Speak && self.speaking.fetch_add(1, Ordering::SeqCst) == 0 {
                vec![kuru_core::ToolCall {
                    id: "consult-one".into(),
                    name: "peer_send".into(),
                    arguments: json!({"to":self.recipient,"message":"consult briefly"}),
                }]
            } else {
                vec![]
            };
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "phase answer",
            calls,
            1,
            1,
        )))
        .await
    }
}

#[tokio::test]
async fn consultation_and_dream_each_admit_separate_provider_invocations() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut initial = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        Arc::new(MeteredProvider::default()),
        None,
    )
    .await
    .unwrap();
    let target = initial.topology.parts[0].id.clone();
    let recipient = initial.topology.parts[1].id.clone();
    let session_id = initial.session.id.clone();
    initial.shutdown(false).await.unwrap();
    let provider = Arc::new(PhaseProvider {
        recipient,
        speaking: AtomicUsize::new(0),
        phases: Mutex::new(vec![]),
    });
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        provider.clone(),
        Some(&session_id),
    )
    .await
    .unwrap();
    let active = harness
        .topology
        .parts
        .iter()
        .filter(|part| part.active)
        .count() as u64;
    harness
        .run_controlled(
            "consult",
            Some(&target),
            "phase-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 4);
    harness.dream().await.unwrap();
    assert_eq!(
        harness.session_usage().await.unwrap().invocation_count,
        4 + active
    );
    let phases = provider.phases.lock().unwrap().clone();
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == UsagePhase::Deliberate)
            .count(),
        1
    );
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == UsagePhase::Speak)
            .count(),
        2
    );
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == UsagePhase::Consult)
            .count(),
        1
    );
    assert_eq!(
        phases
            .iter()
            .filter(|phase| **phase == UsagePhase::Dream)
            .count(),
        active as usize
    );
    drop(phases);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn periodic_and_shutdown_dreams_use_distinct_attempts() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(MeteredProvider::default());
    let mut config = config();
    config.dream_every = 1;
    config.dream_on_exit = true;
    let mut harness = Harness::new(config, directory.path(), memory.clone(), provider, None)
        .await
        .unwrap();
    let active = harness
        .topology
        .parts
        .iter()
        .filter(|part| part.active)
        .count() as u64;
    let target = harness.topology.parts[0].id.clone();
    harness
        .run_controlled(
            "dream after turn",
            Some(&target),
            "periodic-dream-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        harness.session_usage().await.unwrap().invocation_count,
        2 + active
    );
    let context = harness.subscribe_context().borrow().clone();
    assert_eq!(context.latest.as_ref().unwrap().phase, UsagePhase::Dream);
    assert_eq!(
        context.latest_facing.as_ref().unwrap().phase,
        UsagePhase::Speak
    );
    assert_eq!(
        context.latest_facing.as_ref().unwrap().operation_id,
        "periodic-dream-turn"
    );
    harness.shutdown(true).await.unwrap();
    assert_eq!(
        harness.session_usage().await.unwrap().invocation_count,
        2 + 2 * active
    );
    let context = harness.subscribe_context().borrow().clone();
    assert_eq!(context.latest.as_ref().unwrap().phase, UsagePhase::Dream);
    assert!(context.latest_facing.is_none());
    memory.close().await.unwrap();
}

struct CancelAfterUsage {
    observed: Arc<Notify>,
}

#[async_trait]
impl Provider for CancelAfterUsage {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::Usage(Usage {
            input_tokens: Some(17),
            ..Usage::default()
        }))
        .await?;
        self.observed.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancellation_settles_admitted_usage_without_a_terminal_report() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let observed = Arc::new(Notify::new());
    let provider = Arc::new(CancelAfterUsage {
        observed: observed.clone(),
    });
    let mut harness = Harness::new(config(), directory.path(), memory.clone(), provider, None)
        .await
        .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let session = harness.session.id.clone();
    let cancellation = CancellationToken::new();
    let control = cancellation.clone();
    let running = tokio::spawn(async move {
        let result = harness
            .run_controlled("cancel me", Some(&target), "cancelled-turn", &control)
            .await;
        harness.shutdown(false).await.unwrap();
        result
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), observed.notified())
        .await
        .unwrap();
    cancellation.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), running)
        .await
        .unwrap()
        .unwrap();
    let usage = memory
        .usage_ledger()
        .unwrap()
        .session(&session)
        .await
        .unwrap();
    assert_eq!(usage.invocation_count, 1);
    assert_eq!(usage.incomplete_invocations, 1);
    assert_eq!(usage.known_usage.input_tokens, Some(17));
    assert_eq!(usage.known_usage.output_tokens, None);
    memory.close().await.unwrap();
}

#[tokio::test]
async fn abandoned_dream_keeps_usage_after_reopen_without_advancing_main() {
    let directory = tempfile::tempdir().unwrap();
    let scope = crate::project_scope(directory.path()).unwrap();
    let options =
        kuru_memory::test_support::open_options(directory.path().join("memory"), scope).unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let observed = Arc::new(Notify::new());
    let provider = Arc::new(CancelAfterUsage {
        observed: observed.clone(),
    });
    let mut harness = Harness::new(config(), directory.path(), memory.clone(), provider, None)
        .await
        .unwrap();
    let session = harness.session.id.clone();
    let main_before = memory.revision().await.unwrap();
    let cancellation = CancellationToken::new();
    let control = cancellation.clone();
    let running = tokio::spawn(async move {
        let result = harness.dream_controlled(&control).await;
        harness.shutdown(false).await.unwrap();
        result
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), observed.notified())
        .await
        .unwrap();
    cancellation.cancel();
    let error = tokio::time::timeout(std::time::Duration::from_secs(10), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(crate::turn_was_cancelled(&error));
    assert_eq!(memory.revision().await.unwrap(), main_before);
    let before = memory
        .usage_ledger()
        .unwrap()
        .session(&session)
        .await
        .unwrap();
    assert!(before.invocation_count >= 1);
    let known_input = before.known_usage.input_tokens.unwrap();
    assert!(known_input >= 17 && known_input <= 17 * before.invocation_count);
    assert_eq!(before.incomplete_invocations, before.invocation_count);
    memory.close().await.unwrap();
    let reopened = MemoryStore::open(options).await.unwrap();
    let after = reopened
        .usage_ledger()
        .unwrap()
        .session(&session)
        .await
        .unwrap();
    assert_eq!(after, before);
    reopened.close().await.unwrap();
}
