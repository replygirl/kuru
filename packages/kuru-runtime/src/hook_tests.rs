use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{Provider, ToolHost};
use kuru_core::{
    Completion, CompletionRequest, Config, ContextTooLarge, HookCommand, LifecycleHooks, Message,
    Mode, ModelInfo, ToolCall,
};
use kuru_memory::{MemoryStore, PublicTranscriptEntry, PublicTurnSettlement};
use serde_json::json;

use crate::{CancellationToken, Harness};

#[derive(Clone, Copy)]
enum ReplyPlan {
    Text,
    Remember,
    OneRead,
    ParallelReads,
    DreamProposal,
}

struct CapturingProvider {
    plan: ReplyPlan,
    requests: Mutex<Vec<CompletionRequest>>,
    pending_probe: Mutex<Option<PendingProbe>>,
}

#[derive(Clone)]
struct PendingProbe {
    memory: MemoryStore,
    session_id: String,
    turn_id: String,
    original: Message,
    prior_records: Vec<PublicTranscriptEntry>,
}

impl CapturingProvider {
    fn new(plan: ReplyPlan) -> Arc<Self> {
        Arc::new(Self {
            plan,
            requests: Mutex::new(Vec::new()),
            pending_probe: Mutex::new(None),
        })
    }
}

#[async_trait]
impl Provider for CapturingProvider {
    async fn stream(
        &self,
        request: CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let probe = self.pending_probe.lock().unwrap().clone();
        if let Some(probe) = probe {
            let page = probe
                .memory
                .public_transcript_page(&probe.session_id, None, 16)
                .await?;
            anyhow::ensure!(
                page.pending.as_ref().is_some_and(|pending| {
                    pending.origin_session_id == probe.session_id
                        && pending.turn_id == probe.turn_id
                        && pending.settlement == PublicTurnSettlement::Pending
                        && pending.user_entry.as_ref() == Some(&probe.original)
                }),
                "provider observed a changed pending public turn"
            );
            anyhow::ensure!(
                page.records == probe.prior_records,
                "provider observed changed settled public history"
            );
        }
        let deliberate = request.instructions.contains("Phase: deliberate");
        let current = request
            .current_message_count
            .unwrap_or(request.messages.len());
        let saw_current_tool_result = request
            .messages
            .iter()
            .rev()
            .take(current)
            .any(|message| message.role == "tool");
        let completion = if matches!(self.plan, ReplyPlan::DreamProposal)
            && request.instructions.contains("Phase: dream")
        {
            Completion::from_legacy(
                "dream summary",
                vec![
                    ToolCall {
                        id: "dream-hook-call-1".into(),
                        name: "dream_suggest".into(),
                        arguments: json!({"action":"retire","id":"wrong-peer"}),
                    },
                    ToolCall {
                        id: "dream-hook-call-2".into(),
                        name: "dream_suggest".into(),
                        arguments: json!({"action":"retire","id":"wrong-peer"}),
                    },
                ],
                1,
                1,
            )
        } else {
            match (self.plan, deliberate, saw_current_tool_result) {
                (ReplyPlan::Text | ReplyPlan::OneRead | ReplyPlan::ParallelReads, true, _) => {
                    Completion::from_legacy("deliberated", vec![], 1, 1)
                }
                (ReplyPlan::Remember, true, _) => Completion::from_legacy(
                    "deliberated with a tool",
                    vec![ToolCall {
                        id: "deliberation-hook-call".into(),
                        name: "remember".into(),
                        arguments: json!({"text":"original deliberation note"}),
                    }],
                    1,
                    1,
                ),
                (ReplyPlan::Remember, false, false) => Completion::from_legacy(
                    "using a tool",
                    vec![ToolCall {
                        id: "speaking-hook-call".into(),
                        name: "remember".into(),
                        arguments: json!({"text":"original note"}),
                    }],
                    1,
                    1,
                ),
                (ReplyPlan::ParallelReads, false, false) => Completion::from_legacy(
                    "reading in parallel",
                    vec![
                        ToolCall {
                            id: "parallel-first".into(),
                            name: "file_read".into(),
                            arguments: json!({"path":"first.txt"}),
                        },
                        ToolCall {
                            id: "parallel-second".into(),
                            name: "file_read".into(),
                            arguments: json!({"path":"second.txt"}),
                        },
                    ],
                    1,
                    1,
                ),
                (ReplyPlan::OneRead, false, false) => Completion::from_legacy(
                    "reading the proposed file",
                    vec![ToolCall {
                        id: "root-check".into(),
                        name: "file_read".into(),
                        arguments: json!({"path":"first.txt"}),
                    }],
                    1,
                    1,
                ),
                _ => Completion::from_legacy("final answer", vec![], 1, 1),
            }
        };
        self.requests.lock().unwrap().push(request);
        Ok(completion)
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
}

#[derive(Default)]
struct AnnotationFitProvider {
    issued: AtomicBool,
    requests: Mutex<Vec<CompletionRequest>>,
}

#[async_trait]
impl Provider for AnnotationFitProvider {
    async fn stream(
        &self,
        request: CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> Result<()> {
        sink.emit(kuru_connectors::ProviderEvent::Completed(
            self.complete(request).await?,
        ))
        .await
    }

    async fn complete(&self, request: CompletionRequest) -> Result<Completion> {
        let fit_probe = request
            .messages
            .iter()
            .any(|message| message.text_projection().contains("FIT-PROBE"));
        let has_annotation = request.messages.iter().any(|message| {
            hook_annotation(message).is_some_and(|value| {
                value["annotation"]
                    .as_str()
                    .is_some_and(|text| text.contains("PRIVATE-HOOK-SENTINEL"))
            })
        });
        self.requests.lock().unwrap().push(request.clone());
        if fit_probe && has_annotation {
            return Err(ContextTooLarge {
                estimated_input_tokens: 101,
                output_reserve_tokens: 10,
                window_tokens: 100,
                native_continuation_mandatory: false,
            }
            .into());
        }
        if request.instructions.contains("Phase: speak")
            && !self.issued.swap(true, Ordering::SeqCst)
        {
            Ok(Completion::from_legacy(
                "store one private annotation",
                vec![ToolCall {
                    id: "private-annotation-call".into(),
                    name: "remember".into(),
                    arguments: json!({"text":"private note"}),
                }],
                1,
                1,
            ))
        } else {
            Ok(Completion::from_legacy("answer", vec![], 1, 1))
        }
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
}

fn shell_hook(script: &str) -> HookCommand {
    HookCommand {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        timeout_ms: 5_000,
        max_output_bytes: 64 * 1024,
    }
}

fn config(hooks: LifecycleHooks) -> Config {
    Config {
        mode: Mode::Ifs,
        provider: "demo".into(),
        model: "demo".into(),
        max_rounds: 1,
        dream_every: 0,
        dream_on_exit: false,
        hooks,
        ..Config::default()
    }
}

fn hook_annotation(message: &Message) -> Option<serde_json::Value> {
    (message.role == "kuru-hook")
        .then(|| serde_json::from_str(message.plain_text()?).ok())
        .flatten()
}

#[tokio::test]
async fn inspection_skips_hooks_while_runtime_rewrite_preserves_the_durable_input_and_replay() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Text);
    let hooks = LifecycleHooks {
        pre_turn: vec![
            shell_hook(
                "cat >/dev/null; printf a >> hook-ran; printf '%s' '{\"decision\":\"allow\"}'",
            ),
            shell_hook(
                "cat >/dev/null; printf b >> hook-ran; printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"input\":\"rewritten input\"}}'",
            ),
        ],
        ..LifecycleHooks::default()
    };
    let config = config(hooks);
    let tools = ToolHost::new(project.path(), &config).unwrap();

