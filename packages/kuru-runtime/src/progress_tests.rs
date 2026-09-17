use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{Provider, ProviderEvent, ProviderSink, TextDeltaSource};
use kuru_core::{
    Completion, CompletionRequest, Config, Mode, ModelInfo, RelationshipKind, ToolCall,
};
use kuru_memory::MemoryStore;
use serde_json::json;
use tokio::sync::{Semaphore, watch};

use crate::{CancellationToken, Event, FacingProgress, Harness};

struct StagedProvider {
    reached: watch::Sender<usize>,
    release: Semaphore,
    requests: Mutex<Vec<CompletionRequest>>,
    speaking: AtomicUsize,
    tool_loop: bool,
    consult_target: Mutex<Option<String>>,
}

impl StagedProvider {
    fn new(tool_loop: bool) -> (Arc<Self>, watch::Receiver<usize>) {
        let (reached, receiver) = watch::channel(0);
        (
            Arc::new(Self {
                reached,
                release: Semaphore::new(0),
                requests: Mutex::new(Vec::new()),
                speaking: AtomicUsize::new(0),
                tool_loop,
                consult_target: Mutex::new(None),
            }),
            receiver,
        )
    }

    async fn wait_at(&self, stage: usize) -> Result<()> {
        self.reached.send_replace(stage);
        self.release.acquire().await?.forget();
        Ok(())
    }

    fn release(&self) {
        self.release.add_permits(1);
    }

    fn consult(&self, target: String) {
        *self.consult_target.lock().unwrap() = Some(target);
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl Provider for StagedProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let phase = request.instructions.clone();
        self.requests.lock().unwrap().push(request);
        if phase.contains("Phase: deliberate") {
            sink.emit(ProviderEvent::TextDelta {
                item_id: "private-draft".into(),
                output_index: 0,
                content_index: 0,
                source: TextDeltaSource::OutputText,
                text: "PRIVATE-DELIBERATION".into(),
            })
            .await?;
            self.wait_at(1).await?;
            return sink
                .emit(ProviderEvent::Completed(Completion::from_legacy(
                    "private draft",
                    vec![],
                    1,
                    1,
                )))
                .await;
        }
        if phase.contains("Phase: peer consultation") {
            sink.emit(ProviderEvent::TextDelta {
                item_id: "private-consultation".into(),
                output_index: 0,
                content_index: 0,
                source: TextDeltaSource::OutputText,
                text: "PRIVATE-CONSULTATION".into(),
            })
            .await?;
            self.wait_at(3).await?;
            return sink
                .emit(ProviderEvent::Completed(Completion::from_legacy(
                    "private advice",
                    vec![],
                    1,
                    1,
                )))
                .await;
        }
        if phase.contains("Phase: dream:") {
            sink.emit(ProviderEvent::TextDelta {
                item_id: "private-dream".into(),
                output_index: 0,
                content_index: 0,
                source: TextDeltaSource::OutputText,
                text: "PRIVATE-DREAM".into(),
            })
            .await?;
            self.wait_at(5).await?;
            return sink
                .emit(ProviderEvent::Completed(Completion::from_legacy(
                    "private dream note",
                    vec![],
                    1,
                    1,
                )))
                .await;
        }
        assert!(phase.contains("Phase: speak and act:"));
        let round = self.speaking.fetch_add(1, Ordering::SeqCst) + 1;
        sink.emit(ProviderEvent::TextDelta {
            item_id: format!("facing-{round}"),
            output_index: 0,
            content_index: 0,
            source: TextDeltaSource::OutputText,
            text: format!("provisional-{round}"),
        })
        .await?;
        if round == 1 {
            sink.emit(ProviderEvent::ReasoningSummaryDelta {
                item_id: "visible-summary".into(),
                output_index: 1,
                summary_index: 0,
                text: "VISIBLE-SUMMARY".into(),
            })
            .await?;
            if self.tool_loop {
                sink.emit(ProviderEvent::ToolCallDelta {
                    item_id: "tool-item".into(),
                    output_index: 2,
                    arguments_fragment: "{\"activation\":".into(),
                })
                .await?;
            }
        }
        let consult_target = self.consult_target.lock().unwrap().clone();
        self.wait_at(if consult_target.is_some() && round > 1 {
            4
        } else {
            round + 1
        })
        .await?;
        let calls = if self.tool_loop && round == 1 {
            vec![ToolCall {
                id: "state-call".into(),
                name: "state_report".into(),
                arguments: json!({"activation":0.4,"note":"bounded preview"}),
            }]
        } else if let Some(target) = consult_target.filter(|_| round == 1) {
            vec![ToolCall {
                id: "peer-call".into(),
                name: "peer_send".into(),
                arguments: json!({"to":target,"message":"private consultation request"}),
            }]
        } else {
            vec![]
        };
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            if round == 1 {
                "settled-one"
            } else {
                "settled-two"
            },
            calls,
            2,
            3,
        )))
        .await
    }
}

