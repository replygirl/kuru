use kuru_memory::MemoryStore;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use kuru_connectors::{Provider, ToolHost};
use kuru_core::{
    Completion, CompletionRequest, Config, Mode, ModelInfo, RelationshipKind, ToolCall,
    canonical_peer_instruction,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::{
    DreamProposal, Harness, PeerMessage,
    engine::{StateReport, read_topology, validate_topology},
    undo_dream,
};

type Responder = dyn Fn(&CompletionRequest) -> Completion + Send + Sync;
struct Fake {
    requests: Mutex<Vec<CompletionRequest>>,
    respond: Box<Responder>,
    active: AtomicUsize,
    peak: AtomicUsize,
    first_pair: Option<tokio::sync::Barrier>,
    started: AtomicUsize,
}
impl Fake {
    fn new(
        respond: impl Fn(&CompletionRequest) -> Completion + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(vec![]),
            respond: Box::new(respond),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            first_pair: None,
            started: AtomicUsize::new(0),
        })
    }
}
#[async_trait]
impl Provider for Fake {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        if let Some(barrier) = &self.first_pair
            && self.started.fetch_add(1, Ordering::SeqCst) < 2
        {
            tokio::time::timeout(Duration::from_secs(10), barrier.wait()).await?;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        let reply = (self.respond)(&request);
        self.requests.lock().unwrap().push(request);
        Ok(reply)
    }
}
fn answer(text: &str) -> Completion {
    Completion::from_legacy(text, vec![], 7, 3)
}
fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        arguments: args,
    }
}
async fn fixture(mode: Mode, fake: Arc<dyn Provider>) -> (TempDir, Harness) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config {
        mode,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    };
    let harness = Harness::new(
        config,
        directory.path(),
        MemoryStore::temporary().await.unwrap(),
        fake,
        None,
    )
    .await
    .unwrap();
    (directory, harness)
}

#[tokio::test]
async fn equal_activation_keeps_the_same_speaker_across_completed_turns() {
    let provider = Fake::new(|request| {
        if request.instructions.contains("Phase: deliberate") {
            Completion::from_legacy(
                "equal contribution",
                vec![call(
                    "state_report",
                    json!({"activation":0.5,"note":"equal fixture"}),
                )],
                1,
                1,
            )
        } else {
            answer("completed reply")
        }
    });
    let (_directory, mut harness) = fixture(Mode::Ifs, provider).await;
    let first = harness.run("first equal turn").await.unwrap();
    let second = harness.run("second equal turn").await.unwrap();
    assert_eq!(second.speaker, first.speaker);
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn automatic_speaker_selection_is_stable_and_persists_after_dolt_reopen() {
    let project = tempfile::tempdir().unwrap();
    let data = kuru_memory::test_support::tempdir().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().to_owned(),
        crate::project_scope(project.path()).unwrap(),
    )
    .unwrap();
    let higher = Arc::new(Mutex::new(None::<String>));
    let provider = Fake::new({
        let higher = higher.clone();
        move |request| {
            if request.instructions.contains("Phase: deliberate") {
                let activation = if higher
                    .lock()
                    .unwrap()
                    .as_deref()
                    .is_some_and(|id| request.actor.ends_with(id))
                {
                    0.9
                } else {
                    0.5
                };
                Completion::from_legacy(
                    format!("draft from {}", request.actor),
                    vec![call(
                        "state_report",
                        json!({"activation":activation,"note":"fixture"}),
                    )],
                    1,
                    1,
                )
            } else {
                answer(&format!("spoken by {}", request.actor))
            }
        }
    });
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
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
    let first = harness.run("first tie").await.unwrap();
    assert_eq!(
        first
            .events
            .iter()
            .find(|event| event.kind == "speaker-selection")
            .unwrap()
            .detail,
        "mode-authored-order"
    );
    let second = harness.run("second tie").await.unwrap();
    assert_eq!(second.speaker, first.speaker);
    assert_eq!(
        second
            .events
            .iter()
            .find(|event| event.kind == "speaker-selection")
            .unwrap()
            .detail,
        "previous-completed-speaker"
    );
    let session = harness.session.id.clone();
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    drop(harness);

    let memory = MemoryStore::open(options).await.unwrap();
    let mut resumed = Harness::new(
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
        Some(&session),
    )
    .await
    .unwrap();
    let resumed_turn = resumed.run("resumed tie").await.unwrap();
    assert_eq!(resumed_turn.speaker, first.speaker);
    assert_eq!(
        resumed_turn
            .events
            .iter()
            .find(|event| event.kind == "speaker-selection")
            .unwrap()
            .detail,
        "previous-completed-speaker"
    );
    resumed.session.last_completed_speaker = Some("retired-fixture-identity".into());
    let unavailable = resumed.run("missing prior").await.unwrap();
    assert_eq!(unavailable.speaker, first.speaker);
    assert!(unavailable.events.iter().any(|event| {
        event.kind == "speaker-selection" && event.detail == "mode-authored-order"
    }));
    let higher_id = provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| {
            request.instructions.contains("Phase: deliberate")
                && request.actor.rsplit('/').next() != Some(&first.speaker)
        })
        .unwrap()
        .actor
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    *higher.lock().unwrap() = Some(higher_id.clone());
    let higher_turn = resumed.run("higher activation").await.unwrap();
    assert_eq!(higher_turn.speaker, higher_id);
    assert_eq!(
        higher_turn
            .events
            .iter()
            .find(|event| event.kind == "speaker-selection")
            .unwrap()
            .detail,
        "maximum-activation"
    );
    let higher_again = resumed.run("higher activation again").await.unwrap();
    assert_eq!(higher_again.speaker, higher_id);
    assert!(higher_again.events.iter().any(|event| {
        event.kind == "speaker-selection" && event.detail == "maximum-activation"
    }));
    let targeted = resumed
        .run_for("target", Some(&first.speaker))
        .await
        .unwrap();
    assert_eq!(targeted.speaker, first.speaker);
    assert!(
        targeted
            .events
            .iter()
            .any(|event| event.kind == "speaker-selection" && event.detail == "caller-target")
    );
    resumed.focus(Some(&higher_id)).await.unwrap();
    let focused = resumed.run("focused").await.unwrap();
    assert_eq!(focused.speaker, higher_id);
    assert!(
        focused
            .events
            .iter()
            .any(|event| event.kind == "speaker-selection" && event.detail == "active-focus")
    );
    resumed.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn cold_ties_follow_each_modes_authored_order() {
    for (mode, expected_name) in [
        (Mode::Ifs, "Self"),
        (Mode::Polyvagal, "Connection"),
        (Mode::Freudian, "Desire"),
        (Mode::Jungian, "Continuity"),
    ] {
        let provider = Fake::new(|_| answer("equal contribution"));
        let (_directory, mut harness) = fixture(mode, provider).await;
        let expected = harness
            .topology
            .parts
            .iter()
            .find(|part| part.name == expected_name)
            .unwrap()
            .id
            .clone();

        let output = harness.run("cold tie").await.unwrap();

        assert_eq!(output.speaker, expected);
        assert!(output.events.iter().any(|event| {
            event.kind == "speaker-selection" && event.detail == "mode-authored-order"
        }));
        harness.shutdown(false).await.unwrap();
    }
}

