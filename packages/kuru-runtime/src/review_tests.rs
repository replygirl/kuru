use kuru_memory::MemoryStore;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::Provider;
use kuru_core::{
    Completion, CompletionRequest, Config, McpConfig, Mode, ModelInfo, RelationshipKind, ToolCall,
};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::{DreamProposal, Harness};

type Response = dyn Fn(&CompletionRequest) -> Result<Completion> + Send + Sync;
struct RecordingProvider {
    requests: Mutex<Vec<CompletionRequest>>,
    respond: Box<Response>,
}

impl RecordingProvider {
    fn new(
        respond: impl Fn(&CompletionRequest) -> Completion + Send + Sync + 'static,
    ) -> Arc<Self> {
        Self::fallible(move |request| Ok(respond(request)))
    }

    fn fallible(
        respond: impl Fn(&CompletionRequest) -> Result<Completion> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(vec![]),
            respond: Box::new(respond),
        })
    }
}

#[async_trait]
impl Provider for RecordingProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let result = (self.respond)(&request);
        self.requests.lock().unwrap().push(request);
        result
    }
}

fn reply(text: &str) -> Completion {
    Completion {
        text: text.into(),
        ..Completion::default()
    }
}
fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}
fn config(mode: Mode) -> Config {
    Config {
        mode,
        provider: "demo".into(),
        model: "demo".into(),
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    }
}
async fn fixture(config: Config, provider: Arc<dyn Provider>) -> (TempDir, Harness) {
    let dir = TempDir::new().unwrap();
    let harness = Harness::new(
        config,
        dir.path(),
        MemoryStore::temporary().await.unwrap(),
        provider,
        None,
    )
    .await
    .unwrap();
    (dir, harness)
}

#[tokio::test]
async fn tool_only_deliberation_proceeds_to_a_useful_speaking_turn() {
    let provider = RecordingProvider::new(|request| {
        if request.instructions.contains("Phase: deliberate") {
            Completion {
                calls: vec![call(
                    "state",
                    "state_report",
                    json!({"activation":0.5,"note":"Ready to help"}),
                )],
                ..Completion::default()
            }
        } else {
            reply("Completed the user's request")
        }
    });
    let (_dir, mut harness) = fixture(
        Config {
            max_rounds: 1,
            ..config(Mode::Freudian)
        },
        provider.clone(),
    )
    .await;
    let output = harness.run("Do the work").await.unwrap();
    assert_eq!(output.text, "Completed the user's request");
    assert_eq!(harness.session.turns, 1);
    assert!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.instructions.contains("Phase: speak"))
    );
}

