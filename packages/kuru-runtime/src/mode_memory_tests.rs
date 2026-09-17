//! Real-Dolt proof that the selected memory policy changes only checked
//! identity suffixes while preserving the durable project/state contract.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{Provider, ProviderEvent, ProviderSink};
use kuru_core::{
    Completion, CompletionRequest, Config, ConsolidationPlan, MemoryPolicy, Mode, ModeProfile,
    ModelInfo, Part, RolesPolicy, StateKeys,
};
use kuru_memory::MemoryStore;
use tokio::sync::mpsc;

use crate::{CancellationToken, DreamProposal, Harness};

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

#[derive(Clone)]
enum Plan {
    Ordered,
    Frozen(String),
}

#[derive(Clone)]
struct AlternateMemory {
    plan: Plan,
    invalid_namespace: bool,
}

impl MemoryPolicy for AlternateMemory {
    fn identity_namespace(&self, scope: &str, mode: Mode, identity: &str) -> String {
        if self.invalid_namespace {
            format!("{scope}/{mode}/outside")
        } else {
            format!("{scope}/{mode}/identity/{identity}/alternate")
        }
    }

    fn transcript_namespace(&self, scope: &str, session: &str) -> String {
        format!("{scope}/transcript/{session}")
    }

    fn state_keys(&self, scope: &str, mode: Mode) -> StateKeys {
        StateKeys {
            topology: format!("{scope}/{mode}/topology"),
            dream_undo: format!("{scope}/{mode}/dream-undo"),
        }
    }

    fn consolidation_plan(&self, active: &[String]) -> ConsolidationPlan {
        let participants = match &self.plan {
            Plan::Ordered if active.len() > 1 => vec![active[1].clone(), active[0].clone()],
            Plan::Ordered => active.to_vec(),
            Plan::Frozen(id) => vec![id.clone()],
        };
        ConsolidationPlan {
            participants,
            prompt: "write the selected durable summary".into(),
            phase: "dream: selected alternate memory".into(),
            max_proposals_per_part: 2,
        }
    }
}

struct DuplicateRoles(Vec<Part>);

impl RolesPolicy for DuplicateRoles {
    fn seeds(&self) -> Vec<Part> {
        self.0.clone()
    }
}

#[derive(Default)]
struct TaggedProvider {
    requests: Mutex<Vec<CompletionRequest>>,
}

impl TaggedProvider {
    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for TaggedProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let identity = request
            .actor
            .strip_suffix("/alternate")
            .unwrap_or(&request.actor)
            .rsplit('/')
            .next()
            .unwrap()
            .to_owned();
        self.requests.lock().unwrap().push(request);
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            format!("summary-for-{identity}"),
            vec![],
            3,
            2,
        )))
        .await
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

    async fn stream(&self, request: CompletionRequest, _sink: &mut dyn ProviderSink) -> Result<()> {
        self.started.send(request).unwrap();
        std::future::pending().await
    }
}

fn alternate_profile(plan: Plan) -> ModeProfile {
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    profile.memory = Arc::new(AlternateMemory {
        plan,
        invalid_namespace: false,
    });
    profile
}

async fn open_memory(
    project: &Path,
) -> (
    MemoryStore,
    kuru_memory::OpenOptions,
    kuru_memory::test_support::TempDir,
) {
    let data = kuru_memory::test_support::tempdir().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().into(),
        crate::project_scope(project).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    (memory, options, data)
}

