use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{DemoProvider, Provider};
use kuru_core::{Completion, CompletionRequest, Config, Mode, ModelInfo, ToolCall};
use kuru_memory::MemoryStore;
use serde_json::json;
use tokio::sync::{Mutex, mpsc};

use crate::{DreamProposal, Harness, engine::PendingPublication};

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
#[async_trait]
impl Provider for HeldDream {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        if request.instructions.contains("Phase: dream") {
            self.started.send(request).unwrap();
            std::future::pending().await
        } else {
            Ok(Completion {
                text: "A later conversation still works".into(),
                ..Completion::default()
            })
        }
    }
}

#[tokio::test]
async fn cancellation_discards_the_whole_dream_and_each_peer_sees_only_its_history() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let (started, mut requests) = mpsc::unbounded_channel();
    let harness = Harness::new(
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
    let harness = Arc::new(Mutex::new(harness));
    let running = tokio::spawn({
        let harness = harness.clone();
        async move { harness.lock().await.dream().await }
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
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
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

struct ConcurrentWriter {
    memory: MemoryStore,
    wrote: AtomicBool,
}
#[async_trait]
impl Provider for ConcurrentWriter {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn complete(&self, _request: CompletionRequest) -> Result<Completion> {
        if !self.wrote.swap(true, Ordering::SeqCst) {
            self.memory
                .append("concurrent-chat", "user", "A later live message")
                .await?;
            Ok(Completion {
                text: "Candidate summary".into(),
                calls: vec![ToolCall {
                    id: "add-member".into(),
                    name: "dream_suggest".into(),
                    arguments: json!({"action":"add","name":"Observer","role":"ego","instruction":"Consider overlooked details"}),
                }],
                ..Completion::default()
            })
        } else {
            Ok(Completion {
                text: "Another candidate summary".into(),
                ..Completion::default()
            })
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
        memory.history("concurrent-chat", 10).await.unwrap()[0].content,
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
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn undo_is_a_new_revision_that_preserves_later_chats_preferences_and_archived_history() {
    let project = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
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
    drop(memory);

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
        topology: harness.topology.clone(),
        session: harness.session.clone(),
        updates: vec![("missing-write".into(), json!(true))],
    });
    harness.reconcile().await.unwrap();
    assert_eq!(harness.config.model, "durable-model");
    assert!(harness.pending_publication.is_none());
    harness.shutdown(false).await.unwrap();
}