async fn stage(receiver: &mut watch::Receiver<usize>, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while *receiver.borrow_and_update() < expected {
            receiver.changed().await.unwrap();
        }
    })
    .await
    .expect("provider never reached expected stream stage");
}

fn config() -> Config {
    Config {
        mode: Mode::Ifs,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

fn preview(receiver: &watch::Receiver<Option<FacingProgress>>) -> FacingProgress {
    receiver.borrow().clone().expect("missing facing preview")
}

#[tokio::test]
async fn selected_part_and_relationship_only_preview_their_speaking_requests() {
    for relationship in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let memory = MemoryStore::temporary().await.unwrap();
        let (provider, mut stages) = StagedProvider::new(false);
        let mut harness = Harness::new(
            config(),
            directory.path(),
            memory.clone(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let ids = harness
            .topology
            .parts
            .iter()
            .take(2)
            .map(|part| part.id.clone())
            .collect::<Vec<_>>();
        let target = if relationship {
            harness
                .relate(RelationshipKind::Alliance, ids)
                .await
                .unwrap()
                .id
        } else {
            ids[0].clone()
        };
        let mut progress = harness.subscribe_progress();
        let run_target = target.clone();
        let run = tokio::spawn(async move {
            let result = harness
                .run_local_controlled(
                    "preview request",
                    Some(&run_target),
                    "preview-turn",
                    &CancellationToken::new(),
                )
                .await;
            (harness, result)
        });
        stage(&mut stages, 1).await;
        assert!(progress.borrow().is_none(), "private deliberation leaked");
        provider.release();
        stage(&mut stages, 2).await;
        let visible = preview(&progress);
        assert_eq!(visible.turn_id, "preview-turn");
        assert_eq!(visible.request_round, 1);
        assert_eq!(visible.text_tail, "provisional-1");
        assert_eq!(visible.summary_tail, "VISIBLE-SUMMARY");
        assert!(!visible.text_tail.contains("PRIVATE"));
        provider.release();
        let (mut harness, result) = run.await.unwrap();
        let result = result.unwrap();
        assert!(!result.reused);
        assert_eq!(result.output.speaker, target);
        assert_eq!(result.output.text, "settled-one");
        assert!(progress.borrow_and_update().is_none());
        let history = harness.history().await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].text_projection(), "settled-one");
        assert!(!history[1].text_projection().contains("VISIBLE-SUMMARY"));
        let calls = provider.request_count();
        let replay = harness
            .run_local_controlled(
                "preview request",
                Some(&target),
                "preview-turn",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(replay.reused);
        assert_eq!(provider.request_count(), calls);
        assert!(
            progress.borrow().is_none(),
            "exact retry replayed a preview"
        );
        harness.shutdown(false).await.unwrap();
        memory.close().await.unwrap();
    }
}

#[tokio::test]
async fn tool_loop_replaces_preview_and_partial_call_has_no_authority() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let (provider, mut stages) = StagedProvider::new(true);
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
    let mut progress = harness.subscribe_progress();
    let mut events = harness.subscribe();
    let run_target = target.clone();
    let run = tokio::spawn(async move {
        let result = harness
            .run_local_controlled(
                "tool request",
                Some(&run_target),
                "tool-turn",
                &CancellationToken::new(),
            )
            .await;
        (harness, result)
    });
    stage(&mut stages, 1).await;
    provider.release();
    stage(&mut stages, 2).await;
    let first = preview(&progress);
    assert_eq!(first.text_tail, "provisional-1");
    assert_eq!(first.summary_tail, "VISIBLE-SUMMARY");
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, Event::ToolSettled { .. }));
    }
    provider.release();
    stage(&mut stages, 3).await;
    let second = preview(&progress);
    assert_eq!(second.request_round, 2);
    assert!(second.seq > first.seq);
    assert_eq!(second.text_tail, "provisional-2");
    assert_eq!(second.summary_tail, "");
    provider.release();
    let (mut harness, result) = run.await.unwrap();
    assert_eq!(result.unwrap().output.text, "settled-two");
    assert!(progress.borrow_and_update().is_none());
    assert!(
        harness
            .topology
            .states
            .get(&target)
            .is_some_and(|state| state.note == "bounded preview")
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn relationship_consultation_and_dream_never_publish_private_streams() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let (provider, mut stages) = StagedProvider::new(false);
    let mut dream_config = config();
    dream_config.dream_every = 1;
    let mut harness = Harness::new(
        dream_config,
        directory.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let ids = harness
        .topology
        .parts
        .iter()
        .take(2)
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let relation = harness
        .relate(RelationshipKind::Alliance, ids.clone())
        .await
        .unwrap();
    provider.consult(relation.id);
    let active_parts = harness.topology.parts.len();
    let mut progress = harness.subscribe_progress();
    let run_target = ids[0].clone();
    let run = tokio::spawn(async move {
        let result = harness
            .run_local_controlled(
                "consult then dream",
                Some(&run_target),
                "private-stream-turn",
                &CancellationToken::new(),
            )
            .await;
        (harness, result)
    });
    stage(&mut stages, 1).await;
    assert!(progress.borrow().is_none());
    provider.release();
    stage(&mut stages, 2).await;
    assert_eq!(preview(&progress).text_tail, "provisional-1");
    provider.release();
    stage(&mut stages, 3).await;
    let while_consulting = preview(&progress);
    assert!(!while_consulting.text_tail.contains("PRIVATE-CONSULTATION"));
    assert!(
        !while_consulting
            .summary_tail
            .contains("PRIVATE-CONSULTATION")
    );
    provider.release();
    stage(&mut stages, 4).await;
    assert_eq!(preview(&progress).text_tail, "provisional-2");
    provider.release();
    stage(&mut stages, 5).await;
    assert!(
        progress.borrow_and_update().is_none(),
        "dream leaked into preview"
    );
    for _ in 0..active_parts {
        provider.release();
    }
    let (mut harness, result) = tokio::time::timeout(std::time::Duration::from_secs(20), run)
        .await
        .expect("dream did not finish")
        .unwrap();
    assert_eq!(result.unwrap().output.text, "settled-two");
    assert!(
        harness
            .history()
            .await
            .unwrap()
            .iter()
            .all(|message| !message.text_projection().contains("PRIVATE-"))
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_discards_preview_and_resume_keeps_only_durable_marker() {
    let directory = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let (provider, mut stages) = StagedProvider::new(false);
    let mut harness = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let session = harness.session.id.clone();
    let target = harness.topology.parts[0].id.clone();
    let mut progress = harness.subscribe_progress();
    let cancellation = CancellationToken::new();
    let run_cancel = cancellation.clone();
    let run_target = target.clone();
    let run = tokio::spawn(async move {
        let result = harness
            .run_local_controlled(
                "cancel request",
                Some(&run_target),
                "cancel-turn",
                &run_cancel,
            )
            .await;
        (harness, result)
    });
    stage(&mut stages, 1).await;
    provider.release();
    stage(&mut stages, 2).await;
    assert_eq!(preview(&progress).text_tail, "provisional-1");
    cancellation.cancel();
    let (mut harness, result) = run.await.unwrap();
    assert!(result.is_err());
    assert!(progress.borrow_and_update().is_none());
    let history = harness.history().await.unwrap();
    assert!(
        history
            .iter()
            .all(|message| !message.text_projection().contains("provisional-1"))
    );
    assert!(harness.retry_last(&CancellationToken::new()).await.is_err());
    harness.shutdown(false).await.unwrap();
    let mut resumed = Harness::new(
        config(),
        directory.path(),
        memory.clone(),
        provider,
        Some(&session),
    )
    .await
    .unwrap();
    assert!(
        resumed
            .history()
            .await
            .unwrap()
            .iter()
            .all(|message| !message.text_projection().contains("provisional-1"))
    );
    assert!(resumed.subscribe_progress().borrow().is_none());
    resumed.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}