#[tokio::test]
async fn alternate_suffix_survives_real_dolt_dream_promotion_undo_and_reopen() {
    let project = tempfile::tempdir().unwrap();
    let (memory, options, _data) = open_memory(project.path()).await;
    let profile = alternate_profile(Plan::Ordered);
    let provider = Arc::new(TaggedProvider::default());
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile.clone(),
    )
    .await
    .unwrap();
    let ids = harness
        .topology
        .parts
        .iter()
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    let selected = vec![ids[1].clone(), ids[0].clone()];
    assert!(
        harness
            .checked_namespace(&ids[0])
            .unwrap()
            .ends_with("/alternate")
    );

    harness
        .run_for("store one selected private turn", Some(&ids[0]))
        .await
        .unwrap();
    let dream = harness.dream().await.unwrap();
    assert_eq!(dream.summaries, selected.len());
    for id in &selected {
        assert!(
            harness
                .notes_for(id, 10)
                .await
                .unwrap()
                .notes
                .iter()
                .any(|note| { note.content == format!("summary-for-{id}") })
        );
    }
    // Requests can arrive concurrently. Persisted summaries prove each result
    // remained associated with the plan's identity, rather than arrival order.
    for id in &selected {
        assert!(provider.requests().iter().any(|request| {
            request
                .instructions
                .contains("Phase: dream: selected alternate memory")
                && request.actor.ends_with(&format!("/{id}/alternate"))
        }));
    }

    let added_role = harness.topology.parts[0].role.clone();
    harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Alternate retained history".into(),
            role: added_role,
            instruction: "Record later durable context.".into(),
        }])
        .await
        .unwrap();
    let added = harness.resolve("Alternate retained history").unwrap();
    let added_namespace = harness.checked_namespace(&added).unwrap();
    memory
        .append(&added_namespace, "user", "later-history-survives-undo")
        .await
        .unwrap();
    harness.undo_dream().await.unwrap();
    assert!(
        !harness
            .topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    assert!(
        memory
            .history(&added_namespace, 10)
            .await
            .unwrap()
            .iter()
            .any(|message| { message.text_projection() == "later-history-survives-undo" })
    );
    assert!(harness.session_usage().await.unwrap().invocation_count >= 3);

    let session = harness.session.id.clone();
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    let memory = MemoryStore::open(options).await.unwrap();
    let mut resumed = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider,
        Some(&session),
        profile,
    )
    .await
    .unwrap();
    assert!(
        resumed
            .notes_for(&ids[0], 10)
            .await
            .unwrap()
            .notes
            .iter()
            .any(|note| { note.content == format!("summary-for-{}", ids[0]) })
    );
    assert!(
        memory
            .history(&added_namespace, 10)
            .await
            .unwrap()
            .iter()
            .any(|message| { message.text_projection() == "later-history-survives-undo" })
    );
    resumed.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_alternate_dream_abandons_its_candidate_without_live_effects() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let profile = alternate_profile(Plan::Ordered);
    let (started, mut requests) = mpsc::unbounded_channel();
    let harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        Arc::new(HeldDream { started }),
        None,
        profile,
    )
    .await
    .unwrap();
    let before = memory.revision().await.unwrap();
    let namespaces = harness
        .topology
        .parts
        .iter()
        .take(2)
        .map(|part| harness.checked_namespace(&part.id).unwrap())
        .collect::<Vec<_>>();
    let harness = Arc::new(tokio::sync::Mutex::new(harness));
    let cancellation = CancellationToken::new();
    let running = tokio::spawn({
        let harness = harness.clone();
        let cancellation = cancellation.clone();
        async move { harness.lock().await.dream_controlled(&cancellation).await }
    });
    let namespaces = namespaces
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let mut expected = namespaces.clone();
    for _ in 0..expected.len() {
        let request = tokio::time::timeout(std::time::Duration::from_secs(30), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(expected.remove(&request.actor));
    }
    assert!(expected.is_empty());
    cancellation.cancel();
    assert!(crate::turn_was_cancelled(
        &running.await.unwrap().unwrap_err()
    ));
    let mut harness = harness.lock().await;
    harness.reconcile().await.unwrap();
    assert_eq!(memory.revision().await.unwrap(), before);
    let usage = harness.session_usage().await.unwrap();
    assert_eq!(usage.invocation_count, namespaces.len() as u64);
    assert_eq!(usage.incomplete_invocations, namespaces.len() as u64);
    for namespace in namespaces {
        assert!(
            memory
                .history(&format!("{namespace}/notes"), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[tokio::test]
async fn invalid_namespace_and_stale_actual_plan_refuse_before_provider_or_candidate_effects() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let before = memory.revision().await.unwrap();
    let provider = Arc::new(TaggedProvider::default());
    let mut invalid = ModeProfile::builtin(Mode::Ifs);
    invalid.memory = Arc::new(AlternateMemory {
        plan: Plan::Ordered,
        invalid_namespace: true,
    });
    assert!(
        Harness::new_with_test_profile(
            config(),
            project.path(),
            memory.clone(),
            provider.clone(),
            None,
            invalid
        )
        .await
        .is_err()
    );
    assert_eq!(memory.revision().await.unwrap(), before);
    assert!(provider.requests().is_empty());
    memory.close().await.unwrap();

    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let mut profile = ModeProfile::builtin(Mode::Ifs);
    let mut seeds = profile.roles.seeds();
    seeds[1].role = seeds[0].role.clone();
    let retired = seeds[0].id.clone();
    profile.roles = Arc::new(DuplicateRoles(seeds));
    profile.memory = Arc::new(AlternateMemory {
        plan: Plan::Frozen(retired.clone()),
        invalid_namespace: false,
    });
    let provider = Arc::new(TaggedProvider::default());
    let mut harness = Harness::new_with_test_profile(
        config(),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
        profile,
    )
    .await
    .unwrap();
    harness
        .apply_dream(vec![DreamProposal::Retire { id: retired }])
        .await
        .unwrap();
    let before = memory.revision().await.unwrap();
    assert!(harness.dream().await.is_err());
    assert_eq!(memory.revision().await.unwrap(), before);
    assert!(provider.requests().is_empty());
    assert_eq!(harness.session_usage().await.unwrap().invocation_count, 0);
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
}

#[test]
fn builtin_memory_namespaces_and_consolidation_plans_remain_exact() {
    const PROMPT: &str = "Review your own history. Write a concise durable memory summary of useful facts and unresolved concerns. You may suggest a new complementary member of an existing role or retire yourself if your role is redundantly covered. A suggestion is optional; do not manufacture changes. No other tools are available during dreaming.";
    const PHASE: &str = "dream: consolidate your own memory, optionally propose membership changes";
    for mode in Mode::ALL {
        let profile = ModeProfile::builtin(mode);
        let scope = "project/golden";
        let active = profile.roles.authored_order();
        assert_eq!(
            profile.memory.identity_namespace(scope, mode, &active[0]),
            format!("{scope}/{mode}/identity/{}", active[0])
        );
        assert_eq!(
            profile.memory.transcript_namespace(scope, "session"),
            format!("{scope}/transcript/session")
        );
        assert_eq!(
            profile.memory.state_keys(scope, mode),
            StateKeys {
                topology: format!("{scope}/{mode}/topology"),
                dream_undo: format!("{scope}/{mode}/dream-undo")
            }
        );
        assert_eq!(
            profile.memory.consolidation_plan(&active),
            ConsolidationPlan {
                participants: active,
                prompt: PROMPT.into(),
                phase: PHASE.into(),
                max_proposals_per_part: 2
            }
        );
    }
}