#[tokio::test]
async fn authored_tie_break_skips_missing_parts_and_falls_back_to_stable_ids() {
    let provider = Fake::new(|_| answer("fixture"));
    let (_directory, mut harness) = fixture(Mode::Ifs, provider).await;
    let authored = kuru_core::Framework::authored_identity_order(Mode::Ifs);
    let drafts = BTreeMap::from([
        (authored[1].clone(), "second".into()),
        (authored[2].clone(), "third".into()),
    ]);
    let (speaker, reason) = harness.select_speaker(&drafts);
    assert_eq!(speaker, authored[1]);
    assert_eq!(reason, "mode-authored-order");

    let drafts = BTreeMap::from([
        ("dream-z".to_string(), "z".into()),
        ("dream-a".to_string(), "a".into()),
    ]);
    let (speaker, reason) = harness.select_speaker(&drafts);
    assert_eq!(speaker, "dream-a");
    assert_eq!(reason, "stable-id-order");
    harness.shutdown(false).await.unwrap();
}

struct FailingSpeaker(AtomicBool);

#[async_trait]
impl Provider for FailingSpeaker {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        if request.instructions.contains("Phase: deliberate") {
            Ok(Completion::from_legacy(
                "draft",
                vec![call(
                    "state_report",
                    json!({"activation":0.5,"note":"fixture"}),
                )],
                0,
                0,
            ))
        } else if self.0.load(Ordering::SeqCst) {
            Err(anyhow::anyhow!("speaking fixture failed"))
        } else {
            Ok(answer("completed"))
        }
    }
}

#[tokio::test]
async fn failed_speaking_does_not_replace_completed_speaker() {
    let provider = Arc::new(FailingSpeaker(AtomicBool::new(false)));
    let (directory, mut harness) = fixture(Mode::Ifs, provider.clone()).await;
    let completed = harness.run("completed").await.unwrap().speaker;
    let other = harness
        .topology
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .find(|id| *id != completed)
        .unwrap()
        .to_owned();
    provider.0.store(true, Ordering::SeqCst);
    assert!(harness.run_for("failed", Some(&other)).await.is_err());
    assert_eq!(
        harness.session.last_completed_speaker.as_deref(),
        Some(completed.as_str())
    );
    let saved: crate::Session = serde_json::from_value(
        harness
            .memory
            .get(&format!("{}/session/{}", harness.scope, harness.session.id))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        saved.last_completed_speaker.as_deref(),
        Some(completed.as_str())
    );
    harness.shutdown(false).await.unwrap();
    drop(directory);
}

#[test]
fn legacy_session_without_continuity_value_deserializes() {
    let session: crate::Session = serde_json::from_value(json!({
        "id":"legacy-session",
        "mode":"ifs",
        "turns":2,
        "label":"legacy"
    }))
    .unwrap();
    assert!(session.last_completed_speaker.is_none());
}