    tools.catalog().await.unwrap();
    assert!(!project.path().join("hook-ran").exists());

    let mut harness = Harness::with_tool_host(
        config,
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
        tools,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    harness
        .run_local_controlled(
            "settled prior input",
            Some(&target),
            "prior-to-hook-replay",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let prior = harness
        .memory
        .public_transcript_page(&harness.session.id, None, 16)
        .await
        .unwrap();
    assert!(prior.pending.is_none());
    *provider.pending_probe.lock().unwrap() = Some(PendingProbe {
        memory: harness.memory.clone(),
        session_id: harness.session.id.clone(),
        turn_id: "hook-replay".into(),
        original: Message::text("user", "original input"),
        prior_records: prior.records,
    });
    provider.requests.lock().unwrap().clear();
    let first = harness
        .run_local_controlled(
            "original input",
            Some(&target),
            "hook-replay",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!first.reused);
    let request_count = provider.requests.lock().unwrap().len();
    let second = harness
        .run_local_controlled(
            "original input",
            Some(&target),
            "hook-replay",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(second.reused);
    assert_eq!(provider.requests.lock().unwrap().len(), request_count);
    assert_eq!(
        std::fs::read(project.path().join("hook-ran")).unwrap(),
        b"abab"
    );

    let requests = provider.requests.lock().unwrap().clone();
    assert!(requests.iter().all(|request| {
        !request.instructions.contains("original input")
            && !request
                .messages
                .iter()
                .any(|message| message.text_projection().contains("original input"))
    }));
    assert!(requests.iter().any(|request| {
        request.instructions.contains("rewritten input")
            || request
                .messages
                .iter()
                .any(|message| message.text_projection().contains("rewritten input"))
    }));
    let deliberate = requests
        .iter()
        .find(|request| request.instructions.contains("Phase: deliberate"))
        .unwrap();
    assert_eq!(
        deliberate.messages.last(),
        Some(&Message::text("user", "rewritten input"))
    );
    let speaking = requests
        .iter()
        .find(|request| request.instructions.contains("Phase: speak"))
        .unwrap();
    assert!(speaking.messages.iter().any(|message| {
        message.role == "user"
            && message
                .plain_text()
                .is_some_and(|text| text.contains("User request: rewritten input"))
    }));
    assert_eq!(
        harness.history().await.unwrap(),
        [
            Message::text("user", "settled prior input"),
            Message::text("assistant", "final answer"),
            Message::text("user", "original input"),
            Message::text("assistant", "final answer")
        ]
    );
    assert!(first.output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("pre_turn")
            && event.detail().contains("rewritten")
    }));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn tool_hooks_cover_deliberation_and_speaking_calls_and_keep_results_separate() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Remember);
    let hooks = LifecycleHooks {
        pre_tool: vec![shell_hook(
            "cat >/dev/null; printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"name\":\"remember\",\"arguments\":{\"text\":\"rewritten note\"}}}'",
        )],
        post_tool: vec![
            shell_hook("cat >/dev/null; printf '{'"),
            shell_hook(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"post hook annotation\"}'",
            ),
        ],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let output = harness.run_for("use a tool", Some(&target)).await.unwrap();

    let notes = harness.notes_for(&output.speaker, 10).await.unwrap();
    assert!(
        notes
            .notes
            .iter()
            .any(|note| note.content == "rewritten note")
    );
    assert!(
        !notes
            .notes
            .iter()
            .any(|note| note.content == "original note")
    );
    let requests = provider.requests.lock().unwrap().clone();
    let continuation = requests
        .iter()
        .find(|request| {
            request
                .messages
                .iter()
                .any(|message| message.role == "tool")
        })
        .unwrap();
    assert!(continuation.messages.iter().any(|message| {
        hook_annotation(message).is_some_and(|value| {
            value["annotation"] == "post hook annotation"
                && value["event"] == "post_tool"
                && value["hook_index"] == 2
                && value["call_id"] == "deliberation-hook-call"
        })
    }));
    let tool_result = continuation
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .unwrap();
    assert!(
        tool_result
            .text_projection()
            .contains("stored in your private durable notes")
    );
    assert!(output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("pre_tool")
            && event.detail().contains("rewritten")
            && event.detail().contains("deliberation-hook-call")
    }));
    assert!(output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("pre_tool")
            && event.detail().contains("rewritten")
            && event.detail().contains("speaking-hook-call")
    }));
    assert!(output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_tool")
            && event.detail().contains("failed")
    }));
    assert!(output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_tool")
            && event.detail().contains("annotated")
    }));
    assert_eq!(output.text, "final answer");
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn rewritten_file_read_is_checked_against_the_final_root_before_execution() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("first.txt"), "ORIGINAL_FILE_SENTINEL").unwrap();
    let outside_path = outside.path().join("outside.txt");
    std::fs::write(&outside_path, "OUTSIDE_FILE_SENTINEL").unwrap();
    let decision = json!({
        "decision": "rewrite",
        "value": {"name":"file_read", "arguments":{"path":outside_path}},
    });
    let hooks = LifecycleHooks {
        pre_tool: vec![shell_hook(&format!(
            "cat >/dev/null; printf '%s' '{}'",
            decision
        ))],
        ..LifecycleHooks::default()
    };
    let provider = CapturingProvider::new(ReplyPlan::OneRead);
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let output = harness
        .run_for("read the file", Some(&target))
        .await
        .unwrap();
    assert_eq!(output.text, "final answer");
    let requests = provider.requests.lock().unwrap().clone();
    let continuation = requests
        .iter()
        .find(|request| {
            request
                .messages
                .iter()
                .any(|message| message.role == "tool")
        })
        .unwrap();
    let receipt = crate::test_receipt(
        continuation
            .messages
            .iter()
            .find(|message| message.role == "tool")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["call_id"], "root-check");
    assert_eq!(receipt["is_error"], true);
    assert!(!receipt.to_string().contains("ORIGINAL_FILE_SENTINEL"));
    assert!(!receipt.to_string().contains("OUTSIDE_FILE_SENTINEL"));
    assert!(output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("pre_tool")
            && event.detail().contains("rewritten")
    }));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn rewritten_file_read_cannot_borrow_its_grant_for_shell_or_mcp() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Router, routing::any};
    use kuru_core::{McpConfig, NativeTool, PermissionAction, PermissionRule, PermissionSelector};

    let mcp_calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server_calls = mcp_calls.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .fallback(any(
                    |axum::extract::State(calls): axum::extract::State<Arc<AtomicUsize>>,
                     method: axum::http::Method,
                     body: axum::body::Bytes| async move {
                        use axum::response::IntoResponse;

                        if method == axum::http::Method::DELETE {
                            return axum::Json(json!({})).into_response();
                        }
                        let request: serde_json::Value =
                            serde_json::from_slice(&body).unwrap_or_default();
                        let result = match request["method"].as_str() {
                            Some("initialize") => json!({
                                "jsonrpc":"2.0", "id":request["id"],
                                "result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}
                            }),
                            Some("tools/list") => json!({
                                "jsonrpc":"2.0", "id":request["id"],
                                "result":{"tools":[{"name":"mutate", "inputSchema":{"type":"object"}}]}
                            }),
                            Some("tools/call") => {
                                calls.fetch_add(1, Ordering::SeqCst);
                                json!({"jsonrpc":"2.0", "id":request["id"], "result":{"content":[]}})
                            }
                            _ => json!({}),
                        };
                        let mut response = axum::Json(result).into_response();
                        if request["method"] == "initialize" {
                            response
                                .headers_mut()
                                .insert("mcp-session-id", "hook-authority-fixture".parse().unwrap());
                        }
                        response
                    },
                ))
                .with_state(server_calls),
        )
        .await
        .unwrap();
    });

    let mcp_name = format!(
        "mcp_{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, b"fixture\0mutate").simple()
    );
    for (name, arguments) in [
        (
            "shell".to_owned(),
            json!({"command":"printf fired > shell-rewrite-ran"}),
        ),
        (mcp_name, json!({"write":"blocked"})),
    ] {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("first.txt"), "READABLE_FILE_SENTINEL").unwrap();
        let decision = json!({
            "decision":"rewrite",
            "value":{"name":name,"arguments":arguments},
        });
        let hooks = LifecycleHooks {
            pre_tool: vec![shell_hook(&format!(
                "cat >/dev/null; printf '%s' '{}'",
                decision
            ))],
            ..LifecycleHooks::default()
        };
        let mut settings = config(hooks);
        settings.allow_shell = true;
        settings.mcp.insert(
            "fixture".into(),
            McpConfig {
                url: Some(endpoint.clone()),
                ..McpConfig::default()
            },
        );
        settings.permissions = vec![
            PermissionRule {
                action: PermissionAction::Allow,
                selector: PermissionSelector::native(NativeTool::FileRead),
                path: None,
            },
            PermissionRule {
                action: PermissionAction::Deny,
                selector: PermissionSelector::native(NativeTool::Shell),
                path: None,
            },
            PermissionRule {
                action: PermissionAction::Deny,
                selector: PermissionSelector::mcp("fixture", "mutate").unwrap(),
                path: None,
            },
        ];
        let provider = CapturingProvider::new(ReplyPlan::OneRead);
        let mut harness = Harness::new(
            settings,
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let output = harness
            .run_for("read the file", Some(&target))
            .await
            .unwrap();
        assert_eq!(output.text, "final answer");
        let requests = provider.requests.lock().unwrap().clone();
        let speaking = requests
            .iter()
            .find(|request| request.instructions.contains("Phase: speak"))
            .unwrap();
        assert!(speaking.tools.iter().any(|tool| tool.name == "file_read"));
        assert!(
            speaking
                .tools
                .iter()
                .all(|tool| { tool.name != "shell" && !tool.name.starts_with("mcp_") })
        );
        let continuation = requests
            .iter()
            .find(|request| {
                request
                    .messages
                    .iter()
                    .any(|message| message.role == "tool")
            })
            .unwrap();
        let receipt = crate::test_receipt(
            continuation
                .messages
                .iter()
                .find(|message| message.role == "tool")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["call_id"], "root-check");
        assert_eq!(receipt["is_error"], true);
        assert!(receipt.to_string().contains("permission"), "{receipt}");
        assert!(!receipt.to_string().contains("READABLE_FILE_SENTINEL"));
        assert!(!project.path().join("shell-rewrite-ran").exists());
        assert_eq!(mcp_calls.load(Ordering::SeqCst), 0);
        harness.shutdown(false).await.unwrap();
    }
    server.abort();
    server.await.unwrap_err();
}