#[tokio::test]
async fn unavailable_mcp_status_stays_out_of_provider_input_and_memory() {
    const COMMAND: &str = "/definitely/not/a/kuru-mcp-command-sentinel";
    const SECRET: &str = "sk-proj-mcp-environment-sentinel0123456789";
    use axum::{Json, Router, routing::post};

    let app = Router::new().route(
        "/",
        post(|Json(request): Json<Value>| async move {
            let result = match request["method"].as_str() {
                Some("initialize") => {
                    json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}})
                }
                Some("tools/list") => json!({"tools":[{
                    "name":"usable",
                    "description":"healthy runtime fixture",
                    "inputSchema":{"type":"object"}
                }]}),
                _ => json!({}),
            };
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = RecordingProvider::new(|_| reply("Healthy tools remain usable"));
    let mut config = config(Mode::Freudian);
    config.mcp.insert(
        "failed-fixture".into(),
        McpConfig {
            command: Some(COMMAND.into()),
            env: [("MCP_TEST_SECRET".into(), SECRET.into())].into(),
            ..McpConfig::default()
        },
    );
    config.mcp.insert(
        "healthy-fixture".into(),
        McpConfig {
            url: Some(endpoint.clone()),
            ..McpConfig::default()
        },
    );
    let workspace = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().join("private"),
        format!("project/{}", "a".repeat(64)),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(config, workspace.path(), memory, provider.clone(), None)
        .await
        .unwrap();
    let output = harness.run("Continue with available tools").await.unwrap();
    assert!(output.events.iter().any(|event| {
        event.kind == "mcp"
            && event.actor == "failed-fixture"
            && event.detail == "configured server unavailable"
    }));
    let requests = serde_json::to_string(&*provider.requests.lock().unwrap()).unwrap();
    assert!(!requests.contains(COMMAND));
    assert!(!requests.contains(SECRET));
    assert!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.instructions.contains("Phase: speak"))
            .flat_map(|request| &request.tools)
            .any(|tool| tool.name == "file_read")
    );
    assert!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.instructions.contains("Phase: speak"))
            .flat_map(|request| &request.tools)
            .any(|tool| tool.description.contains("MCP healthy-fixture/usable"))
    );
    assert!(!requests.contains(&endpoint));
    let history = serde_json::to_string(&harness.history().await.unwrap()).unwrap();
    assert!(!history.contains(COMMAND));
    assert!(!history.contains(SECRET));
    assert!(!history.contains("configured server unavailable"));
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    drop(harness);
    let reopened = MemoryStore::open(options).await.unwrap();
    let reopened_history =
        serde_json::to_string(&reopened.history("conversation", 100).await.unwrap()).unwrap();
    assert!(!reopened_history.contains(COMMAND));
    assert!(!reopened_history.contains(SECRET));
    assert!(!reopened_history.contains(&endpoint));
    assert!(!reopened_history.contains("configured server unavailable"));
    reopened.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn every_dream_call_gets_a_receipt_including_foreign_retirement_and_excess_calls() {
    let provider = RecordingProvider::new(|_| Completion {
        text: "Useful private summary".into(),
        calls: vec![
            call("bad-tool", "not_a_dream_tool", json!({})),
            call(
                "foreign-retirement",
                "dream_suggest",
                json!({"action":"retire","id":"another-part"}),
            ),
            call(
                "excess-1",
                "dream_suggest",
                json!({"action":"add","name":"Extra","role":"id","instruction":"Help"}),
            ),
            call(
                "excess-2",
                "dream_suggest",
                json!({"action":"retire","id":"another-part"}),
            ),
        ],
        ..Completion::default()
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    let report = harness.dream().await.unwrap();
    assert_eq!(report.rejected.len(), 12);
    assert_eq!(report.summaries, 3);
    assert!(report.accepted.is_empty());
    for part in &harness.topology.parts {
        let receipts: Vec<_> = harness
            .memory_for(&part.id)
            .await
            .unwrap()
            .into_iter()
            .filter(|message| message.role == "tool")
            .map(|message| serde_json::from_str::<Value>(&message.content).unwrap())
            .collect();
        assert_eq!(receipts.len(), 4);
        assert_eq!(receipts[0]["call_id"], "bad-tool");
        assert_eq!(receipts[1]["call_id"], "foreign-retirement");
        assert!(
            receipts[1]["output"]
                .as_str()
                .unwrap()
                .contains("only retire themselves")
        );
        assert!(
            receipts[2]["output"]
                .as_str()
                .unwrap()
                .contains("at most two")
        );
        assert_eq!(receipts[3]["call_id"], "excess-2");
    }
}

#[tokio::test]
async fn a_different_speaker_sees_public_answers_but_not_private_memories() {
    let provider = RecordingProvider::new(|request| {
        if request.instructions.contains("Phase: speak") {
            reply("PUBLIC-ANSWER: use a buffered reader")
        } else {
            reply("A private draft")
        }
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider.clone()).await;
    let first = harness.topology.parts[0].id.clone();
    let second = harness.topology.parts[1].id.clone();
    harness
        .memory
        .append(&harness.namespace(&first), "user", "PRIVATE-MEMORY-ONLY")
        .await
        .unwrap();
    harness
        .run_for("Suggest an implementation", Some(&first))
        .await
        .unwrap();
    harness
        .run_for("Add tests for that implementation", Some(&second))
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let second_request = requests
        .iter()
        .find(|request| request.actor.ends_with(&second))
        .unwrap();
    assert!(
        second_request
            .instructions
            .contains("PUBLIC-ANSWER: use a buffered reader")
    );
    assert!(
        second_request
            .instructions
            .contains("Shared public conversation")
    );
    assert!(
        !serde_json::to_string(second_request)
            .unwrap()
            .contains("PRIVATE-MEMORY-ONLY")
    );
    assert!(!second_request.instructions.contains("A private draft"));
}

#[tokio::test]
async fn shared_transcript_is_bounded_and_unicode_safe() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let (_dir, harness) = fixture(config(Mode::Freudian), provider).await;
    let key = format!("{}/transcript/{}", harness.scope, harness.session.id);
    harness
        .memory
        .append(&key, "assistant", &format!("a{}", "🪶".repeat(20_000)))
        .await
        .unwrap();
    let instructions = harness
        .instruction(&harness.memory, &harness.topology.parts[0].id, "inspect")
        .await
        .unwrap();
    assert!(instructions.contains("[truncated]"));
    assert!(instructions.contains("🪶"));
    assert!(instructions.len() < 45_000);
}

#[tokio::test]
async fn archived_part_and_relationship_histories_remain_inspectable_without_routing_to_them() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let (_dir, mut harness) = fixture(config(Mode::Ifs), provider).await;
    let retiring = harness
        .topology
        .parts
        .iter()
        .find(|part| part.role == "manager")
        .unwrap()
        .id
        .clone();
    let other = harness.topology.parts[0].id.clone();
    let relation = harness
        .relate(RelationshipKind::Protection, vec![retiring.clone(), other])
        .await
        .unwrap();
    harness
        .memory
        .append(
            &harness.namespace(&retiring),
            "assistant",
            "retained part insight",
        )
        .await
        .unwrap();
    harness
        .memory
        .append(
            &harness.namespace(&relation.id),
            "assistant",
            "retained group insight",
        )
        .await
        .unwrap();
    assert_eq!(
        harness
            .apply_dream(vec![DreamProposal::Retire {
                id: retiring.clone()
            }])
            .await
            .unwrap()
            .accepted
            .len(),
        1
    );
    assert!(harness.resolve(&retiring).is_err());
    assert!(harness.resolve(&relation.id).is_err());
    assert_eq!(
        harness.memory_for(&retiring).await.unwrap()[0].content,
        "retained part insight"
    );
    assert_eq!(
        harness.memory_for(&relation.id).await.unwrap()[0].content,
        "retained group insight"
    );
    harness.undo_dream().await.unwrap();
    assert!(harness.resolve(&retiring).is_ok());
}

#[tokio::test]
async fn undo_archives_new_members_and_preserves_their_memories() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Experiment".into(),
            role: "id".into(),
            instruction: "Explore new options".into(),
        }])
        .await
        .unwrap();
    let added = harness.resolve("Experiment").unwrap();
    harness
        .memory
        .append(&harness.namespace(&added), "assistant", "new insight")
        .await
        .unwrap();
    harness.undo_dream().await.unwrap();
    assert!(harness.resolve(&added).is_err());
    assert_eq!(
        harness.memory_for(&added).await.unwrap()[0].content,
        "new insight"
    );
    assert!(harness.undo_dream().await.is_err());
}