#[tokio::test]
async fn peers_are_concurrent_bounded_and_never_receive_each_others_private_context() {
    let mut fake = Fake::new(|_| answer("A useful contribution"));
    // Real durable writes may take longer than a tiny simulated model response.
    // Hold the first response until a second call proves actual pool overlap.
    Arc::get_mut(&mut fake).unwrap().first_pair = Some(tokio::sync::Barrier::new(2));
    let (_dir, mut harness) = fixture(Mode::Ifs, fake.clone()).await;
    let private_owner = harness.topology.parts[0].id.clone();
    harness
        .memory
        .append(
            &harness.namespace(&private_owner),
            "user",
            "PRIVATE-OWNER-SECRET",
        )
        .await
        .unwrap();
    let result = harness.run("List tradeoffs").await.unwrap();
    assert!(!result.text.is_empty());
    assert_eq!(harness.session.turns, 1);
    assert_eq!(harness.history().await.unwrap().len(), 2);
    assert!(fake.peak.load(Ordering::SeqCst) > 1);
    assert!(fake.peak.load(Ordering::SeqCst) <= harness.config.max_parallel);
    let requests = fake.requests.lock().unwrap();
    assert_eq!(requests.len(), harness.topology.parts.len() + 1);
    for request in requests
        .iter()
        .filter(|r| !r.actor.ends_with(&private_owner))
    {
        assert!(
            !serde_json::to_string(request)
                .unwrap()
                .contains("PRIVATE-OWNER-SECRET")
        );
    }
    assert_eq!(result.input_tokens, 7 * requests.len() as u64);
}

#[tokio::test]
async fn peer_messages_route_directly_with_a2a_provenance_and_tool_receipts() {
    let routes = Arc::new(Mutex::new((String::new(), String::new())));
    let routing = routes.clone();
    let fake = Fake::new(move |r| {
        let (sender, recipient) = routing.lock().unwrap().clone();
        let mut reply = answer("Ready");
        if r.actor.ends_with(&sender) && r.messages.len() == 1 {
            reply.push_call(call(
                "peer_send",
                json!({"to":recipient,"message":"Please check this boundary"}),
            ));
        }
        reply
    });
    let (_dir, mut harness) = fixture(Mode::Freudian, fake.clone()).await;
    let sender = harness.topology.parts[0].id.clone();
    let recipient = harness.topology.parts[1].id.clone();
    *routes.lock().unwrap() = (sender.clone(), recipient.clone());
    let result = harness.run("Plan a feature").await.unwrap();
    let event = result.events.iter().find(|e| e.kind == "peer").unwrap();
    let rpc: Value = serde_json::from_str(&event.detail).unwrap();
    assert_eq!(rpc["method"], "SendMessage");
    assert_eq!(rpc["params"]["message"]["metadata"]["sender"], sender);
    assert_eq!(rpc["params"]["message"]["metadata"]["recipient"], recipient);
    let requests = fake.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|r| {
        r.actor.ends_with(&recipient)
            && r.messages
                .iter()
                .any(|m| m.text_projection().contains("Please check this boundary"))
    }));
    assert!(
        harness
            .memory_for(&sender)
            .await
            .unwrap()
            .iter()
            .any(|m| m.role == "tool" && m.text_projection().contains("delivered"))
    );
}

#[tokio::test]
async fn relationships_preserve_their_own_history_without_access_to_part_notes() {
    let fake = Fake::new(|_| answer("Shared voice"));
    let (_dir, mut harness) = fixture(Mode::Freudian, fake.clone()).await;
    let members = harness.topology.parts[..2]
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    harness
        .memory
        .append(
            &format!("{}/notes", harness.namespace(&members[0])),
            "note",
            "PART-ONLY-NOTE",
        )
        .await
        .unwrap();
    let relation = harness
        .relate(RelationshipKind::Alliance, members.clone())
        .await
        .unwrap();
    harness
        .memory
        .append(
            &harness.namespace(&relation.id),
            "user",
            "RELATION-ONLY-NOTE",
        )
        .await
        .unwrap();
    let output = harness.run("Use the relationship").await.unwrap();
    assert_eq!(output.speaker, relation.id);
    assert_eq!(output.relationship.unwrap().members.len(), 2);
    harness.focus(None).await.unwrap();
    let mut reversed = members;
    reversed.reverse();
    assert_eq!(
        harness
            .relate(RelationshipKind::Alliance, reversed)
            .await
            .unwrap()
            .id,
        relation.id
    );
    assert!(
        harness
            .memory_for(&relation.id)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text_projection().contains("RELATION-ONLY-NOTE"))
    );
    let requests = fake.requests.lock().unwrap();
    let group = requests
        .iter()
        .find(|r| r.actor.ends_with(&relation.id))
        .unwrap();
    assert!(!group.instructions.contains("PART-ONLY-NOTE"));
    assert!(
        group
            .messages
            .iter()
            .any(|m| m.text_projection().contains("RELATION-ONLY-NOTE"))
    );
    for request in requests.iter().filter(|r| !r.actor.ends_with(&relation.id)) {
        assert!(
            !serde_json::to_string(request)
                .unwrap()
                .contains("RELATION-ONLY-NOTE")
        );
    }
}

