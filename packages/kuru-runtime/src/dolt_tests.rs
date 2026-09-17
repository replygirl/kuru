use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{DemoProvider, Provider};
use kuru_core::{
    Completion, CompletionRequest, Config, Mode, ModelInfo, ToolCall, canonical_peer_instruction,
};
use kuru_memory::MemoryStore;
use serde_json::json;
use tokio::sync::{Mutex, Notify, mpsc};

use crate::{
    CancellationToken, DreamProposal, Harness, Topology,
    engine::{PendingPublication, turn_was_cancelled},
};

fn config() -> Config {
    Config {
        mode: Mode::Freudian,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}

struct HeldDream {
    started: mpsc::UnboundedSender<CompletionRequest>,
}

struct HeldPeriodicDream {
    calls: std::sync::atomic::AtomicUsize,
    started: Notify,
}

struct StalePeriodicDream {
    live: MemoryStore,
    calls: std::sync::atomic::AtomicUsize,
    wrote: AtomicBool,
}

#[async_trait]
impl Provider for StalePeriodicDream {
    async fn stream(
        &self,
        request: kuru_core::CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if request.instructions.contains("Phase: dream") {
            if !self.wrote.swap(true, Ordering::SeqCst) {
                self.live
                    .append("post-answer-live", "user", "force stale dream")
                    .await?;
            }
            Ok(Completion::from_legacy("candidate summary", vec![], 0, 0))
        } else {
            Ok(Completion::from_legacy(
                "answer before failed maintenance",
                vec![],
                0,
                0,
            ))
        }
    }
}

#[async_trait]
impl Provider for HeldPeriodicDream {
    async fn stream(
        &self,
        request: kuru_core::CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if request.instructions.contains("Phase: dream") {
            self.started.notify_waiters();
            std::future::pending().await
        } else {
            Ok(Completion::from_legacy(
                "answer before maintenance",
                vec![],
                4,
                2,
            ))
        }
    }
}
#[async_trait]
impl Provider for HeldDream {
    async fn stream(
        &self,
        request: kuru_core::CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        if request.instructions.contains("Phase: dream") {
            self.started.send(request).unwrap();
            std::future::pending().await
        } else {
            Ok(Completion::from_legacy(
                "A later conversation still works",
                vec![],
                0,
                0,
            ))
        }
    }
}

#[tokio::test]
async fn explicit_cancellation_keeps_live_dream_state_and_private_histories_isolated() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let (started, mut requests) = mpsc::unbounded_channel();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(HeldDream { started }),
        None,
    )
    .await
    .unwrap();
    let identities = harness
        .topology
        .parts
        .iter()
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let namespaces = identities
        .iter()
        .map(|id| harness.namespace(id))
        .collect::<Vec<_>>();
    for (index, namespace) in namespaces.iter().enumerate() {
        memory
            .append(namespace, "user", &format!("PRIVATE-MARKER-{index}"))
            .await
            .unwrap();
    }
    let before_revision = memory.revision().await.unwrap();
    let before_topology = serde_json::to_value(&harness.topology).unwrap();
    let before_dispatch = CancellationToken::new();
    before_dispatch.cancel();
    assert!(turn_was_cancelled(
        &harness
            .dream_controlled(&before_dispatch)
            .await
            .unwrap_err()
    ));
    assert_eq!(memory.revision().await.unwrap(), before_revision);
    let harness = Arc::new(Mutex::new(harness));
    let cancellation = CancellationToken::new();
    let running = tokio::spawn({
        let harness = harness.clone();
        let cancellation = cancellation.clone();
        async move { harness.lock().await.dream_controlled(&cancellation).await }
    });
    for _ in &identities {
        let request = tokio::time::timeout(Duration::from_secs(30), requests.recv())
            .await
            .unwrap()
            .unwrap();
        let index = namespaces
            .iter()
            .position(|namespace| *namespace == request.actor)
            .unwrap();
        let contents = serde_json::to_string(&request.messages).unwrap();
        assert!(contents.contains(&format!("PRIVATE-MARKER-{index}")));
        for other in 0..identities.len() {
            if other != index {
                assert!(!contents.contains(&format!("PRIVATE-MARKER-{other}")));
            }
        }
    }
    // Every peer has consumed its candidate input, but no live history changed.
    assert_eq!(memory.revision().await.unwrap(), before_revision);
    for namespace in &namespaces {
        assert_eq!(memory.history(namespace, 100).await.unwrap().len(), 1);
    }
    cancellation.cancel();
    assert!(turn_was_cancelled(&running.await.unwrap().unwrap_err()));
    let mut harness = harness.lock().await;
    harness.reconcile().await.unwrap();
    assert_eq!(
        serde_json::to_value(&harness.topology).unwrap(),
        before_topology
    );
    assert_eq!(memory.revision().await.unwrap(), before_revision);
    for namespace in &namespaces {
        assert!(
            memory
                .history(&format!("{namespace}/notes"), 100)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let output = harness.run("Continue after cancellation").await.unwrap();
    assert_eq!(output.text, "A later conversation still works");
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn periodic_dream_cancellation_preserves_the_exact_completed_output() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(HeldPeriodicDream {
        calls: std::sync::atomic::AtomicUsize::new(0),
        started: Notify::new(),
    });
    let harness = Harness::new(
        Config {
            dream_every: 1,
            ..config()
        },
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let mut events = harness.subscribe();
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let dream_started = provider.started.notified();
    tokio::pin!(dream_started);
    let task = tokio::spawn(async move {
        let mut harness = harness;
        let output = harness
            .run_controlled(
                "finish before dreaming",
                Some(&target),
                "periodic-boundary",
                &task_cancellation,
            )
            .await;
        (harness, output, target)
    });
    tokio::time::timeout(Duration::from_secs(30), dream_started)
        .await
        .unwrap();
    cancellation.cancel();
    let (mut harness, output, target) = task.await.unwrap();
    let output = output.unwrap();
    assert!(matches!(output.events.last(), Some(event) if event.kind() == "response"));
    assert!(!output.events.iter().any(|event| event.kind() == "dream"));
    let calls = provider.calls.load(Ordering::SeqCst);
    let retry = harness
        .run_controlled(
            "finish before dreaming",
            Some(&target),
            "periodic-boundary",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&retry).unwrap(),
        serde_json::to_vec(&output).unwrap()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    let mut kinds = Vec::new();
    while let Ok(event) = events.try_recv() {
        kinds.push(event.kind().to_owned());
    }
    assert!(kinds.iter().any(|kind| kind == "response"));
    assert!(kinds.iter().any(|kind| kind == "dream"));
    assert_eq!(harness.history().await.unwrap().len(), 2);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn stale_periodic_dream_failure_preserves_the_exact_completed_output() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(StalePeriodicDream {
        live: memory.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        wrote: AtomicBool::new(false),
    });
    let mut harness = Harness::new(
        Config {
            dream_every: 1,
            ..config()
        },
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let mut events = harness.subscribe();
    let output = harness
        .run_controlled(
            "finish despite stale dream",
            Some(&target),
            "failed-periodic-boundary",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(output.events.last(), Some(event) if event.kind() == "response"));
    assert!(!output.events.iter().any(|event| event.kind() == "dream"));
    let calls = provider.calls.load(Ordering::SeqCst);
    let retry = harness
        .run_controlled(
            "finish despite stale dream",
            Some(&target),
            "failed-periodic-boundary",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&retry).unwrap(),
        serde_json::to_vec(&output).unwrap()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), calls);
    let mut observed_failure = false;
    while let Ok(event) = events.try_recv() {
        observed_failure |=
            event.kind() == "error" && event.actor() == "dream" && event.detail().contains("stale");
    }
    assert!(observed_failure);
    assert_eq!(harness.history().await.unwrap().len(), 2);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

struct ConcurrentWriter {
    memory: MemoryStore,
    wrote: AtomicBool,
}
#[async_trait]
impl Provider for ConcurrentWriter {
    async fn stream(
        &self,
        request: kuru_core::CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        if !self.wrote.swap(true, Ordering::SeqCst) {
            self.memory
                .append("concurrent-chat", "user", "A later live message")
                .await?;
            Ok(Completion::from_legacy(
                "Candidate summary",
                vec![ToolCall {
                    id: "add-member".into(),
                    name: "dream_suggest".into(),
                    arguments: json!({"action":"add","name":"Observer","role":"ego","instruction":"Consider overlooked details"}),
                }],
                0,
                0,
            ))
        } else {
            Ok(Completion::from_legacy(
                "Another candidate summary",
                vec![],
                0,
                0,
            ))
        }
    }
}

#[tokio::test]
async fn stale_promotion_keeps_later_live_data_and_discards_all_candidate_effects() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = Arc::new(ConcurrentWriter {
        memory: memory.clone(),
        wrote: AtomicBool::new(false),
    });
    let mut harness = Harness::new(config(), project.path(), memory.clone(), provider, None)
        .await
        .unwrap();
    let before_topology = serde_json::to_value(&harness.topology).unwrap();
    let error = harness.dream().await.unwrap_err();
    assert!(
        format!("{error:#}").contains("candidate is stale"),
        "{error:#}"
    );
    harness.reconcile().await.unwrap();
    assert_eq!(
        serde_json::to_value(&harness.topology).unwrap(),
        before_topology
    );
    assert_eq!(
        memory.history("concurrent-chat", 10).await.unwrap()[0].text_projection(),
        "A later live message"
    );
    for part in &harness.topology.parts {
        assert!(harness.memory_for(&part.id).await.unwrap().is_empty());
        assert!(
            memory
                .history(&format!("{}/notes", harness.namespace(&part.id)), 100)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert!(
        memory
            .history(&format!("{}/freudian/dream-log", harness.scope), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        memory
            .get(&format!("{}/freudian/dream-undo", harness.scope))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        memory
            .get(&format!("{}/freudian/last-dream", harness.scope))
            .await
            .unwrap()
            .is_none()
    );
    let output = harness.run("Continue after failed dream").await.unwrap();
    assert_eq!(output.text, "Another candidate summary");
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn undo_is_a_new_revision_that_preserves_later_chats_preferences_and_archived_history() {
    let project = tempfile::tempdir().unwrap();
    let data = kuru_memory::test_support::tempdir().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().into(),
        crate::project_scope(project.path()).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Observer".into(),
            role: "ego".into(),
            instruction: "Consider overlooked details".into(),
        }])
        .await
        .unwrap();
    let added = harness.resolve("Observer").unwrap();
    harness
        .run_for("Remember this message after undo", Some(&added))
        .await
        .unwrap();
    harness
        .set_model("later-demo", Some("high".into()))
        .await
        .unwrap();
    let before_history = serde_json::to_value(harness.history().await.unwrap()).unwrap();
    let before_private = serde_json::to_value(harness.memory_for(&added).await.unwrap()).unwrap();
    let before_revision = memory.revision().await.unwrap();
    let session = harness.session.id.clone();
    harness.undo_dream().await.unwrap();
    let undo_revision = memory.revision().await.unwrap();
    assert_ne!(before_revision, undo_revision);
    assert!(
        memory
            .revisions(100)
            .await
            .unwrap()
            .iter()
            .any(|revision| revision.hash == before_revision)
    );
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    drop(harness);

    let memory = MemoryStore::open(options).await.unwrap();
    let preferences = Harness::load_preferences(&memory, project.path())
        .await
        .unwrap();
    assert_eq!(preferences.providers["demo"].model, "later-demo");
    assert_eq!(
        preferences.providers["demo"].effort.as_deref(),
        Some("high")
    );
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory,
        Arc::new(DemoProvider),
        Some(&session),
    )
    .await
    .unwrap();
    assert_eq!(harness.session.turns, 1);
    assert_eq!(
        serde_json::to_value(harness.history().await.unwrap()).unwrap(),
        before_history
    );
    assert_eq!(
        serde_json::to_value(harness.memory_for(&added).await.unwrap()).unwrap(),
        before_private
    );
    assert!(
        !harness
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    assert!(harness.resolve(&added).is_err());
    assert!(harness.undo_dream().await.is_err());
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn canonical_dream_additions_survive_promotion_stopped_reload_and_reversal() {
    let project = tempfile::tempdir().unwrap();
    let data = kuru_memory::test_support::tempdir().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().into(),
        crate::project_scope(project.path()).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    let session = harness.session.id.clone();
    let legacy_id = harness.topology.parts[1].id.clone();
    let legacy_instruction = "Legacy persisted instruction without the current preamble 🪶";
    harness
        .topology
        .parts
        .iter_mut()
        .find(|part| part.id == legacy_id)
        .unwrap()
        .instruction = legacy_instruction.into();
    harness.save().await.unwrap();
    let role = harness.topology.parts[0].role.clone();
    let tendency = "Keep the reviewed tendency bytes unchanged 🪶";
    let canonical = canonical_peer_instruction(tendency).unwrap();
    let prefix = canonical.strip_suffix(tendency).unwrap();
    let submitted = format!("{prefix}{prefix}{tendency}");

    let report = harness
        .apply_dream(vec![
            DreamProposal::Add {
                name: "Canonical observer".into(),
                role: role.clone(),
                instruction: submitted.clone(),
            },
            DreamProposal::Add {
                name: "Rejected prefix only".into(),
                role,
                instruction: prefix.into(),
            },
        ])
        .await
        .unwrap();
    assert_eq!(report.accepted.len(), 1);
    assert_eq!(report.rejected.len(), 1);
    assert!(matches!(
        &report.accepted[0],
        DreamProposal::Add { instruction, .. } if instruction == &submitted
    ));
    let added = harness.resolve("Canonical observer").unwrap();
    assert_eq!(
        harness
            .topology
            .parts
            .iter()
            .find(|part| part.id == legacy_id)
            .unwrap()
            .instruction,
        legacy_instruction
    );
    assert_eq!(
        harness
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .instruction,
        canonical
    );

    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    drop(harness);

    let memory = MemoryStore::open(options).await.unwrap();
    let mut reloaded = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        Some(&session),
    )
    .await
    .unwrap();
    assert_eq!(
        reloaded
            .topology
            .parts
            .iter()
            .find(|part| part.id == legacy_id)
            .unwrap()
            .instruction,
        legacy_instruction
    );
    assert_eq!(
        reloaded
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .instruction,
        canonical
    );
    reloaded.undo_dream().await.unwrap();
    assert_eq!(
        reloaded
            .topology
            .parts
            .iter()
            .find(|part| part.id == legacy_id)
            .unwrap()
            .instruction,
        legacy_instruction
    );
    assert!(
        !reloaded
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    reloaded.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_live_undo_reconciles_before_a_later_save() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    let role = harness.topology.parts[0].role.clone();
    harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Interrupted undo identity".into(),
            role,
            instruction: "Remain archived after the undo".into(),
        }])
        .await
        .unwrap();
    let added = harness.resolve("Interrupted undo identity").unwrap();
    memory
        .append(
            &harness.namespace(&added),
            "user",
            "later conversation survives",
        )
        .await
        .unwrap();
    let harness = Arc::new(Mutex::new(harness));
    let (written, release) = {
        let mut harness = harness.lock().await;
        harness.pause_after_next_memory_write()
    };
    let running = tokio::spawn({
        let harness = harness.clone();
        async move { harness.lock().await.undo_dream().await }
    });
    tokio::time::timeout(Duration::from_secs(10), written)
        .await
        .unwrap()
        .unwrap();
    // The actual live undo has made its SQL write, but cancellation prevents
    // `persist_state` from publishing that topology in the harness.
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    drop(release);

    let mut harness = harness.lock().await;
    assert!(harness.pending_publication.is_some());
    assert!(
        harness
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    harness.reconcile().await.unwrap();
    assert!(
        !harness
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    harness.set_effort(Some("high".into())).await.unwrap();
    let durable: Topology = serde_json::from_value(
        memory
            .get(&format!(
                "{}/{}/topology",
                harness.scope, harness.config.mode
            ))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        !durable
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    assert!(
        memory
            .history(&harness.namespace(&added), 10)
            .await
            .unwrap()
            .iter()
            .any(|message| message.text_projection() == "later conversation survives")
    );
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn reconciliation_publishes_only_durable_choices_before_the_next_mutation() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut harness = Harness::new(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    // Model the acknowledged database write whose caller was cancelled before publication.
    let mut config = harness.config.clone();
    config.model = "durable-model".into();
    let updates = vec![(
        format!("{}/preferences", harness.scope),
        json!({"providers":{"demo":{"model":"durable-model","effort":null}}}),
    )];
    harness.pending_publication = Some(PendingPublication {
        config,
        profile: harness.profile.clone(),
        actor_namespaces: crate::engine::prepared_actor_namespaces(
            &harness.scope,
            &harness.profile,
            &harness.topology,
        )
        .unwrap(),
        topology: harness.topology.clone(),
        session: harness.session.clone(),
        updates: updates.clone(),
    });
    memory.put_many(&updates).await.unwrap();
    assert_eq!(harness.config.model, "demo");
    harness.set_effort(Some("high".into())).await.unwrap();
    assert_eq!(harness.config.model, "durable-model");
    assert_eq!(harness.config.effort.as_deref(), Some("high"));
    let mut unpublished = harness.config.clone();
    unpublished.model = "not-committed".into();
    harness.pending_publication = Some(PendingPublication {
        config: unpublished,
        profile: harness.profile.clone(),
        actor_namespaces: crate::engine::prepared_actor_namespaces(
            &harness.scope,
            &harness.profile,
            &harness.topology,
        )
        .unwrap(),
        topology: harness.topology.clone(),
        session: harness.session.clone(),
        updates: vec![("missing-write".into(), json!(true))],
    });
    harness.reconcile().await.unwrap();
    assert_eq!(harness.config.model, "durable-model");
    assert!(harness.pending_publication.is_none());
    harness.shutdown(false).await.unwrap();
}