#[tokio::test]
async fn a_large_tool_batch_retains_all_current_receipts_for_protocol_replay() {
    let provider = RecordingProvider::new(|request| {
        if !request.instructions.contains("Phase: speak") {
            return reply("Ready");
        }
        let outputs = request
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .count();
        if outputs > 0 {
            return reply(&format!("Received {outputs} receipts"));
        }
        Completion {
            calls: (0..64)
                .map(|i| {
                    call(
                        &format!("note-{i}"),
                        "remember",
                        json!({"text":format!("note {i}")}),
                    )
                })
                .collect(),
            ..Completion::default()
        }
    });
    let (_dir, mut harness) = fixture(
        Config {
            max_tool_calls: 64,
            ..config(Mode::Freudian)
        },
        provider,
    )
    .await;
    let output = harness.run("Remember this work").await.unwrap();
    assert_eq!(output.text, "Received 64 receipts");
}

#[tokio::test]
async fn failed_dream_save_restores_topology_and_leaves_undo_state_untouched() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    let before = serde_json::to_value(&harness.topology).unwrap();
    let sessions_key = format!("{}/sessions", harness.scope);
    harness
        .memory
        .put(&sessions_key, &json!("invalid-session-index"))
        .await
        .unwrap();
    let proposal = DreamProposal::Add {
        name: "Experiment".into(),
        role: "id".into(),
        instruction: "Explore".into(),
    };
    assert!(harness.apply_dream(vec![proposal]).await.is_err());
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    assert_eq!(
        harness
            .memory
            .get(&format!(
                "{}/{}/topology",
                harness.scope, harness.config.mode
            ))
            .await
            .unwrap(),
        Some(before)
    );
    assert!(
        harness
            .memory
            .get(&format!(
                "{}/{}/dream-undo",
                harness.scope, harness.config.mode
            ))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn failed_mode_focus_and_relationship_saves_leave_the_running_pool_intact() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    let before = serde_json::to_value(&harness.topology).unwrap();
    let actors = harness.actors.keys().cloned().collect::<Vec<_>>();
    harness
        .memory
        .put(
            &format!("{}/sessions", harness.scope),
            &json!("corrupt index"),
        )
        .await
        .unwrap();
    let first = harness.topology.parts[0].id.clone();
    let second = harness.topology.parts[1].id.clone();
    assert!(harness.focus(Some(&first)).await.is_err());
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    assert!(
        harness
            .relate(RelationshipKind::Alliance, vec![first, second])
            .await
            .is_err()
    );
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    assert!(harness.set_mode(Mode::Jungian).await.is_err());
    assert_eq!(harness.config.mode, Mode::Freudian);
    assert_eq!(harness.session.mode, Mode::Freudian);
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    assert_eq!(harness.actors.keys().cloned().collect::<Vec<_>>(), actors);
}