#[tokio::test]
async fn tool_calls_execute_and_feed_real_outputs_back_only_to_speaker() {
    const TOOL_TOKEN: &str = "sk-proj-abcdefghijklmnop0123456789";
    const PRIOR_TOKEN: &str = "sk-proj-priorhistory0123456789";
    const ORDINARY_CONTROL: &str = "ordinary-control-remains-exact";
    const MARKER: &str = "[REDACTED:recognized-secret]";
    let resumed_phase = Arc::new(AtomicBool::new(false));
    let provider_phase = resumed_phase.clone();
    let fake = Fake::new(move |r| {
        let mut reply = answer("Draft");
        if r.instructions.contains("Phase: speak") {
            if r.messages.iter().any(|m| {
                let output = crate::test_receipt_output(m).unwrap_or_default();
                m.role == "tool"
                    && output.contains(MARKER)
                    && output.contains(ORDINARY_CONTROL)
                    && !output.contains(TOOL_TOKEN)
            }) {
                reply.set_text(format!("Read {ORDINARY_CONTROL} from actual tool"));
            } else if provider_phase.load(Ordering::SeqCst) {
                reply.set_text("The resumed context omitted its persisted tool receipt");
            } else {
                reply.push_call(call("file_read", json!({"path":"sample.txt"})));
            }
        }
        reply
    });
    let dir = tempfile::tempdir().unwrap();
    let data = kuru_memory::test_support::tempdir().unwrap();
    let config = Config {
        mode: Mode::Freudian,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    };
    let options = kuru_memory::test_support::open_options(
        data.path().to_owned(),
        crate::project_scope(dir.path()).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
        config.clone(),
        dir.path(),
        memory.clone(),
        fake.clone(),
        None,
    )
    .await
    .unwrap();
    let prior_owner = harness.topology.parts[0].id.clone();
    harness.focus(Some(&prior_owner)).await.unwrap();
    let prior_history = format!("preexisting history openai_api_key={PRIOR_TOKEN}");
    harness
        .memory
        .append(
            &harness.namespace(&prior_owner),
            "assistant",
            &prior_history,
        )
        .await
        .unwrap();
    let source = format!(
        "openai_api_key={TOOL_TOKEN}\n{}{}:TAIL",
        "x".repeat(2 * 1024 * 1024),
        ORDINARY_CONTROL,
    );
    let source_path = dir.path().join("sample.txt");
    std::fs::write(&source_path, &source).unwrap();
    let result = harness.run("Read sample.txt").await.unwrap();
    assert!(result.text.contains(ORDINARY_CONTROL));
    let tool_event = result.events.iter().find(|e| e.kind == "tool").unwrap();
    assert_eq!(tool_event.detail, "file_read");
    assert!(!tool_event.detail.contains(TOOL_TOKEN));

    let speaker = result.speaker.clone();
    assert_eq!(speaker, prior_owner);
    let session = harness.session.id.clone();
    let stored = harness.memory_for(&speaker).await.unwrap();
    let receipt = stored.iter().find(|m| m.role == "tool").unwrap();
    let receipt: Value = crate::test_receipt(receipt).unwrap();
    let call_id = receipt["call_id"].as_str().unwrap().to_owned();
    let output = receipt["output"].as_str().unwrap();
    assert!(output.len() <= 8192);
    assert!(output.starts_with(&format!("openai_api_key={MARKER}\n")));
    assert!(output.ends_with(&format!("{ORDINARY_CONTROL}:TAIL")));
    assert!(output.contains("[truncated]"));
    assert!(!output.contains(TOOL_TOKEN));
    assert_eq!(std::fs::read_to_string(&source_path).unwrap(), source);

    let requests_before_reopen = fake.requests.lock().unwrap().len();
    harness.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    drop(harness);
    let memory = MemoryStore::open(options).await.unwrap();
    let mut reopened = Harness::new(
        config,
        dir.path(),
        memory.clone(),
        fake.clone(),
        Some(&session),
    )
    .await
    .unwrap();
    reopened.focus(Some(&speaker)).await.unwrap();
    let persisted = reopened.memory_for(&speaker).await.unwrap();
    assert!(persisted.iter().any(|message| {
        message.role == "tool"
            && crate::test_receipt_output(message).is_some_and(|output| {
                output.contains(MARKER)
                    && output.contains(ORDINARY_CONTROL)
                    && !output.contains(TOOL_TOKEN)
            })
    }));
    assert!(
        reopened
            .memory_for(&prior_owner)
            .await
            .unwrap()
            .iter()
            .any(
                |message| message.role == "assistant" && message.text_projection() == prior_history
            )
    );
    resumed_phase.store(true, Ordering::SeqCst);
    let continued = reopened.run("Continue from the tool result").await.unwrap();
    assert!(continued.text.contains(ORDINARY_CONTROL));
    {
        let requests = fake.requests.lock().unwrap();
        let resumed_request = requests[requests_before_reopen..]
            .iter()
            .find(|request| request.actor.ends_with(&speaker))
            .unwrap();
        assert!(resumed_request.messages.iter().any(|message| {
            message.role == "tool"
                && crate::test_receipt(message)
                    .ok()
                    .is_some_and(|receipt| receipt["call_id"] == call_id)
                && crate::test_receipt_output(message).is_some_and(|output| {
                    output.contains(MARKER)
                        && output.contains(ORDINARY_CONTROL)
                        && !output.contains(TOOL_TOKEN)
                })
        }));
        for request in requests
            .iter()
            .filter(|request| !request.actor.ends_with(&speaker))
        {
            assert!(
                !request
                    .messages
                    .iter()
                    .any(|message| message.role == "tool"),
                "tool receipt crossed private peer boundary: {}",
                request.actor
            );
        }
    }
    assert_eq!(std::fs::read_to_string(&source_path).unwrap(), source);
    reopened.shutdown(false).await.unwrap();
    memory.close().await.unwrap();
    drop(reopened);
}

