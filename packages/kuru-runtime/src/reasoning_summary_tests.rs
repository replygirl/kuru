use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use async_trait::async_trait;
use kuru_connectors::{Provider, ProviderEvent, ProviderReasoningSummary, ProviderSink};
use kuru_core::{
    Completion, CompletionRequest, Config, InvocationOutcome, InvocationStart, Mode, ModelInfo,
    UsagePhase,
};
use kuru_memory::{MemoryStore, StorageRecord};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use crate::{CancellationToken, Harness, turn_was_cancelled};

struct SettledSummaryProvider;

#[async_trait]
impl Provider for SettledSummaryProvider {
    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::SettledReasoningSummaries(vec![
            ProviderReasoningSummary {
                item_id: Some("provider-item".into()),
                output_index: Some(4),
                summary_index: 1,
                text: "private settled summary".into(),
            },
        ]))
        .await?;
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "public answer",
            vec![],
            3,
            2,
        )))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        Ok(Completion::from_legacy("public answer", vec![], 3, 2))
    }
}

struct FailingAfterSummaryProvider;

#[async_trait]
impl Provider for FailingAfterSummaryProvider {
    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::SettledReasoningSummaries(vec![
            ProviderReasoningSummary {
                item_id: None,
                output_index: None,
                summary_index: 0,
                text: "must not persist".into(),
            },
        ]))
        .await?;
        bail!("provider failed after a private sidecar")
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        bail!("provider failed after a private sidecar")
    }
}

struct SummaryFreeProvider;

#[async_trait]
impl Provider for SummaryFreeProvider {
    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "public answer without a summary",
            vec![],
            3,
            2,
        )))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        Ok(Completion::from_legacy(
            "public answer without a summary",
            vec![],
            3,
            2,
        ))
    }
}

#[derive(Default)]
struct RequestRecordingProvider {
    requests: Mutex<Vec<CompletionRequest>>,
}

impl RequestRecordingProvider {
    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for RequestRecordingProvider {
    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        self.requests.lock().unwrap().push(request);
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "later public answer",
            vec![],
            0,
            0,
        )))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        self.requests.lock().unwrap().push(request);
        Ok(Completion::from_legacy("later public answer", vec![], 0, 0))
    }
}

/// Sends a terminal event that the accounting observer accepts, then violates
/// the provider stream contract. The sidecar must still remain buffered: a
/// collector rejection after `Completed` means there is no accepted completion
/// on which private persistence may depend.
struct RejectedTerminalAfterSummaryProvider;

#[async_trait]
impl Provider for RejectedTerminalAfterSummaryProvider {
    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::SettledReasoningSummaries(vec![
            ProviderReasoningSummary {
                item_id: Some("rejected-terminal-item".into()),
                output_index: Some(0),
                summary_index: 0,
                text: "must not persist after terminal rejection".into(),
            },
        ]))
        .await?;
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "public answer",
            vec![],
            0,
            0,
        )))
        .await?;
        sink.emit(ProviderEvent::TextDelta {
            item_id: "late-output".into(),
            output_index: 0,
            content_index: 0,
            source: kuru_connectors::TextDeltaSource::OutputText,
            text: "invalid late event".into(),
        })
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        Ok(Completion::from_legacy("public answer", vec![], 0, 0))
    }
}

struct BlockingAfterSummaryProvider {
    emitted_sidecar: Notify,
}

#[async_trait]
impl Provider for BlockingAfterSummaryProvider {
    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::SettledReasoningSummaries(vec![
            ProviderReasoningSummary {
                item_id: Some("cancelled-item".into()),
                output_index: Some(0),
                summary_index: 0,
                text: "must not survive cancellation".into(),
            },
        ]))
        .await?;
        self.emitted_sidecar.notify_waiters();
        std::future::pending().await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        std::future::pending().await
    }
}

async fn harness(memory: MemoryStore, provider: Arc<dyn Provider>) -> (tempfile::TempDir, Harness) {
    let project = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        Config {
            mode: Mode::Ifs,
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        memory,
        provider,
        None,
    )
    .await
    .unwrap();
    (project, harness)
}