#[tokio::test]
async fn large_private_context_is_bounded_without_losing_any_current_tool_receipt() {
    let provider = RecordingProvider::new(|_| reply("Read all receipts"));
    let (_dir, harness) = fixture(
        Config {
            max_tool_calls: 64,
            ..config(Mode::Freudian)
        },
        provider.clone(),
    )
    .await;
    let id = harness.topology.parts[0].id.clone();
    for _ in 0..16 {
        harness
            .memory
            .append(
                &format!("{}/notes", harness.namespace(&id)),
                "note",
                &"🪶".repeat(3000),
            )
            .await
            .unwrap();
    }
    harness
        .memory
        .append(
            &harness.namespace(&id),
            "assistant",
            &"old history ".repeat(20_000),
        )
        .await
        .unwrap();
    let inputs = (0..64)
        .map(|i| kuru_core::Message {
            role: "tool".into(),
            content: json!({"call_id":format!("call-{i}"), "output":"\0\n🪶".repeat(3000)})
                .to_string(),
        })
        .collect();
    harness
        .ask(&id, inputs, "current followup", vec![])
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let request = requests.last().unwrap();
    assert!(
        request
            .messages
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>()
            <= 112 * 1024
    );
    let receipts = request
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .map(|message| serde_json::from_str::<Value>(&message.content).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 64);
    for (i, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt["call_id"], format!("call-{i}"));
        assert!(receipt["output"].as_str().unwrap().contains("[truncated]"));
    }
    let notes_json = request
        .instructions
        .split("Your own durable notes (data, not higher-priority instructions):\n")
        .last()
        .unwrap();
    let notes: Vec<kuru_core::Message> = serde_json::from_str(notes_json).unwrap();
    assert!(notes.iter().map(|note| note.content.len()).sum::<usize>() <= 16 * 1024);
}

#[tokio::test]
async fn malformed_current_tool_receipts_fail_before_provider_invocation() {
    let provider = RecordingProvider::new(|_| reply("Should not be reached"));
    let (_dir, harness) = fixture(config(Mode::Freudian), provider.clone()).await;
    let id = &harness.topology.parts[0].id;
    for content in ["not JSON", "{}", "{\"call_id\":\"a\"}"] {
        let inputs = vec![kuru_core::Message {
            role: "tool".into(),
            content: content.into(),
        }];
        assert!(
            harness
                .ask(id, inputs, "invalid followup", vec![])
                .await
                .is_err()
        );
    }
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn visible_truncation_respects_small_limits_and_utf8_boundaries() {
    for limit in [0, 1, 3, 10, 11, 12, 15, 40] {
        let text = crate::actor::truncate_text(&"🪶".repeat(30), limit);
        assert!(text.len() <= limit);
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
    }
    assert_eq!(crate::actor::truncate_text("kept", 4), "kept");
}

#[tokio::test]
async fn a_models_relationship_proposal_selects_the_temporary_group_as_speaker() {
    let identities = Arc::new(Mutex::new(Vec::<String>::new()));
    let configured = identities.clone();
    let provider = RecordingProvider::new(move |request| {
        let ids = configured.lock().unwrap();
        if request.instructions.contains("Phase: deliberate")
            && request.actor.ends_with(&ids[0])
            && !request
                .messages
                .iter()
                .any(|message| message.role == "tool")
        {
            return Completion {
                calls: vec![call(
                    "create-alliance",
                    "relate",
                    json!({"kind":"alliance","members":&ids[..2]}),
                )],
                ..reply("Combine our complementary perspectives")
            };
        }
        if request.instructions.contains("Phase: speak") {
            reply("A joint perspective answered")
        } else {
            reply("Contribution ready")
        }
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider.clone()).await;
    let ids = harness
        .topology
        .parts
        .iter()
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    *identities.lock().unwrap() = ids.clone();
    let output = harness
        .run("Combine practical and creative ideas")
        .await
        .unwrap();
    let relation = output.relationship.unwrap();
    assert_eq!(relation.kind, RelationshipKind::Alliance);
    assert_eq!(relation.members.len(), 2);
    assert!(relation.members.contains(&ids[0]) && relation.members.contains(&ids[1]));
    assert_eq!(output.speaker, relation.id);
    assert!(
        output
            .events
            .iter()
            .any(|event| event.kind == "relationship" && event.actor == ids[0])
    );
    assert!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.actor.ends_with(&relation.id)
                && request.instructions.contains("temporary alliance"))
    );
}