#[tokio::test]
async fn invalid_tool_calls_are_visible_to_the_model_and_cannot_change_state() {
    let fake = Fake::new(|r| {
        let mut reply = answer("Done");
        if r.messages.len() == 1 {
            reply = reply.with_calls(vec![
                call("state_report", json!({"activation":3.0,"note":"bad"})),
                call("remember", json!({"text":""})),
                call("unavailable", json!({})),
            ]);
        }
        reply
    });
    let (_dir, mut harness) = fixture(Mode::Freudian, fake).await;
    harness.run("Try invalid proposals").await.unwrap();
    assert!(harness.topology.states.is_empty());
    let history = harness
        .memory_for(&harness.topology.parts[0].id)
        .await
        .unwrap();
    assert!(
        history
            .iter()
            .any(|m| m.role == "tool" && m.text_projection().contains("ERROR"))
    );
}

#[tokio::test]
async fn modeled_state_selects_a_peer_and_stores_private_notes() {
    let target = Arc::new(Mutex::new(String::new()));
    let role = target.clone();
    let fake = Fake::new(move |r| {
        let mut reply = answer("Stateful response");
        if r.actor.ends_with(role.lock().unwrap().as_str()) && r.messages.len() == 1 {
            reply = reply.with_calls(vec![
                call(
                    "state_report",
                    json!({"activation":0.9,"note":"This task matches my concerns"}),
                ),
                call("remember", json!({"text":"durable priority"})),
            ]);
        }
        reply
    });
    let (_dir, mut harness) = fixture(Mode::Freudian, fake.clone()).await;
    let id = harness.topology.parts[1].id.clone();
    *target.lock().unwrap() = id.clone();
    let output = harness.run("Choose a perspective").await.unwrap();
    assert_eq!(output.speaker, id);
    assert_eq!(harness.topology.states[&id].activation, 0.9);
    assert!(
        fake.requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.actor.ends_with(&id) && r.instructions.contains("durable priority"))
    );
}

#[tokio::test]
async fn cyclic_peers_stop_at_peer_round_limit() {
    let roster = Arc::new(Mutex::new(Vec::<String>::new()));
    let ids = roster.clone();
    let fake = Fake::new(move |r| {
        let ids = ids.lock().unwrap();
        let target = ids
            .iter()
            .find(|id| !r.actor.ends_with(id.as_str()))
            .unwrap();
        let mut reply = answer("bounded contribution");
        if r.instructions.contains("Phase: deliberate") && !r.tools.is_empty() {
            reply.push_call(call("peer_send", json!({"to":target,"message":"again"})));
        }
        reply
    });
    let (_dir, mut harness) = fixture(Mode::Freudian, fake.clone()).await;
    *roster.lock().unwrap() = harness
        .topology
        .parts
        .iter()
        .map(|p| p.id.clone())
        .collect();
    harness.config.max_rounds = 2;
    harness.config.max_tool_calls = 100;
    let output = tokio::time::timeout(Duration::from_secs(2), harness.run("Cycle"))
        .await
        .unwrap()
        .unwrap();
    assert!(output.limited);
    assert_eq!(
        output.limit_reasons,
        Some(vec![crate::TurnLimitReason::PeerRounds])
    );
    assert_eq!(output.response_outcome, Some(crate::ResponseOutcome::Text));
    assert!(output.events.iter().any(|e| e.kind == "budget"));
    assert!(fake.requests.lock().unwrap().len() <= 10);
}