async fn exported_private_records(memory: &MemoryStore) -> Vec<(String, serde_json::Value)> {
    let export = memory.begin_active_export().await.unwrap();
    let messages = export.page(None).await.unwrap();
    let state = export.page(messages.next).await.unwrap();
    state
        .records
        .into_iter()
        .filter_map(|record| match record {
            StorageRecord::State { key, value }
                if key.starts_with("kuru/private/reasoning-summary/v1/") =>
            {
                Some((key, value))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn settled_summary_persists_with_the_admitted_turn_and_stays_out_of_transcript() {
    let memory = MemoryStore::temporary().await.unwrap();
    let (_project, mut harness) = harness(memory.clone(), Arc::new(SettledSummaryProvider)).await;
    let output = harness
        .run_controlled(
            "retain the provider summary privately",
            None,
            "reasoning-summary-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(output.text, "public answer");
    assert!(
        harness
            .history()
            .await
            .unwrap()
            .iter()
            .all(|message| message.plain_text() != Some("private settled summary"))
    );
    let records = exported_private_records(&memory).await;
    assert_eq!(records.len(), 1);
    let value = &records[0].1;
    assert_eq!(value["session_id"], harness.session.id);
    assert_eq!(value["turn_id"], "reasoning-summary-turn");
    assert_eq!(value["actor_id"], output.speaker);
    assert!(
        value["invocation_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("v1-") && id.len() > 3)
    );
    assert_eq!(value["item_id"], "provider-item");
    assert_eq!(value["output_index"], 4);
    assert_eq!(value["summary_index"], 1);
    assert_eq!(value["text"], "private settled summary");
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn settled_summary_survives_a_real_memory_reopen_without_entering_history() {
    let project = tempfile::tempdir().unwrap();
    let data = kuru_memory::test_support::tempdir().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().into(),
        crate::project_scope(project.path()).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
        Config {
            mode: Mode::Freudian,
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        memory.clone(),
        Arc::new(SettledSummaryProvider),
        None,
    )
    .await
    .unwrap();

    harness
        .run_controlled(
            "retain a summary across reopen",
            None,
            "reopened-summary-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(
        harness
            .history()
            .await
            .unwrap()
            .iter()
            .all(|message| message.plain_text() != Some("private settled summary"))
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    drop(harness);

    let reopened = MemoryStore::open(options).await.unwrap();
    let records = exported_private_records(&reopened).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1["turn_id"], "reopened-summary-turn");
    assert_eq!(records[0].1["text"], "private settled summary");
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn private_summary_is_not_injected_into_a_new_sessions_provider_context() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut first = Harness::new(
        Config {
            mode: Mode::Ifs,
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        memory.clone(),
        Arc::new(SettledSummaryProvider),
        None,
    )
    .await
    .unwrap();
    first
        .run_controlled(
            "retain a private summary for the first session",
            None,
            "first-private-summary-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let first_session = first.session.id.clone();
    first.shutdown(false).await.unwrap();
    drop(first);

    let provider = Arc::new(RequestRecordingProvider::default());
    let mut second = Harness::new(
        Config {
            mode: Mode::Ifs,
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    assert_ne!(second.session.id, first_session);
    second
        .run_controlled(
            "start an independent second session",
            None,
            "second-private-summary-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let requests = provider.requests();
    assert!(!requests.is_empty());
    assert!(requests.iter().all(|request| {
        !request.instructions.contains("private settled summary")
            && request.messages.iter().all(|message| {
                !message
                    .text_projection()
                    .contains("private settled summary")
            })
    }));
    second.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn failed_completion_after_sidecar_does_not_persist_a_private_summary() {
    let memory = MemoryStore::temporary().await.unwrap();
    let (_project, mut harness) =
        harness(memory.clone(), Arc::new(FailingAfterSummaryProvider)).await;
    assert!(
        harness
            .run_controlled(
                "fail after sidecar",
                None,
                "failed-summary-turn",
                &CancellationToken::new(),
            )
            .await
            .is_err()
    );
    assert!(exported_private_records(&memory).await.is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn summary_free_completion_does_not_persist_a_private_summary() {
    let memory = MemoryStore::temporary().await.unwrap();
    let (_project, mut harness) = harness(memory.clone(), Arc::new(SummaryFreeProvider)).await;
    let output = harness
        .run_controlled(
            "complete without a provider summary",
            None,
            "summary-free-turn",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(output.text, "public answer without a summary");
    assert!(exported_private_records(&memory).await.is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn failed_ledger_settlement_after_sidecar_does_not_persist_a_private_summary() {
    let memory = MemoryStore::temporary().await.unwrap();
    let (_project, mut harness) = harness(memory.clone(), Arc::new(SettledSummaryProvider)).await;
    let actor_id = harness.topology.parts[0].id.clone();
    let turn_id = "ledger-failure-summary-turn";
    // `run_controlled` uses its durable turn-journal ID as the operation ID.
    // With the default one deliberation round, the selected speaking request is
    // the second admitted provider invocation.
    let invocation_id = speaking_invocation_id(&harness.session.id, turn_id, &actor_id, 1);
    let invocation = InvocationStart {
        session_id: harness.session.id.clone(),
        invocation_id: invocation_id.clone(),
        operation_id: turn_id.into(),
        phase: UsagePhase::Speak,
        actor_id: actor_id.clone(),
        route: "demo".into(),
        model: "demo".into(),
        price_at_invocation: None,
    };
    let ledger = memory.usage_ledger().unwrap();
    ledger.admit(invocation).await.unwrap();
    ledger
        .settle(&invocation_id, InvocationOutcome::Failed)
        .await
        .unwrap();

    assert!(
        harness
            .run_controlled(
                "fail accounting after a provider sidecar",
                Some(&actor_id),
                turn_id,
                &CancellationToken::new(),
            )
            .await
            .is_err(),
        "a conflicting settled ledger outcome must fail the controlled turn"
    );
    assert!(exported_private_records(&memory).await.is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

fn speaking_invocation_id(
    session_id: &str,
    operation_id: &str,
    actor_id: &str,
    ordinal: u64,
) -> String {
    let mut digest = Sha256::new();
    for component in ["kuru-invocation-v1", session_id, operation_id, actor_id] {
        digest.update((component.len() as u64).to_be_bytes());
        digest.update(component.as_bytes());
    }
    digest.update([2]);
    digest.update(ordinal.to_be_bytes());
    format!(
        "v1-{}",
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[tokio::test]
async fn rejected_terminal_after_sidecar_does_not_persist_a_private_summary() {
    let memory = MemoryStore::temporary().await.unwrap();
    let (_project, mut harness) = harness(
        memory.clone(),
        Arc::new(RejectedTerminalAfterSummaryProvider),
    )
    .await;
    assert!(
        harness
            .run_controlled(
                "reject terminal after sidecar",
                None,
                "rejected-terminal-summary-turn",
                &CancellationToken::new(),
            )
            .await
            .is_err(),
        "a collector-rejected terminal event must fail the controlled turn"
    );
    assert!(exported_private_records(&memory).await.is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_after_sidecar_does_not_persist_a_private_summary() {
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(BlockingAfterSummaryProvider {
        emitted_sidecar: Notify::new(),
    });
    let (_project, harness) = harness(memory.clone(), provider.clone()).await;
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let emitted = provider.emitted_sidecar.notified();
    tokio::pin!(emitted);
    let task = tokio::spawn(async move {
        let mut harness = harness;
        let result = harness
            .run_controlled(
                "cancel after sidecar",
                None,
                "cancelled-summary-turn",
                &cancel,
            )
            .await;
        (harness, result)
    });
    emitted.await;
    cancellation.cancel();
    let (mut harness, result) = task.await.unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    assert!(exported_private_records(&memory).await.is_empty());
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}