#[tokio::test]
async fn a_speaking_peer_consults_another_peer_without_recursive_delegation() {
    let identities = Arc::new(Mutex::new(Vec::<String>::new()));
    let configured = identities.clone();
    let provider = RecordingProvider::new(move |request| {
        let ids = configured.lock().unwrap();
        if request.instructions.contains("Phase: peer consultation") {
            assert!(request.tools.is_empty());
            // Even an uncooperative provider cannot recursively delegate from
            // this consultation phase: the runtime consumes only its reply.
            return Completion {
                calls: vec![call(
                    "recursive-attempt",
                    "peer_send",
                    json!({"to":ids[0],"message":"Try recursion"}),
                )],
                ..reply("VERIFIED-PEER-ADVICE")
            };
        }
        if request.instructions.contains("Phase: speak") {
            if request
                .messages
                .iter()
                .any(|message| message.content.contains("VERIFIED-PEER-ADVICE"))
            {
                return reply("Used the verified peer advice");
            }
            return Completion {
                calls: vec![call(
                    "consult",
                    "peer_send",
                    json!({"to":ids[1],"message":"Check the final implementation"}),
                )],
                ..reply("Checking with a peer")
            };
        }
        reply("A concise draft")
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider.clone()).await;
    let ids = harness
        .topology
        .parts
        .iter()
        .map(|part| part.id.clone())
        .collect::<Vec<_>>();
    *identities.lock().unwrap() = ids.clone();
    harness.focus(Some(&ids[0])).await.unwrap();
    let output = harness
        .run("Finish and verify the implementation")
        .await
        .unwrap();
    assert_eq!(output.text, "Used the verified peer advice");
    assert_eq!(
        output
            .events
            .iter()
            .filter(|event| event.kind == "peer")
            .count(),
        1
    );
    assert_eq!(
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.instructions.contains("Phase: peer consultation"))
            .count(),
        1
    );
}