#[tokio::test]
async fn empty_response_is_separate_from_resource_limits() {
    let provider = Fake::new(|request| {
        if request.instructions.contains("Phase: deliberate") {
            answer("draft")
        } else {
            Completion::default()
        }
    });
    let (_directory, mut harness) = fixture(Mode::Freudian, provider).await;

    let output = harness.run("empty response fixture").await.unwrap();

    assert!(output.limited);
    assert_eq!(output.limit_reasons, Some(vec![]));
    assert_eq!(output.response_outcome, Some(crate::ResponseOutcome::Empty));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn tool_call_limit_is_reported_without_a_peer_round_limit() {
    let provider = Fake::new(|request| {
        if request.instructions.contains("Phase: deliberate") {
            answer("draft")
        } else {
            Completion::from_legacy(
                "answer with exhausted tool request",
                vec![call("remember", json!({"text":"not executed"}))],
                0,
                0,
            )
        }
    });
    let (_directory, mut harness) = fixture(Mode::Freudian, provider).await;
    harness.config.max_tool_calls = 0;

    let output = harness.run("tool budget fixture").await.unwrap();

    assert!(output.limited);
    assert_eq!(
        output.limit_reasons,
        Some(vec![crate::TurnLimitReason::ToolCalls])
    );
    assert_eq!(output.response_outcome, Some(crate::ResponseOutcome::Text));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn dreaming_adds_retires_and_undoes_without_losing_memories() {
    let (_dir, mut harness) = fixture(Mode::Ifs, Fake::new(|_| answer("Summary"))).await;
    let before = harness.topology.parts.iter().filter(|p| p.active).count();
    let role = harness
        .topology
        .parts
        .iter()
        .find(|p| p.role == "manager")
        .unwrap()
        .role
        .clone();
    let retired = harness
        .topology
        .parts
        .iter()
        .find(|p| p.role == role)
        .unwrap()
        .id
        .clone();
    harness
        .memory
        .append(&harness.namespace(&retired), "user", "preserve me")
        .await
        .unwrap();
    let report = harness
        .apply_dream(vec![
            DreamProposal::Add {
                name: "Planner two".into(),
                role: role.clone(),
                instruction: "Check sequencing".into(),
            },
            DreamProposal::Retire {
                id: retired.clone(),
            },
        ])
        .await
        .unwrap();
    assert_eq!(report.accepted.len(), 2);
    assert_eq!(
        harness.topology.parts.iter().filter(|p| p.active).count(),
        before
    );
    assert!(!harness.actors.contains_key(&retired));
    harness.undo_dream().await.unwrap();
    assert!(harness.actors.contains_key(&retired));
    assert!(
        harness
            .memory_for(&retired)
            .await
            .unwrap()
            .iter()
            .any(|m| m.text_projection() == "preserve me")
    );
    assert!(
        !harness
            .topology
            .parts
            .iter()
            .find(|p| p.name == "Planner two")
            .unwrap()
            .active
    );
    assert!(harness.undo_dream().await.is_err());
}

#[tokio::test]
async fn dream_additions_persist_the_equal_peer_preamble() {
    let (_dir, mut harness) = fixture(Mode::Freudian, Fake::new(|_| answer("Summary"))).await;
    let role = harness.topology.parts[0].role.clone();
    let tendency = "Compare practical alternatives without supervising the pool";

    let report = harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Alternative".into(),
            role,
            instruction: tendency.into(),
        }])
        .await
        .unwrap();

    assert_eq!(report.accepted.len(), 1);
    let added = harness
        .topology
        .parts
        .iter()
        .find(|part| part.name == "Alternative")
        .unwrap();
    assert_eq!(
        added.instruction,
        canonical_peer_instruction(tendency).unwrap(),
        "dream additions must persist the same equal-peer preamble as builtins"
    );
}

#[tokio::test]
async fn provider_free_undo_preserves_sessions_and_archives_added_identities() {
    let (_directory, mut harness) = fixture(Mode::Ifs, Fake::new(|_| answer("Summary"))).await;
    let config = harness.config.clone();
    let scope = harness.scope.clone();
    let memory = harness.memory.clone();
    let session = harness.session.id.clone();
    let role = harness.topology.parts[0].role.clone();
    harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Later observer".into(),
            role,
            instruction: "Preserve the later identity".into(),
        }])
        .await
        .unwrap();
    let added = harness.resolve("Later observer").unwrap();
    memory
        .append(
            &format!("{scope}/{}/identity/{added}", config.mode),
            "user",
            "later private conversation",
        )
        .await
        .unwrap();
    let sessions = memory.get(&format!("{scope}/sessions")).await.unwrap();
    harness.shutdown(false).await.unwrap();
    drop(harness);

    undo_dream(&config, &scope, &memory, Some(&session))
        .await
        .unwrap();

    assert_eq!(
        memory.get(&format!("{scope}/sessions")).await.unwrap(),
        sessions
    );
    assert!(
        memory
            .history(&format!("{scope}/{}/identity/{added}", config.mode), 10)
            .await
            .unwrap()
            .iter()
            .any(|message| message.text_projection() == "later private conversation")
    );
    let topology = read_topology(&memory, &scope, config.mode).await.unwrap();
    assert!(
        !topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
}

#[tokio::test]
async fn injected_host_must_match_the_canonical_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let config = Config {
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        ..Config::default()
    };
    let host = ToolHost::new(other.path(), &config).unwrap();
    let result = Harness::with_tool_host(
        config,
        workspace.path(),
        MemoryStore::temporary().await.unwrap(),
        Fake::new(|_| answer("unused")),
        None,
        host,
    )
    .await;
    let error = result.err().expect("mismatched host must be rejected");
    assert!(
        error
            .to_string()
            .contains("does not match the canonical workspace")
    );
}

