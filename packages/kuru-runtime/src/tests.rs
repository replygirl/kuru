use kuru_memory::MemoryStore;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use kuru_connectors::Provider;
use kuru_core::{
    Completion, CompletionRequest, Config, Mode, ModelInfo, RelationshipKind, ToolCall,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::{
    DreamProposal, Harness, PeerMessage,
    engine::{StateReport, validate_topology},
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
    Completion {
        text: text.into(),
        calls: vec![],
        input_tokens: 7,
        output_tokens: 3,
    }
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
            reply.calls.push(call(
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
                .any(|m| m.content.contains("Please check this boundary"))
    }));
    assert!(
        harness
            .memory_for(&sender)
            .await
            .unwrap()
            .iter()
            .any(|m| m.role == "tool" && m.content.contains("delivered"))
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
            .any(|m| m.content.contains("RELATION-ONLY-NOTE"))
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
            .any(|m| m.content.contains("RELATION-ONLY-NOTE"))
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
    let fake = Fake::new(|r| {
        let mut reply = answer("Draft");
        if r.instructions.contains("Phase: speak") {
            if r.messages
                .iter()
                .any(|m| m.role == "tool" && m.content.contains("unique-file-content"))
            {
                reply.text = "Read unique-file-content from actual tool".into();
            } else {
                reply
                    .calls
                    .push(call("file_read", json!({"path":"sample.txt"})));
            }
        }
        reply
    });
    let (dir, mut harness) = fixture(Mode::Freudian, fake.clone()).await;
    std::fs::write(dir.path().join("sample.txt"), "unique-file-content").unwrap();
    let result = harness.run("Read sample.txt").await.unwrap();
    assert!(result.text.contains("unique-file-content"));
    let tool_event = result.events.iter().find(|e| e.kind == "tool").unwrap();
    assert_eq!(tool_event.detail, "file_read");
    assert!(fake.requests.lock().unwrap().iter().any(|r| {
        r.messages
            .iter()
            .any(|m| m.role == "tool" && m.content.contains("unique-file-content"))
    }));
}

#[tokio::test]
async fn invalid_tool_calls_are_visible_to_the_model_and_cannot_change_state() {
    let fake = Fake::new(|r| {
        let mut reply = answer("Done");
        if r.messages.len() == 1 {
            reply.calls = vec![
                call("state_report", json!({"activation":3.0,"note":"bad"})),
                call("remember", json!({"text":""})),
                call("unavailable", json!({})),
            ];
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
            .any(|m| m.role == "tool" && m.content.contains("ERROR"))
    );
}

#[tokio::test]
async fn modeled_state_selects_a_peer_and_stores_private_notes() {
    let target = Arc::new(Mutex::new(String::new()));
    let role = target.clone();
    let fake = Fake::new(move |r| {
        let mut reply = answer("Stateful response");
        if r.actor.ends_with(role.lock().unwrap().as_str()) && r.messages.len() == 1 {
            reply.calls = vec![
                call(
                    "state_report",
                    json!({"activation":0.9,"note":"This task matches my concerns"}),
                ),
                call("remember", json!({"text":"durable priority"})),
            ];
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
async fn cyclic_peers_stop_at_round_and_call_limits() {
    let roster = Arc::new(Mutex::new(Vec::<String>::new()));
    let ids = roster.clone();
    let fake = Fake::new(move |r| {
        let ids = ids.lock().unwrap();
        let target = ids
            .iter()
            .find(|id| !r.actor.ends_with(id.as_str()))
            .unwrap();
        let mut reply = answer("bounded contribution");
        if !r.tools.is_empty() {
            reply
                .calls
                .push(call("peer_send", json!({"to":target,"message":"again"})));
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
    harness.config.max_tool_calls = 4;
    let output = tokio::time::timeout(Duration::from_secs(2), harness.run("Cycle"))
        .await
        .unwrap()
        .unwrap();
    assert!(output.limited);
    assert!(output.events.iter().any(|e| e.kind == "budget"));
    assert!(fake.requests.lock().unwrap().len() <= 10);
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
            .any(|m| m.content == "preserve me")
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
    let output = harness.run("one turn").await.unwrap();
    assert!(
        output
            .events
            .iter()
            .any(|e| e.kind == "dream" && e.detail.contains("3 summaries"))
    );
    for part in &harness.topology.parts {
        assert_eq!(
            harness
                .memory
                .history(&format!("{}/notes", harness.namespace(&part.id)), 20)
                .await
                .unwrap()[0]
                .content,
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
    let db = tempfile::tempdir().unwrap();
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
            .content
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
    let (_dir, h) = fixture(Mode::Freudian, Fake::new(|_| answer("real A2A answer"))).await;
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