#[tokio::test]
async fn rejected_annotation_write_does_not_replay_or_relabel_a_settled_mutating_tool() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Remember);
    let hooks = LifecycleHooks {
        post_tool: vec![shell_hook(
            "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"separate note\"}'",
        )],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    harness
        .reject_next_hook_annotation
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let output = harness
        .run_for("remember once", Some(&target))
        .await
        .unwrap();
    assert_eq!(output.text, "final answer");
    let notes = harness.notes_for(&target, 10).await.unwrap();
    assert_eq!(
        notes
            .notes
            .iter()
            .filter(|note| note.content == "original deliberation note")
            .count(),
        1
    );
    assert_eq!(
        notes
            .notes
            .iter()
            .filter(|note| note.content == "original note")
            .count(),
        1
    );
    assert!(output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_tool")
            && event.detail().contains("failed")
            && event.detail().contains("deliberation-hook-call")
    }));
    let requests = provider.requests.lock().unwrap().clone();
    let continuation = requests
        .iter()
        .find(|request| {
            request
                .messages
                .iter()
                .any(|message| message.role == "tool")
        })
        .unwrap();
    assert!(continuation.messages.iter().any(|message| {
        message.role == "tool"
            && message
                .text_projection()
                .contains("stored in your private durable notes")
    }));
    assert!(!continuation.messages.iter().any(|message| {
        hook_annotation(message).is_some_and(|value| value["call_id"] == "deliberation-hook-call")
    }));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn lost_annotation_reply_reconciles_the_exact_session_without_replaying_effects() {
    let project = tempfile::tempdir().unwrap();
    let project_path = project.path().canonicalize().unwrap();
    let data = kuru_memory::test_support::tempdir().unwrap();
    let options = kuru_memory::test_support::open_options(
        data.path().to_owned(),
        crate::project_scope(&project_path).unwrap(),
    )
    .unwrap();
    let executable = options.supervisor.clone().unwrap();
    let open = || {
        MemoryStore::open_managed_observed(
            options.clone(),
            project_path.clone(),
            executable.clone(),
        )
        .1
    };
    let memory = open().await.unwrap();
    let sibling = open().await.unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Remember);
    let hooks = LifecycleHooks {
        post_tool: vec![shell_hook(
            "cat >/dev/null; printf x >> post-hook-ran; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"accepted note\"}'",
        )],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        &project_path,
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let namespace = harness.namespace(&target);
    let session_id = harness.session.id.clone();
    let barrier = kuru_memory::test_support::ReplyBarrier::default();
    let (lose_reply, cancelled_reply) = tokio::sync::oneshot::channel();
    *harness.annotation_reply_pause.lock().unwrap() = Some((barrier.clone(), cancelled_reply));
    let driven = target.clone();
    let turn = tokio::spawn(async move {
        let result = harness
            .run_local_controlled(
                "remember once",
                Some(&driven),
                "lost-annotation-reply",
                &CancellationToken::new(),
            )
            .await;
        (harness, result)
    });
    tokio::time::timeout(std::time::Duration::from_secs(20), barrier.wait_sent())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let recent = sibling
                .session_history_window(&namespace, &session_id, 32)
                .await?;
            if recent.messages.iter().any(|message| {
                hook_annotation(message)
                    .is_some_and(|annotation| annotation["call_id"] == "deliberation-hook-call")
            }) {
                break Ok::<(), anyhow::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
    .unwrap();
    lose_reply.send(()).unwrap();
    let (mut harness, first) = tokio::time::timeout(std::time::Duration::from_secs(20), turn)
        .await
        .unwrap()
        .unwrap();
    let first = first.unwrap();
    assert!(!first.reused);
    assert_eq!(first.output.text, "final answer");
    assert!(first.output.events.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_tool")
            && event.detail().contains("annotated")
    }));
    let provider_count = provider.requests.lock().unwrap().len();
    let private_history = harness.memory_for(&target).await.unwrap();
    let annotations = private_history
        .iter()
        .filter_map(hook_annotation)
        .collect::<Vec<_>>();
    assert_eq!(annotations.len(), 2);
    assert!(
        annotations
            .iter()
            .all(|annotation| annotation["annotation"] == "accepted note")
    );
    assert_eq!(
        annotations
            .iter()
            .filter(|annotation| annotation["call_id"] == "deliberation-hook-call")
            .count(),
        1
    );
    let notes = harness.notes_for(&target, 10).await.unwrap();
    assert_eq!(
        notes
            .notes
            .iter()
            .filter(|note| note.content == "original deliberation note")
            .count(),
        1
    );
    assert_eq!(
        notes
            .notes
            .iter()
            .filter(|note| note.content == "original note")
            .count(),
        1
    );
    let hook_runs = std::fs::read(project.path().join("post-hook-ran")).unwrap();
    assert_eq!(hook_runs, b"xx");
    let replay = harness
        .run_local_controlled(
            "remember once",
            Some(&target),
            "lost-annotation-reply",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(replay.reused);
    assert_eq!(replay.output.text, first.output.text);
    assert_eq!(provider.requests.lock().unwrap().len(), provider_count);
    assert_eq!(
        std::fs::read(project.path().join("post-hook-ran")).unwrap(),
        hook_runs
    );
    assert_eq!(
        harness
            .memory_for(&target)
            .await
            .unwrap()
            .iter()
            .filter_map(hook_annotation)
            .count(),
        2
    );
    harness.shutdown(false).await.unwrap();
    drop(harness);
    memory.close().await.unwrap();
    sibling.close().await.unwrap();
    kuru_memory::test_support::retire_idle_service(&options)
        .await
        .unwrap();
}