#[tokio::test]
async fn dreaming_rejects_last_role_removal_unknown_roles_names_and_capacity_overflow() {
    let (_dir, mut harness) = fixture(Mode::Freudian, Fake::new(|_| answer("Summary"))).await;
    harness.config.max_parts = harness.topology.parts.len();
    let part = harness.topology.parts[0].clone();
    let proposals = vec![
        DreamProposal::Retire {
            id: part.id.clone(),
        },
        DreamProposal::Retire {
            id: "missing".into(),
        },
        DreamProposal::Add {
            name: "".into(),
            role: part.role.clone(),
            instruction: "x".into(),
        },
        DreamProposal::Add {
            name: part.name.clone(),
            role: part.role.clone(),
            instruction: "x".into(),
        },
        DreamProposal::Add {
            name: "new".into(),
            role: "invented".into(),
            instruction: "x".into(),
        },
        DreamProposal::Add {
            name: "new".into(),
            role: part.role.clone(),
            instruction: "x".into(),
        },
    ];
    let report = harness.apply_dream(proposals).await.unwrap();
    assert_eq!(report.rejected.len(), 6);
    assert!(report.accepted.is_empty());
    assert_eq!(harness.topology.parts.len(), 3);
    assert!(
        harness
            .apply_dream(vec![DreamProposal::Retire { id: part.id }; 7])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn dreaming_uses_isolated_actor_histories_and_runs_periodically() {
    let fake = Fake::new(|r| {
        if r.instructions.contains("Phase: dream") {
            answer("durable dream summary")
        } else {
            answer("response")
        }
    });
    let (_dir, mut harness) = fixture(Mode::Polyvagal, fake.clone()).await;
    harness.config.dream_every = 1;
    let mut events = harness.subscribe();
    let output = harness.run("one turn").await.unwrap();
    assert!(
        !output.events.iter().any(|event| event.kind == "dream"),
        "returned turn output freezes before maintenance dreaming"
    );
    assert_eq!(output.events.last().unwrap().kind, "response");
    assert!(
        std::iter::from_fn(|| events.try_recv().ok())
            .any(|e| e.kind == "dream" && e.detail.contains("3 summaries"))
    );
    for part in &harness.topology.parts {
        assert_eq!(
            harness
                .memory
                .history(&format!("{}/notes", harness.namespace(&part.id)), 20)
                .await
                .unwrap()[0]
                .text_projection(),
            "durable dream summary"
        );
    }
    harness.config.dream_on_exit = true;
    harness.shutdown(true).await.unwrap();
    assert!(harness.actors.is_empty());
    assert_eq!(
        fake.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.instructions.contains("Phase: dream"))
            .count(),
        6
    );
}

#[tokio::test]
async fn sessions_resume_mode_and_memory_and_projects_do_not_share_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let db = kuru_memory::test_support::tempdir().unwrap();
    let fake = Fake::new(|_| answer("persisted answer"));
    let config = Config {
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        ..Config::default()
    };
    let options = kuru_memory::test_support::open_options(
        db.path().to_owned(),
        crate::project_scope(dir.path()).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options).await.unwrap();
    let mut harness = Harness::new(
        config.clone(),
        dir.path(),
        memory.clone(),
        fake.clone(),
        None,
    )
    .await
    .unwrap();
    harness.set_mode(Mode::Jungian).await.unwrap();
    harness.run("retain this session").await.unwrap();
    let id = harness.session.id.clone();
    assert_eq!(harness.sessions().await.unwrap().len(), 1);
    drop(harness);
    let resumed = Harness::new(
        config.clone(),
        dir.path(),
        memory.clone(),
        fake.clone(),
        Some(&id),
    )
    .await
    .unwrap();
    assert_eq!(resumed.config.mode, Mode::Jungian);
    assert_eq!(resumed.session.turns, 1);
    assert!(
        resumed.history().await.unwrap()[0]
            .text_projection()
            .contains("retain")
    );
    let other = tempfile::tempdir().unwrap();
    assert!(
        Harness::new(
            config.clone(),
            other.path(),
            memory.clone(),
            fake.clone(),
            Some(&id)
        )
        .await
        .is_err()
    );
    let another = Harness::new(config, other.path(), memory, fake, None)
        .await
        .unwrap();
    assert_ne!(resumed.scope, another.scope);
}

#[tokio::test]
async fn manual_focus_and_relationship_validation_reject_invalid_topology() {
    let (_dir, mut h) = fixture(Mode::Ifs, Fake::new(|_| answer("ok"))).await;
    assert!(h.resolve("manager").is_err());
    assert!(h.resolve("missing").is_err());
    let ids = h.topology.parts[..4]
        .iter()
        .map(|p| p.id.clone())
        .collect::<Vec<_>>();
    for kind in [
        RelationshipKind::Protection,
        RelationshipKind::Polarization,
        RelationshipKind::Alliance,
    ] {
        let r = h.relate(kind, ids.clone()).await.unwrap();
        assert_eq!(h.resolve(&r.id).unwrap(), r.id);
        assert!(h.relate(kind, vec![r.id, ids[0].clone()]).await.is_err());
    }
    h.focus(Some(&ids[0])).await.unwrap();
    assert_eq!(h.run("focused").await.unwrap().speaker, ids[0]);
    h.focus(None).await.unwrap();
    assert!(h.topology.focus.is_none());
    assert!(h.run("").await.is_err());
    assert!(h.run_for("hello", Some("absent")).await.is_err());
    assert!(h.run_for("direct", Some(&ids[1])).await.unwrap().speaker == ids[1]);
    let mut invalid = h.topology.clone();
    invalid.parts.push(invalid.parts[0].clone());
    assert!(validate_topology(&invalid, &h.config).is_err());
    invalid = h.topology.clone();
    invalid.parts.clear();
    assert!(validate_topology(&invalid, &h.config).is_err());
    invalid = h.topology.clone();
    invalid.relationships[0].id = "wrong".into();
    assert!(validate_topology(&invalid, &h.config).is_err());
    assert_eq!(h.cwd().canonicalize().unwrap(), h.cwd());
    h.topology.states.insert(
        ids[0].clone(),
        StateReport {
            activation: 0.4,
            note: "state".into(),
        },
    );
    h.save().await.unwrap();
}