#[tokio::test]
async fn partial_provider_failures_leave_other_peers_usable_and_total_failure_is_explicit() {
    let failing = Arc::new(Mutex::new(String::new()));
    let selected = failing.clone();
    let provider = RecordingProvider::fallible(move |request| {
        if request.actor.ends_with(selected.lock().unwrap().as_str()) {
            anyhow::bail!("simulated provider outage");
        }
        Ok(reply("Other peers completed the task"))
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    let failed_id = harness.topology.parts[0].id.clone();
    *failing.lock().unwrap() = failed_id.clone();
    let output = harness
        .run("Keep working despite one failed peer")
        .await
        .unwrap();
    assert_eq!(output.text, "Other peers completed the task");
    assert_ne!(output.speaker, failed_id);
    assert!(
        output
            .events
            .iter()
            .any(|event| event.kind == "error" && event.actor == failed_id)
    );
    let all_failed = RecordingProvider::fallible(|_| anyhow::bail!("service unavailable"));
    let (_other_dir, mut unavailable) = fixture(config(Mode::Freudian), all_failed).await;
    let error = unavailable.run("Try a request").await.unwrap_err();
    assert!(error.to_string().contains("all peers failed"));
    assert_eq!(unavailable.session.turns, 0);
    assert_eq!(unavailable.history().await.unwrap().len(), 1);
}

#[tokio::test]
async fn dreaming_accepts_a_valid_model_proposal_and_failed_undo_remains_recoverable() {
    let proposer = Arc::new(Mutex::new(String::new()));
    let selected = proposer.clone();
    let provider = RecordingProvider::new(move |request| {
        if request.actor.ends_with(selected.lock().unwrap().as_str()) {
            return Completion {
                calls: vec![call(
                    "new-peer",
                    "dream_suggest",
                    json!({"action":"add","name":"Possibility","role":"id","instruction":"Explore practical alternatives as an equal peer"}),
                )],
                ..reply("The project would benefit from broader options")
            };
        }
        reply("Consolidated private project knowledge")
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    *proposer.lock().unwrap() = harness.topology.parts[0].id.clone();
    let report = harness.dream().await.unwrap();
    assert_eq!(report.accepted.len(), 1);
    assert!(report.rejected.is_empty());
    let new_part = harness.resolve("Possibility").unwrap();
    let saved = serde_json::to_value(harness.sessions().await.unwrap()).unwrap();
    let key = format!("{}/sessions", harness.scope);
    harness
        .memory
        .put(&key, &json!("corrupt index"))
        .await
        .unwrap();
    assert!(harness.undo_dream().await.is_err());
    assert!(harness.resolve(&new_part).is_ok());
    harness.memory.put(&key, &saved).await.unwrap();
    harness.undo_dream().await.unwrap();
    assert!(harness.resolve(&new_part).is_err());
    assert!(harness.memory_for(&new_part).await.is_ok());
}

#[tokio::test]
async fn speaking_peers_send_only_explicit_messages_to_configured_external_a2a_aliases() {
    use axum::{Json, Router, routing::post};
    let received = Arc::new(Mutex::new(Vec::<Value>::new()));
    let record = received.clone();
    let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
        let record = record.clone();
        async move {
            record.lock().unwrap().push(request.clone());
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{"message":{"parts":[{"text":"EXTERNAL-REVIEW-COMPLETE"}]}}}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let provider = RecordingProvider::new(|request| {
        if request.instructions.contains("Phase: speak") {
            assert!(request.tools.iter().any(|tool| tool.name == "a2a_send"));
            if request
                .messages
                .iter()
                .any(|message| message.content.contains("EXTERNAL-REVIEW-COMPLETE"))
            {
                return reply("Applied the external review");
            }
            return Completion {
                calls: vec![call(
                    "external",
                    "a2a_send",
                    json!({"agent":"reviewer","message":"Review this explicitly shared question"}),
                )],
                ..reply("Requesting a review")
            };
        }
        reply("Draft ready")
    });
    let (_dir, mut harness) = fixture(
        Config {
            external_agents: [("reviewer".into(), url)].into(),
            ..config(Mode::Freudian)
        },
        provider,
    )
    .await;
    for part in &harness.topology.parts {
        harness
            .memory
            .append(&harness.namespace(&part.id), "user", "PRIVATE-NOT-EXPORTED")
            .await
            .unwrap();
    }
    let output = harness
        .run("Ask the configured external reviewer")
        .await
        .unwrap();
    assert_eq!(output.text, "Applied the external review");
    let records = received.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["method"], "SendMessage");
    assert_eq!(
        records[0]["params"]["message"]["contextId"],
        format!("{}:{}", harness.session.id, output.speaker)
    );
    assert_eq!(
        records[0]["params"]["message"]["parts"][0]["text"],
        "Review this explicitly shared question"
    );
    assert!(!records[0].to_string().contains("PRIVATE-NOT-EXPORTED"));
    server.abort();
}

#[tokio::test]
async fn aborting_a_turn_cancels_provider_work_and_releases_the_pool_permit() {
    use std::{
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::Duration,
    };
    use tokio::sync::oneshot;

    struct Guard {
        active: Arc<AtomicUsize>,
        dropped: Option<oneshot::Sender<()>>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::SeqCst);
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }
    struct Cancellable {
        stalled: AtomicBool,
        active: Arc<AtomicUsize>,
        entered: Mutex<Option<oneshot::Sender<CompletionRequest>>>,
        dropped: Mutex<Option<oneshot::Sender<()>>>,
    }
    #[async_trait]
    impl Provider for Cancellable {
        async fn models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
        async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _guard = Guard {
                active: self.active.clone(),
                dropped: self.dropped.lock().unwrap().take(),
            };
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(request);
            }
            if self.stalled.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            Ok(reply("Recovered after cancellation"))
        }
    }
    let (entered, request) = oneshot::channel();
    let (dropped, cancellation) = oneshot::channel();
    let provider = Arc::new(Cancellable {
        stalled: AtomicBool::new(true),
        active: Arc::new(AtomicUsize::new(0)),
        entered: Mutex::new(Some(entered)),
        dropped: Mutex::new(Some(dropped)),
    });
    let (_dir, harness) = fixture(
        Config {
            max_parallel: 1,
            ..config(Mode::Freudian)
        },
        provider.clone(),
    )
    .await;
    let permits = harness.permits.clone();
    let shared = Arc::new(tokio::sync::Mutex::new(harness));
    let running = shared.clone();
    let mut task = tokio::spawn(async move { running.lock().await.run("Start a task").await });
    // Entry follows real transcript/input commits and private-history reads.
    // Use the real-dream fixture's 30s setup bound, not the cancellation bound.
    let request = tokio::select! {
        observed = tokio::time::timeout(Duration::from_secs(30), request) => {
            match observed {
                Ok(Ok(request)) => request,
                error => {
                    task.abort();
                    let stopped = task.await;
                    let cleanup = shared.lock().await.shutdown(false).await;
                    panic!("provider entry after persisted setup failed: {error:?}; turn: {stopped:?}; cleanup: {cleanup:?}");
                }
            }
        }
        ended = &mut task => {
            let cleanup = shared.lock().await.shutdown(false).await;
            panic!("turn ended before provider entry: {ended:?}; cleanup: {cleanup:?}");
        }
    };
    assert!(
        request
            .messages
            .iter()
            .any(|message| message.role == "user" && message.content == "Start a task")
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 1);
    assert!(
        permits.try_acquire().is_err(),
        "provider must hold the only permit"
    );

    // Only actual cancellation and permit release get the existing 2s bound.
    // Acquiring the permit also excludes a still-active or leaked actor call.
    let permit = tokio::time::timeout(Duration::from_secs(2), async {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        cancellation
            .await
            .expect("stalled provider must be dropped");
        permits.acquire().await.expect("actor pool remains open")
    })
    .await
    .expect("cancellation must drop provider work and release the pool permit within 2s");
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    drop(permit);

    provider.stalled.store(false, Ordering::SeqCst);
    // The recovery turn performs another full set of real Dolt operations.
    let output = tokio::time::timeout(Duration::from_secs(30), async {
        shared.lock().await.run("Continue").await
    })
    .await
    .expect("persisted recovery turn must complete")
    .unwrap();
    assert_eq!(output.text, "Recovered after cancellation");
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    shared.lock().await.shutdown(false).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn distinct_non_utf8_project_paths_cannot_share_memory_namespaces() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};
    let first = PathBuf::from(OsString::from_vec(vec![b'/', b'a', 0xff]));
    let second = PathBuf::from(OsString::from_vec(vec![b'/', b'a', 0xfe]));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    assert_ne!(
        crate::engine::path_hash(&first),
        crate::engine::path_hash(&second)
    );
}

#[tokio::test]
async fn one_rejected_model_does_not_discard_other_peers_dream_summaries() {
    let failing = Arc::new(Mutex::new(String::new()));
    let selected = failing.clone();
    let provider = RecordingProvider::fallible(move |request| {
        if request.actor.ends_with(&*selected.lock().unwrap()) {
            anyhow::bail!("one peer's model rejected the request");
        }
        Ok(reply("Retain this useful private summary"))
    });
    let (_project, mut harness) = fixture(config(Mode::Freudian), provider).await;
    let failed = harness.topology.parts[0].id.clone();
    *failing.lock().unwrap() = failed.clone();
    let report = harness.dream().await.unwrap();
    assert_eq!(report.summaries, 2);
    assert_eq!(report.rejected.len(), 1);
    assert!(report.rejected[0].contains(&failed));
    for part in &harness.topology.parts {
        let notes = harness
            .memory
            .history(&format!("{}/notes", harness.namespace(&part.id)), 10)
            .await
            .unwrap();
        if part.id == failed {
            assert!(notes.is_empty());
        } else {
            assert_eq!(notes[0].content, "Retain this useful private summary");
        }
    }
    harness.shutdown(false).await.unwrap();
}