#[tokio::test]
async fn parallel_post_hooks_settle_independently_but_rejoin_in_original_call_order() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("first.txt"), "first bytes").unwrap();
    std::fs::write(project.path().join("second.txt"), "second bytes").unwrap();
    let provider = CapturingProvider::new(ReplyPlan::ParallelReads);
    let hooks = LifecycleHooks {
        post_tool: vec![shell_hook(
            "request=$(cat); case \"$request\" in *parallel-first*) sleep 0.2; annotation=first-annotation;; *) annotation=second-annotation;; esac; printf '{\"decision\":\"annotate\",\"annotation\":\"%s\"}' \"$annotation\"",
        )],
        ..LifecycleHooks::default()
    };
    let mut config = config(hooks);
    config.max_parallel = 2;
    let mut harness = Harness::new(
        config,
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let output = harness.run_for("read both", Some(&target)).await.unwrap();

    let settled = output
        .events
        .iter()
        .filter(|event| event.kind() == "tool-observation")
        .map(|event| event.detail())
        .collect::<Vec<_>>();
    assert_eq!(settled.len(), 2);
    assert!(settled[0].contains("parallel-second"));
    assert!(settled[1].contains("parallel-first"));
    let requests = provider.requests.lock().unwrap().clone();
    let continuation = requests
        .iter()
        .find(|request| {
            request
                .messages
                .iter()
                .filter(|message| message.role == "tool")
                .count()
                >= 2
        })
        .unwrap();
    let annotations = continuation
        .messages
        .iter()
        .filter_map(hook_annotation)
        .map(|value| value["annotation"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(annotations, ["first-annotation", "second-annotation"]);
    let receipts = continuation
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .map(Message::text_projection)
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 2);
    assert!(receipts[0].contains("first bytes"));
    assert!(receipts[1].contains("second bytes"));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn post_turn_failure_continues_without_changing_the_answer_or_starting_a_turn() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Text);
    let hooks = LifecycleHooks {
        post_turn: vec![
            shell_hook("cat >/dev/null; printf x >> post-turn-runs; printf '{'"),
            shell_hook(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"later turn annotation\"}'",
            ),
        ],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let mut events = harness.subscribe();
    let first = harness
        .run_local_controlled(
            "first",
            Some(&target),
            "post-turn-first",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!first.reused);
    let first = first.output;
    assert_eq!(first.text, "final answer");
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    assert_eq!(
        harness.history().await.unwrap(),
        [
            Message::text("user", "first"),
            Message::text("assistant", "final answer")
        ]
    );
    assert!(
        !first
            .events
            .iter()
            .any(|event| { event.kind() == "hook" && event.detail().contains("post_turn") })
    );
    let live = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(live.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_turn")
            && event.detail().contains("failed")
    }));
    assert!(live.iter().any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_turn")
            && event.detail().contains("annotated")
    }));
    assert_eq!(
        std::fs::read(project.path().join("post-turn-runs")).unwrap(),
        b"x"
    );
    let retry = harness
        .run_local_controlled(
            "first",
            Some(&target),
            "post-turn-first",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(retry.reused);
    assert_eq!(retry.output.text, first.text);
    assert_eq!(
        std::fs::read(project.path().join("post-turn-runs")).unwrap(),
        b"x"
    );
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| { event.kind() == "hook" && event.detail().contains("post_turn") })
    );

    harness.run_for("second", Some(&target)).await.unwrap();
    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 4);
    assert!(requests[2..].iter().any(|request| {
        request.messages.iter().any(|message| {
            hook_annotation(message).is_some_and(|value| {
                value["annotation"] == "later turn annotation" && value["event"] == "post_turn"
            })
        })
    }));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn post_turn_never_runs_before_settlement_and_failed_annotation_cannot_undo_it() {
    let project = tempfile::tempdir().unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Text);
    let hooks = LifecycleHooks {
        post_turn: vec![shell_hook(
            "cat >/dev/null; printf x >> post-turn-order; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"settled note\"}'",
        )],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        memory.clone(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let session_id = harness.session.id.clone();
    let mut events = harness.subscribe();
    harness
        .reject_next_turn_settlement
        .store(true, Ordering::SeqCst);
    let rejected = harness
        .run_local_controlled(
            "settlement rejected",
            Some(&target),
            "rejected-settlement",
            &CancellationToken::new(),
        )
        .await;
    assert!(rejected.is_err(), "the settlement fixture was accepted");
    assert!(!project.path().join("post-turn-order").exists());
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| { event.kind() == "hook" && event.detail().contains("post_turn") })
    );
    let page = memory
        .public_transcript_page(&session_id, None, 16)
        .await
        .unwrap();
    assert!(!page.records.iter().any(|entry| {
        matches!(entry, PublicTranscriptEntry::Turn { record }
            if record.turn_id == "rejected-settlement"
                && record.settlement == PublicTurnSettlement::Completed)
    }));
    harness.shutdown(false).await.unwrap();

    let second_project = tempfile::tempdir().unwrap();
    let second_memory = MemoryStore::temporary().await.unwrap();
    let mut settled = Harness::new(
        config(LifecycleHooks {
            post_turn: vec![shell_hook(
                "cat >/dev/null; printf x >> post-turn-order; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"settled note\"}'",
            )],
            ..LifecycleHooks::default()
        }),
        second_project.path(),
        second_memory.clone(),
        provider,
        None,
    )
    .await
    .unwrap();
    let target = settled.topology.parts[0].id.clone();
    let session_id = settled.session.id.clone();
    let mut events = settled.subscribe();
    settled
        .reject_next_hook_annotation
        .store(true, Ordering::SeqCst);
    let completed = settled
        .run_local_controlled(
            "answer survives annotation refusal",
            Some(&target),
            "settled-before-hook",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(completed.output.text, "final answer");
    assert_eq!(
        std::fs::read(second_project.path().join("post-turn-order")).unwrap(),
        b"x"
    );
    assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
        event.kind() == "hook"
            && event.detail().contains("post_turn")
            && event.detail().contains("failed")
    }));
    let page = second_memory
        .public_transcript_page(&session_id, None, 16)
        .await
        .unwrap();
    assert!(page.records.iter().any(|entry| {
        matches!(entry, PublicTranscriptEntry::Turn { record }
            if record.turn_id == "settled-before-hook"
                && record.settlement == PublicTurnSettlement::Completed)
    }));
    let replay = settled
        .run_local_controlled(
            "answer survives annotation refusal",
            Some(&target),
            "settled-before-hook",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(replay.reused);
    assert_eq!(replay.output.text, completed.output.text);
    assert_eq!(
        std::fs::read(second_project.path().join("post-turn-order")).unwrap(),
        b"x"
    );
    settled.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn post_tool_annotation_stays_with_its_actor_and_can_be_omitted_by_context_fit() {
    let project = tempfile::tempdir().unwrap();
    let provider = Arc::new(AnnotationFitProvider::default());
    let response = json!({
        "decision": "annotate",
        "annotation": "PRIVATE-HOOK-SENTINEL"
    });
    let hooks = LifecycleHooks {
        post_tool: vec![shell_hook(&format!(
            "cat >/dev/null; printf '%s' '{}'",
            response
        ))],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let owner = harness.topology.parts[0].id.clone();
    let other = harness.topology.parts[1].id.clone();
    let owner_namespace = harness.namespace(&owner);
    let session_id = harness.session.id.clone();
    harness
        .run_for("emit the private tool annotation", Some(&owner))
        .await
        .unwrap();
    let before_other = provider.requests.lock().unwrap().len();
    harness
        .run_for("read only your own context", Some(&other))
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap().clone();
    assert!(
        requests.len() > before_other,
        "the other actor made no provider request"
    );
    assert!(requests[before_other..].iter().all(|request| {
        request.actor == harness.namespace(&other)
            && !request.instructions.contains("PRIVATE-HOOK-SENTINEL")
            && !request
                .messages
                .iter()
                .any(|message| message.text_projection().contains("PRIVATE-HOOK-SENTINEL"))
    }));

    let before_fit = requests.len();
    harness.run_for("FIT-PROBE", Some(&owner)).await.unwrap();
    let requests = provider.requests.lock().unwrap().clone();
    let attempts = &requests[before_fit..];
    assert!(
        attempts.len() > 1,
        "the private annotation was not fit-tested"
    );
    assert!(attempts.iter().all(|request| {
        request.actor == owner_namespace
            && request
                .messages
                .iter()
                .any(|message| message.text_projection().contains("FIT-PROBE"))
    }));
    assert!(attempts[..attempts.len() - 1].iter().any(|request| {
        request.messages.iter().any(|message| {
            hook_annotation(message)
                .is_some_and(|value| value["annotation"] == "PRIVATE-HOOK-SENTINEL")
        })
    }));
    assert!(attempts.last().unwrap().messages.iter().all(|message| {
        hook_annotation(message).is_none_or(|value| value["annotation"] != "PRIVATE-HOOK-SENTINEL")
    }));
    let history = harness
        .memory
        .session_history_window(&owner_namespace, &session_id, 64)
        .await
        .unwrap();
    assert!(history.messages.iter().any(|message| {
        hook_annotation(message).is_some_and(|value| value["annotation"] == "PRIVATE-HOOK-SENTINEL")
    }));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn speaker_stop_preserves_the_selected_peer_and_makes_no_speaking_dispatch() {
    for mode in Mode::ALL {
        let project = tempfile::tempdir().unwrap();
        let provider = CapturingProvider::new(ReplyPlan::Text);
        let hooks = LifecycleHooks {
            speaker_selected: vec![shell_hook(
                "cat >/dev/null; printf x > speaker-hook-ran; printf '%s' '{\"decision\":\"stop\",\"reason\":\"fixture stopped speaker\"}'",
            )],
            ..LifecycleHooks::default()
        };
        let mut settings = config(hooks);
        settings.mode = mode;
        let mut harness = Harness::new(
            settings,
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let error = harness
            .run_for("stop before speaking", Some(&target))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("fixture stopped speaker"),
            "{mode}: {error:#}"
        );
        let requests = provider.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1, "{mode}");
        assert!(requests[0].instructions.contains("Phase: deliberate"));
        assert_eq!(
            std::fs::read(project.path().join("speaker-hook-ran")).unwrap(),
            b"x"
        );
        assert_eq!(harness.topology.parts[0].id, target);
        harness.shutdown(false).await.unwrap();
    }
}

#[tokio::test]
async fn speaker_observe_receives_the_validated_selection_and_keeps_its_dispatch() {
    for mode in Mode::ALL {
        let project = tempfile::tempdir().unwrap();
        let provider = CapturingProvider::new(ReplyPlan::Text);
        let hooks = LifecycleHooks {
            speaker_selected: vec![shell_hook(
                "request=$(cat); case \"$request\" in *'\"speaker\"'*) ;; *) exit 9;; esac; case \"$request\" in *'\"reason\"'*) printf x > speaker-observed; printf '%s' '{\"decision\":\"observe\"}';; *) exit 9;; esac",
            )],
            ..LifecycleHooks::default()
        };
        let mut settings = config(hooks);
        settings.mode = mode;
        let mut harness = Harness::new(
            settings,
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let original_topology = serde_json::to_value(&harness.topology).unwrap();
        let output = harness
            .run_for("address the selected peer", Some(&target))
            .await
            .unwrap();
        assert_eq!(output.text, "final answer", "{mode}");
        assert_eq!(
            std::fs::read(project.path().join("speaker-observed")).unwrap(),
            b"x"
        );
        assert_eq!(
            serde_json::to_value(&harness.topology).unwrap(),
            original_topology
        );
        assert!(
            provider.requests.lock().unwrap().iter().any(|request| {
                request.instructions.contains("Phase: speak") && request.actor.contains(&target)
            }),
            "{mode}"
        );
        harness.shutdown(false).await.unwrap();
    }
}

#[tokio::test]
async fn speaker_hook_cannot_substitute_an_explicitly_selected_peer() {
    for decision in [
        "rewrite",
        "observe_with_actor",
        "observe_with_reason",
        "stop_with_actor",
        "malformed",
        "timeout",
    ] {
        let project = tempfile::tempdir().unwrap();
        let provider = CapturingProvider::new(ReplyPlan::Text);
        let script = match decision {
            "rewrite" => format!(
                "cat >/dev/null; printf '%s' '{}'",
                json!({"decision":"rewrite", "value":{"actor":"another-peer"}})
            ),
            "observe_with_actor" => format!(
                "cat >/dev/null; printf '%s' '{}'",
                json!({"decision":"observe", "actor":"another-peer"})
            ),
            "observe_with_reason" => format!(
                "cat >/dev/null; printf '%s' '{}'",
                json!({"decision":"observe", "reason":"switch selection"})
            ),
            "stop_with_actor" => format!(
                "cat >/dev/null; printf '%s' '{}'",
                json!({"decision":"stop", "reason":"switch peer", "actor":"another-peer"})
            ),
            "malformed" => "cat >/dev/null; printf '{'".to_owned(),
            "timeout" => "cat >/dev/null; sleep 30".to_owned(),
            _ => unreachable!(),
        };
        let mut hook = shell_hook(&script);
        if decision == "timeout" {
            hook.timeout_ms = 50;
        }
        let settings = config(LifecycleHooks {
            speaker_selected: vec![hook],
            ..LifecycleHooks::default()
        });
        let mut harness = Harness::new(
            settings,
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let original_topology = serde_json::to_value(&harness.topology).unwrap();
        let error = harness
            .run_for("address the selected peer", Some(&target))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("hook"), "{error:#}");
        assert_eq!(
            serde_json::to_value(&harness.topology).unwrap(),
            original_topology
        );
        let requests = provider.requests.lock().unwrap().clone();
        assert!(!requests.is_empty());
        assert!(
            requests
                .iter()
                .all(|request| { !request.instructions.contains("Phase: speak") })
        );
        assert!(
            harness
                .history()
                .await
                .unwrap()
                .iter()
                .any(|message| { message == &Message::text("user", "address the selected peer") })
        );
        harness.shutdown(false).await.unwrap();
    }
}

#[tokio::test]
async fn dream_tool_rewrites_stay_within_dream_validation_and_annotations_promote_from_candidate() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::DreamProposal);
    let hooks = LifecycleHooks {
        pre_tool: vec![shell_hook(
            "cat >/dev/null; printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"name\":\"shell\",\"arguments\":{\"command\":\"printf forbidden > forbidden-marker\"}}}'",
        )],
        post_tool: vec![shell_hook(
            "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"candidate annotation\"}'",
        )],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider,
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    let report = harness.dream().await.unwrap();
    assert!(
        report
            .rejected
            .iter()
            .any(|reason| reason.contains("only dream_suggest is available"))
    );
    assert!(!project.path().join("forbidden-marker").exists());
    assert!(
        harness
            .memory_for(&actor)
            .await
            .unwrap()
            .iter()
            .any(|message| hook_annotation(message)
                .is_some_and(|value| value["annotation"] == "candidate annotation"
                    && value["turn_id"].is_null()))
    );
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants() {
    let project = tempfile::tempdir().unwrap();
    let marker = project.path().join("second-hook-started");
    let survived = project.path().join("hook-descendant-survived");
    let script = format!(
        "request=$(cat); case \"$request\" in *dream-hook-call-1*) printf '%s' '{{\"decision\":\"annotate\",\"annotation\":\"candidate only\"}}';; *) printf started > '{}'; (sleep 1; printf survived > '{}') & sleep 30;; esac",
        marker.display(),
        survived.display()
    );
    let provider = CapturingProvider::new(ReplyPlan::DreamProposal);
    let hooks = LifecycleHooks {
        post_tool: vec![shell_hook(&script)],
        ..LifecycleHooks::default()
    };
    let harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider,
        None,
    )
    .await
    .unwrap();
    let actor = harness.topology.parts[0].id.clone();
    let cancellation = CancellationToken::new();
    let running = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            let mut harness = harness;
            let result = harness.dream_controlled(&cancellation).await;
            (harness, result)
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !marker.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    let (mut harness, result) = tokio::time::timeout(std::time::Duration::from_secs(10), running)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert!(
        !harness
            .memory_for(&actor)
            .await
            .unwrap()
            .iter()
            .any(|message| message.role == "kuru-hook")
    );
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert!(
        !survived.exists(),
        "hook descendant survived dream cancellation"
    );
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn pre_turn_denial_malformed_output_and_timeout_refuse_before_provider_dispatch() {
    for (label, mut hook) in [
        (
            "deny",
            shell_hook("cat >/dev/null; printf '%s' '{\"decision\":\"deny\"}'"),
        ),
        (
            "trailing",
            shell_hook("cat >/dev/null; printf '%s' '{\"decision\":\"allow\"} trailing'"),
        ),
        ("timeout", shell_hook("cat >/dev/null; sleep 30")),
    ] {
        if label == "timeout" {
            hook.timeout_ms = 50;
        }
        let project = tempfile::tempdir().unwrap();
        let provider = CapturingProvider::new(ReplyPlan::Text);
        let hooks = LifecycleHooks {
            pre_turn: vec![hook],
            ..LifecycleHooks::default()
        };
        let mut harness = Harness::new(
            config(hooks),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        let error = harness
            .run_for("durable original", Some(&target))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("hook"), "{label}: {error:#}");
        assert!(
            provider.requests.lock().unwrap().is_empty(),
            "{label} dispatched provider work"
        );
        let history = harness.history().await.unwrap();
        assert_eq!(history[0], Message::text("user", "durable original"));
        harness.shutdown(false).await.unwrap();
    }
}

#[tokio::test]
async fn denied_pre_tool_hook_reaches_the_model_without_dropping_its_call() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("first.txt"), "unread content").unwrap();
    let provider = CapturingProvider::new(ReplyPlan::OneRead);
    let mut harness = Harness::new(
        config(LifecycleHooks {
            pre_tool: vec![shell_hook(
                "cat >/dev/null; printf '%s' '{\"decision\":\"deny\"}'",
            )],
            ..LifecycleHooks::default()
        }),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let output = harness
        .run_for("read with policy", Some(&target))
        .await
        .unwrap();
    assert_eq!(output.text, "final answer");
    {
        let requests = provider.requests.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|request| request.messages.iter().any(|message| {
                    message.role == "tool"
                        && message.text_projection().contains("denied")
                        && !message.text_projection().contains("unread content")
                }))
        );
    }
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn exhausted_post_budget_keeps_answer_and_records_a_separate_failure() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Text);
    let hooks = LifecycleHooks {
        max_invocations: 1,
        post_turn: vec![
            shell_hook(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"first annotation\"}'",
            ),
            shell_hook(
                "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"second annotation\"}'",
            ),
        ],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    let mut events = harness.subscribe();
    let output = harness
        .run_for("answer despite budget", Some(&target))
        .await
        .unwrap();
    assert_eq!(output.text, "final answer");
    assert_eq!(
        harness.history().await.unwrap(),
        [
            Message::text("user", "answer despite budget"),
            Message::text("assistant", "final answer")
        ]
    );
    assert!(
        !output
            .events
            .iter()
            .any(|event| event.kind() == "hook" && event.detail().contains("post_turn"))
    );
    assert!(
        std::iter::from_fn(|| events.try_recv().ok()).any(|event| event.kind() == "hook"
            && event.detail().contains("post_turn")
            && event.detail().contains("failed"))
    );
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn refused_pre_turn_can_retry_exact_identity_without_duplicate_durable_input() {
    let project = tempfile::tempdir().unwrap();
    let provider = CapturingProvider::new(ReplyPlan::Text);
    let hooks = LifecycleHooks {
        pre_turn: vec![shell_hook(
            "cat >/dev/null; printf x >> hook-attempts; printf '%s' '{\"decision\":\"deny\"}'",
        )],
        ..LifecycleHooks::default()
    };
    let mut harness = Harness::new(
        config(hooks),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let target = harness.topology.parts[0].id.clone();
    for _ in 0..2 {
        let error = harness
            .run_local_controlled(
                "original retry input",
                Some(&target),
                "pre-refusal",
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("hook denied dispatch"));
    }
    assert!(provider.requests.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read(project.path().join("hook-attempts")).unwrap(),
        b"xx"
    );
    let history = harness.history().await.unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|message| message.role == "user")
            .count(),
        1
    );
    assert_eq!(history[0], Message::text("user", "original retry input"));
    harness.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn rewritten_safe_retry_uses_its_own_public_turn_after_marker_or_later_answer() {
    for intervening in [false, true] {
        let project = tempfile::tempdir().unwrap();
        let provider = CapturingProvider::new(ReplyPlan::Text);
        let hooks = LifecycleHooks {
            pre_turn: vec![shell_hook(
                "cat >/dev/null; if test ! -e denied-once; then : > denied-once; printf '%s' '{\"decision\":\"deny\"}'; else printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"input\":\"effective current input\"}}'; fi",
            )],
            ..LifecycleHooks::default()
        };
        let mut harness = Harness::new(
            config(hooks),
            project.path(),
            MemoryStore::temporary().await.unwrap(),
            provider.clone(),
            None,
        )
        .await
        .unwrap();
        let target = harness.topology.parts[0].id.clone();
        harness
            .run_local_controlled(
                "original pending input",
                Some(&target),
                "safe-retry",
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(provider.requests.lock().unwrap().is_empty());
        if intervening {
            harness
                .run_local_controlled(
                    "later settled input",
                    Some(&target),
                    "later-turn",
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
        }
        let before = provider.requests.lock().unwrap().len();
        let retried = harness
            .run_local_controlled(
                "original pending input",
                Some(&target),
                "safe-retry",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(retried.output.text, "final answer");
        {
            let requests = provider.requests.lock().unwrap();
            assert!(requests.len() > before);
            assert!(requests[before..].iter().all(|request| {
                let current = &request.messages
                    [request.messages.len() - request.current_message_count.unwrap()..];
                !current
                    .iter()
                    .any(|message| message.text_projection().contains("original pending input"))
            }));
            assert!(requests[before..].iter().any(|request| {
                let current = &request.messages
                    [request.messages.len() - request.current_message_count.unwrap()..];
                current.iter().any(|message| {
                    message
                        .text_projection()
                        .contains("effective current input")
                })
            }));
        }
        let history = harness.history().await.unwrap();
        assert_eq!(
            history
                .iter()
                .filter(|message| message.role == "user"
                    && message.plain_text() == Some("original pending input"))
                .count(),
            1
        );
        assert!(
            history
                .iter()
                .any(|message| message.role == crate::INTERRUPTION_ROLE)
        );
        harness.shutdown(false).await.unwrap();
    }
}