#[tokio::test]
async fn a2a_messages_round_trip_and_reject_empty_or_oversized_payloads() {
    let message = PeerMessage::new("a", "b", "c", "hello 世界").unwrap();
    let decoded: PeerMessage =
        serde_json::from_value(message.rpc()["params"]["message"].clone()).unwrap();
    assert_eq!(decoded.text(), "hello 世界");
    assert_eq!(decoded.metadata.recipient, "b");
    assert!(PeerMessage::new("", "b", "c", "x").is_err());
    assert!(PeerMessage::new("a", "b", "c", "").is_err());
    assert!(PeerMessage::new("a", "b", "c", &"x".repeat(32769)).is_err());
}

async fn rpc(app: axum::Router, body: Value, token: &str, version: &str) -> (u16, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .header("authorization", token)
                .header("a2a-version", version)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a2a_server_enforces_auth_validates_protocol_and_returns_peer_output() {
    let provider = Fake::new(|_| answer("real A2A answer"));
    let (_dir, h) = fixture(Mode::Freudian, provider.clone()).await;
    let shared = Arc::new(tokio::sync::Mutex::new(h));
    assert!(crate::server::router(shared.clone(), "http://localhost", "short").is_err());
    let app = crate::server::router(shared, "http://localhost", "test-token-123456").unwrap();
    let valid = json!({"jsonrpc":"2.0","id":1,"method":"SendMessage","params":{"message":{"messageId":"m1","role":"ROLE_USER","contextId":"c1","parts":[{"text":"hello"}]}}});
    assert_eq!(
        rpc(app.clone(), valid.clone(), "Bearer wrong", "1.0")
            .await
            .0,
        401
    );
    assert_eq!(
        rpc(
            app.clone(),
            valid.clone(),
            "Bearer test-token-123456",
            "0.3"
        )
        .await
        .1["error"]["code"],
        -32009
    );
    let response = rpc(
        app.clone(),
        valid.clone(),
        "Bearer test-token-123456",
        "1.0",
    )
    .await
    .1;
    assert_eq!(
        response["result"]["message"]["parts"][0]["text"],
        "real A2A answer"
    );
    assert_eq!(response["result"]["message"]["contextId"], "c1");
    let sends = provider.requests.lock().unwrap().len();
    let duplicate = rpc(
        app.clone(),
        valid.clone(),
        "Bearer test-token-123456",
        "1.0",
    )
    .await
    .1;
    assert_eq!(
        duplicate["result"]["message"]["parts"][0]["text"],
        "real A2A answer"
    );
    assert_eq!(provider.requests.lock().unwrap().len(), sends);
    let mut changed = valid.clone();
    changed["params"]["message"]["parts"][0]["text"] = json!("changed");
    let changed = rpc(app.clone(), changed, "Bearer test-token-123456", "1.0")
        .await
        .1;
    assert_eq!(changed["error"]["code"], -32603);
    assert!(
        changed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different request")
    );
    assert_eq!(provider.requests.lock().unwrap().len(), sends);
    let mut malformed = valid.clone();
    malformed["method"] = json!("GetTask");
    assert_eq!(
        rpc(app.clone(), malformed, "Bearer test-token-123456", "1.0")
            .await
            .1["error"]["code"],
        -32601
    );
    for path in ["role", "messageId", "parts"] {
        let mut malformed = valid.clone();
        malformed["params"]["message"][path] = Value::Null;
        assert_eq!(
            rpc(app.clone(), malformed, "Bearer test-token-123456", "1.0")
                .await
                .1["error"]["code"],
            -32602
        );
    }
    let mut oversized_id = valid.clone();
    oversized_id["params"]["message"]["messageId"] = json!("x".repeat(257));
    assert_eq!(
        rpc(app.clone(), oversized_id, "Bearer test-token-123456", "1.0")
            .await
            .1["error"]["code"],
        -32602
    );
    let card = app
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent-card.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let card: Value =
        serde_json::from_slice(&to_bytes(card.into_body(), 10000).await.unwrap()).unwrap();
    assert_eq!(card["supportedInterfaces"][0]["protocolVersion"], "1.0");
    assert_eq!(card["capabilities"]["streaming"], false);
}
