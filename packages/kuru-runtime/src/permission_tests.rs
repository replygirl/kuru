use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use axum::{Json, Router, routing::post};
use kuru_connectors::{
    ApprovalAnswer, ApprovalRequest, ApprovalSender, InstructionActivation, InstructionGate,
    InstructionGateOutcome, InstructionReviewSender, PermissionInvocation, PermissionOutcome,
    PermissionService, Provider, ProviderEvent, ProviderSink, ToolHost,
};
use kuru_core::{
    Completion, CompletionRequest, Config, ContentBlock, Mode, ModelInfo, NativeTool,
    PermissionAction, PermissionRule, PermissionSelector, ProjectRelativeTarget, ToolCall,
};
use kuru_memory::MemoryStore;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{CancellationToken, Event, Harness, ToolOutcome};

struct FirstPathInstructions {
    active: Arc<std::sync::atomic::AtomicBool>,
}

struct PublishPathInstructions(Arc<std::sync::atomic::AtomicBool>);

#[async_trait]
impl InstructionActivation for PublishPathInstructions {
    async fn publish(self: Box<Self>) -> Result<String> {
        self.0.store(true, Ordering::SeqCst);
        Ok("new path-qualified instruction".into())
    }
}

#[async_trait]
impl InstructionGate for FirstPathInstructions {
    async fn review(
        &self,
        directories: &[ProjectRelativeTarget],
        _approval: Option<&InstructionReviewSender>,
    ) -> Result<InstructionGateOutcome> {
        assert_eq!(directories, &[ProjectRelativeTarget::parse("src")?]);
        Ok(if self.active.load(Ordering::SeqCst) {
            InstructionGateOutcome::Unchanged
        } else {
            InstructionGateOutcome::Proposed(Box::new(PublishPathInstructions(self.active.clone())))
        })
    }
}

struct TwoStaleFileCalls {
    speaking_requests: Mutex<Vec<String>>,
}

#[async_trait]
impl Provider for TwoStaleFileCalls {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let completion = if request.instructions.contains("Phase: deliberate") {
            Completion::from_legacy("ready", vec![], 0, 0)
        } else {
            let mut requests = self.speaking_requests.lock().unwrap();
            requests.push(request.instructions.clone());
            if requests.len() == 1 {
                Completion::from_legacy(
                    "",
                    ["first", "second"]
                        .into_iter()
                        .map(|id| ToolCall {
                            id: id.into(),
                            name: "file_write".into(),
                            arguments: json!({"path":format!("src/{id}.txt"),"content":"stale"}),
                        })
                        .collect(),
                    0,
                    0,
                )
            } else {
                Completion::from_legacy("replanned", vec![], 0, 0)
            }
        };
        sink.emit(ProviderEvent::Completed(completion)).await
    }
}

#[tokio::test]
async fn newly_activated_instructions_settle_stale_parallel_calls_without_effects() -> Result<()> {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    let config = Config {
        mode: Mode::Freudian,
        provider: "demo".into(),
        model: "demo".into(),
        allow_write: true,
        max_rounds: 1,
        dream_every: 0,
        dream_on_exit: false,
        ..Config::default()
    };
    let gate = Arc::new(FirstPathInstructions {
        active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let tools = ToolHost::new(project.path(), &config)?.with_instruction_gate(gate);
    let provider = Arc::new(TwoStaleFileCalls {
        speaking_requests: Mutex::new(Vec::new()),
    });
    let mut harness = Harness::with_tool_host_and_instructions(
        config,
        project.path(),
        String::new(),
        MemoryStore::temporary().await?,
        provider.clone(),
        None,
        tools,
    )
    .await?;
    let target = harness.topology.parts[0].id.clone();
    let output = harness
        .run_controlled(
            "write after review",
            Some(&target),
            "nested-replan",
            &CancellationToken::new(),
        )
        .await?;
    assert_eq!(output.text, "replanned");
    assert!(!project.path().join("src/first.txt").exists());
    assert!(!project.path().join("src/second.txt").exists());
    {
        let requests = provider.speaking_requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].contains("new path-qualified instruction"));
        assert!(requests[1].contains("new path-qualified instruction"));
    }
    let file_results = output.events.iter().filter(|event| matches!(event, Event::ToolSettled { observation, .. } if observation.name == "file_write")).count();
    assert_eq!(file_results, 2);
    harness.shutdown(false).await?;
    harness.memory.close().await?;
    Ok(())
}

struct OneExternalCall {
    issued: Mutex<bool>,
    in_deliberation: bool,
}

#[async_trait]
impl Provider for OneExternalCall {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let completion =
            if request.instructions.contains("Phase: deliberate") != self.in_deliberation {
                Completion::from_legacy("ready", vec![], 0, 0)
            } else {
                let mut issued = self.issued.lock().unwrap();
                if *issued {
                    Completion::from_legacy("finished", vec![], 0, 0)
                } else {
                    *issued = true;
                    Completion::from_legacy(
                        "",
                        vec![ToolCall {
                            id: "external-one".into(),
                            name: "a2a_send".into(),
                            arguments: json!({"agent":"reviewer","message":"one explicit request"}),
                        }],
                        0,
                        0,
                    )
                }
            };
        sink.emit(ProviderEvent::Completed(completion)).await
    }
}

fn provider(in_deliberation: bool) -> Arc<OneExternalCall> {
    Arc::new(OneExternalCall {
        issued: Mutex::new(false),
        in_deliberation,
    })
}

async fn peer() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
        let observed = observed.clone();
        async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{"message":{"parts":[{"text":"review complete"}]}}}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, hits, server)
}

fn config(url: String, action: Option<PermissionAction>) -> Config {
    let mut config = Config {
        mode: Mode::Freudian,
        provider: "demo".into(),
        model: "demo".into(),
        max_rounds: 1,
        dream_every: 0,
        dream_on_exit: false,
        external_agents: [("reviewer".into(), url)].into(),
        ..Config::default()
    };
    if let Some(action) = action {
        config.permissions.push(PermissionRule {
            action,
            selector: PermissionSelector::a2a("reviewer").unwrap(),
            path: None,
        });
    }
    config
}

async fn harness(config: Config, in_deliberation: bool) -> (tempfile::TempDir, Harness) {
    let project = tempfile::tempdir().unwrap();
    let harness = Harness::new(
        config,
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider(in_deliberation),
        None,
    )
    .await
    .unwrap();
    (project, harness)
}

fn settled(output: &crate::TurnOutput) -> Vec<ToolOutcome> {
    output
        .events
        .iter()
        .filter_map(|event| match event {
            Event::ToolSettled { observation, .. } if observation.name == "a2a_send" => {
                Some(observation.outcome)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn outbound_a2a_allow_ask_and_deny_have_real_network_effect_boundaries() {
    for (action, expected, outcome) in [
        (PermissionAction::Allow, 1, ToolOutcome::Ok),
        (PermissionAction::Ask, 0, ToolOutcome::Denied),
        (PermissionAction::Deny, 0, ToolOutcome::Denied),
    ] {
        let (url, hits, server) = peer().await;
        let (_project, mut harness) = harness(config(url, Some(action)), false).await;
        let target = harness.topology.parts[0].id.clone();
        let output = harness
            .run_controlled(
                "review once",
                Some(&target),
                "one-a2a",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), expected, "{action:?}");
        assert_eq!(settled(&output), [outcome], "{action:?}");
        harness.shutdown(false).await.unwrap();
        harness.memory.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn foreground_approval_is_operation_scoped_and_completed_retry_has_no_effect() {
    let (url, hits, server) = peer().await;
    let (_project, mut harness) = harness(config(url, Some(PermissionAction::Ask)), false).await;
    let target = harness.topology.parts[0].id.clone();
    let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
    let decision = tokio::spawn(async move {
        let request = receiver.recv().await.unwrap();
        assert_eq!(request.display.scope, "external agent reviewer");
        request.reply.send(ApprovalAnswer::Once).unwrap();
    });
    let first = harness
        .run_local_controlled_with_approval(
            "review once",
            Some(&target),
            "foreground-a2a",
            &CancellationToken::new(),
            ApprovalSender::new(sender),
        )
        .await
        .unwrap();
    decision.await.unwrap();
    assert_eq!(settled(&first.output), [ToolOutcome::Ok]);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let (unused, _receiver) = mpsc::channel(1);
    let reused = harness
        .retry_last_with_approval(&CancellationToken::new(), ApprovalSender::new(unused))
        .await
        .unwrap();
    assert!(reused.reused);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn unattended_cognitive_a2a_asks_without_dispatch() {
    let (url, hits, server) = peer().await;
    let (_project, mut harness) = harness(config(url, Some(PermissionAction::Ask)), true).await;
    let target = harness.topology.parts[0].id.clone();
    let output = harness
        .run_controlled(
            "deliberate",
            Some(&target),
            "internal-a2a",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(settled(&output), [ToolOutcome::Denied]);
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn closed_foreground_approval_refuses_without_network_dispatch() {
    let (url, hits, server) = peer().await;
    let (_project, mut harness) = harness(config(url, Some(PermissionAction::Ask)), false).await;
    let target = harness.topology.parts[0].id.clone();
    let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
    let close = tokio::spawn(async move {
        let request = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.display.scope, "external agent reviewer");
        drop(request);
        drop(receiver);
    });
    let result = harness
        .run_local_controlled_with_approval(
            "review once",
            Some(&target),
            "closed-approval",
            &CancellationToken::new(),
            ApprovalSender::new(sender),
        )
        .await
        .unwrap();
    close.await.unwrap();
    assert_eq!(settled(&result.output), [ToolOutcome::Denied]);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn cancelled_pending_approval_cannot_dispatch_later() {
    let (url, hits, server) = peer().await;
    let (_project, mut harness) = harness(config(url, Some(PermissionAction::Ask)), false).await;
    let target = harness.topology.parts[0].id.clone();
    let cancellation = CancellationToken::new();
    let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
    let error = {
        let run = harness.run_local_controlled_with_approval(
            "review once",
            Some(&target),
            "cancelled-approval",
            &cancellation,
            ApprovalSender::new(sender),
        );
        tokio::pin!(run);
        let request = tokio::select! {
            request = tokio::time::timeout(Duration::from_secs(10), receiver.recv()) =>
                request.unwrap().unwrap(),
            result = &mut run => panic!("turn settled before asking permission: {result:?}"),
        };
        cancellation.cancel();
        let error = run.await.unwrap_err();
        assert!(request.reply.send(ApprovalAnswer::Once).is_err());
        error
    };
    assert!(error.to_string().contains("turn cancelled"), "{error:#}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn new_harness_clears_grants_from_a_reused_permission_service() {
    let project = tempfile::tempdir().unwrap();
    let config = config("http://127.0.0.1:1/".into(), Some(PermissionAction::Ask));
    let tools = ToolHost::new(project.path(), &config).unwrap();
    let service = tools.permission_service();
    let arguments = json!({"agent":"reviewer","message":"one explicit request"});
    let invocation = PermissionInvocation::new(
        PermissionSelector::a2a("reviewer").unwrap(),
        None,
        &arguments,
    )
    .unwrap();
    let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
    let decision = tokio::spawn(async move {
        receiver
            .recv()
            .await
            .unwrap()
            .reply
            .send(ApprovalAnswer::Session)
            .unwrap();
    });
    assert_eq!(
        service
            .authorize(&invocation, Some(&ApprovalSender::new(sender)))
            .await
            .unwrap(),
        PermissionOutcome::SessionAuthorized
    );
    decision.await.unwrap();
    assert_eq!(service.inspect().unwrap().session.len(), 1);

    let mut harness = Harness::with_tool_host(
        config,
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider(false),
        None,
        tools,
    )
    .await
    .unwrap();
    assert!(service.inspect().unwrap().session.is_empty());
    assert!(Arc::ptr_eq(&service, &harness.permission_service()));
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

async fn grant_session_for_test(
    service: &PermissionService,
    invocation: &PermissionInvocation<'_>,
) {
    let (sender, mut receiver) = mpsc::channel::<ApprovalRequest>(1);
    let decision = tokio::spawn(async move {
        receiver
            .recv()
            .await
            .unwrap()
            .reply
            .send(ApprovalAnswer::Session)
            .unwrap();
    });
    assert_eq!(
        service
            .authorize(invocation, Some(&ApprovalSender::new(sender)))
            .await
            .unwrap(),
        PermissionOutcome::SessionAuthorized
    );
    decision.await.unwrap();
    assert_eq!(
        service.authorize(invocation, None).await.unwrap(),
        PermissionOutcome::Authorized
    );
}

#[tokio::test]
async fn resume_new_and_fork_clear_session_only_tool_authority() {
    let project = tempfile::tempdir().unwrap();
    let config = config("http://127.0.0.1:1/".into(), Some(PermissionAction::Ask));
    let tools = ToolHost::new(project.path(), &config).unwrap();
    let service = tools.permission_service();
    let mut harness = Harness::with_tool_host(
        config,
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        provider(false),
        None,
        tools,
    )
    .await
    .unwrap();
    let source = harness.session.id.clone();
    let target = harness.topology.parts[0].id.clone();
    harness
        .run_controlled(
            "one settled fork boundary",
            Some(&target),
            "permission-boundary",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let node = harness
        .memory
        .session_catalog_record(&source)
        .await
        .unwrap()
        .unwrap()
        .head_node_id
        .unwrap();
    let arguments = json!({"agent":"reviewer","message":"one explicit request"});
    let invocation = PermissionInvocation::new(
        PermissionSelector::a2a("reviewer").unwrap(),
        None,
        &arguments,
    )
    .unwrap();

    grant_session_for_test(&service, &invocation).await;
    let fresh = harness.new_session().await.unwrap();
    assert_ne!(fresh, source);
    assert_eq!(
        service.authorize(&invocation, None).await.unwrap(),
        PermissionOutcome::PermissionRequired
    );
    grant_session_for_test(&service, &invocation).await;
    harness.resume_session(&source).await.unwrap();
    assert_eq!(
        service.authorize(&invocation, None).await.unwrap(),
        PermissionOutcome::PermissionRequired
    );
    grant_session_for_test(&service, &invocation).await;
    let child = harness
        .fork_session(&source, &node, "permission fork")
        .await
        .unwrap();
    assert_ne!(child, source);
    assert_eq!(
        service.authorize(&invocation, None).await.unwrap(),
        PermissionOutcome::PermissionRequired
    );
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

#[tokio::test]
async fn changed_a2a_endpoint_cannot_reuse_the_frozen_alias_grant() {
    let (original_url, original_hits, original_server) = peer().await;
    let (changed_url, changed_hits, changed_server) = peer().await;
    let original_config = config(original_url, Some(PermissionAction::Allow));
    let (_project, mut harness) = harness(original_config, false).await;
    harness
        .config
        .external_agents
        .insert("reviewer".into(), changed_url);
    let target = harness.topology.parts[0].id.clone();
    let output = harness
        .run_controlled(
            "review once",
            Some(&target),
            "changed-route",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(settled(&output), [ToolOutcome::Error]);
    assert_eq!(original_hits.load(Ordering::SeqCst), 0);
    assert_eq!(changed_hits.load(Ordering::SeqCst), 0);
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    original_server.abort();
    changed_server.abort();
}

#[tokio::test]
async fn injected_host_with_a_different_route_is_rejected_at_construction() {
    let project = tempfile::tempdir().unwrap();
    let original = config("http://127.0.0.1:1/".into(), Some(PermissionAction::Allow));
    let changed = config("http://127.0.0.1:2/".into(), Some(PermissionAction::Allow));
    let tools = ToolHost::new(project.path(), &original).unwrap();
    let memory = MemoryStore::temporary().await.unwrap();
    let error = Harness::with_tool_host(
        changed,
        project.path(),
        memory.clone(),
        provider(false),
        None,
        tools,
    )
    .await
    .err()
    .expect("mismatched host must be rejected");
    assert!(
        error
            .to_string()
            .contains("permission service does not match"),
        "{error:#}"
    );
    memory.close().await.unwrap();
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefusedReceipt {
    call_id: String,
    output: Value,
    is_error: bool,
}

struct RefusalAwareProvider {
    speaking_round: AtomicUsize,
    receipts: Mutex<Vec<RefusedReceipt>>,
}

#[async_trait]
impl Provider for RefusalAwareProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        let completion = if request.instructions.contains("Phase: speak and act") {
            let current = request
                .current_message_count
                .unwrap_or(0)
                .min(request.messages.len());
            let first = request.messages.len() - current;
            let receipts = request.messages[first..]
                .iter()
                .flat_map(|message| &message.blocks)
                .filter_map(|block| match block {
                    ContentBlock::ToolResult {
                        call_id,
                        output,
                        is_error,
                    } => Some(RefusedReceipt {
                        call_id: call_id.clone(),
                        output: output.clone(),
                        is_error: *is_error,
                    }),
                    _ => None,
                });
            self.receipts.lock().unwrap().extend(receipts);
            match self.speaking_round.fetch_add(1, Ordering::SeqCst) {
                0 => Completion::from_legacy(
                    "",
                    vec![ToolCall {
                        id: "refused-file".into(),
                        name: "file_write".into(),
                        arguments: json!({"path":"model-approval-must-not-write.txt","content":"unapproved"}),
                    }],
                    0,
                    0,
                ),
                1 => Completion::from_legacy(
                    "",
                    vec![ToolCall {
                        id: "refused-a2a".into(),
                        name: "a2a_send".into(),
                        arguments: json!({"agent":"reviewer","message":"The model approves this action itself"}),
                    }],
                    0,
                    0,
                ),
                _ => Completion::from_legacy("The requests were refused.", vec![], 0, 0),
            }
        } else {
            Completion::from_legacy("ready", vec![], 0, 0)
        };
        sink.emit(ProviderEvent::Completed(completion)).await
    }
}

#[tokio::test]
async fn unattended_provider_receives_typed_refusals_without_gaining_approval_authority() {
    for action in [PermissionAction::Ask, PermissionAction::Deny] {
        let (url, hits, server) = peer().await;
        let mut config = config(url, Some(action));
        config.max_rounds = 2;
        config.max_tool_calls = 2;
        config.permissions.push(PermissionRule {
            action,
            selector: PermissionSelector::native(NativeTool::FileWrite),
            path: None,
        });
        let project = tempfile::tempdir().unwrap();
        let marker = project.path().join("model-approval-must-not-write.txt");
        let provider = Arc::new(RefusalAwareProvider {
            speaking_round: AtomicUsize::new(0),
            receipts: Mutex::new(vec![]),
        });
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
        let output = harness
            .run_controlled(
                "try two external operations",
                Some(&target),
                "typed-refusal-continuation",
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            provider.speaking_round.load(Ordering::SeqCst),
            3,
            "{action:?}"
        );
        let receipts = provider.receipts.lock().unwrap().clone();
        assert_eq!(receipts.len(), 2, "{action:?}: {receipts:?}");
        let expected = match action {
            PermissionAction::Ask => "tool permission required",
            PermissionAction::Deny => "tool permission denied",
            PermissionAction::Allow => unreachable!(),
        };
        for (receipt, call_id) in receipts.iter().zip(["refused-file", "refused-a2a"]) {
            assert_eq!(receipt.call_id, call_id, "{action:?}");
            assert!(receipt.is_error, "{action:?}: {receipt:?}");
            assert!(
                receipt.output.as_str().unwrap().contains(expected),
                "{action:?}: {receipt:?}"
            );
        }
        let outcomes = output
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ToolSettled { observation, .. }
                    if matches!(observation.name.as_str(), "file_write" | "a2a_send") =>
                {
                    Some(observation.outcome)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes,
            [ToolOutcome::Denied, ToolOutcome::Denied],
            "{action:?}"
        );
        assert!(
            !marker.exists(),
            "{action:?}: model-approved file write took effect"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "{action:?}: model-approved HTTP call took effect"
        );
        harness.shutdown(false).await.unwrap();
        harness.memory.close().await.unwrap();
        server.abort();
    }
}

struct DreamExternalCalls;

#[async_trait]
impl Provider for DreamExternalCalls {
    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    async fn stream(&self, _request: CompletionRequest, sink: &mut dyn ProviderSink) -> Result<()> {
        sink.emit(ProviderEvent::Completed(Completion::from_legacy(
            "A short memory summary",
            vec![
                ToolCall {
                    id: "dream-file".into(),
                    name: "file_write".into(),
                    arguments: json!({"path":"dream-must-not-write.txt","content":"unapproved"}),
                },
                ToolCall {
                    id: "dream-a2a".into(),
                    name: "a2a_send".into(),
                    arguments: json!({"agent":"reviewer","message":"dream proposes external work"}),
                },
            ],
            0,
            0,
        )))
        .await
    }
}

#[tokio::test]
async fn dream_proposed_external_calls_never_prompt_or_dispatch() {
    let (url, hits, server) = peer().await;
    let project = tempfile::tempdir().unwrap();
    let marker = project.path().join("dream-must-not-write.txt");
    let mut harness = Harness::new(
        config(url, Some(PermissionAction::Ask)),
        project.path(),
        MemoryStore::temporary().await.unwrap(),
        Arc::new(DreamExternalCalls),
        None,
    )
    .await
    .unwrap();
    let report = harness.dream().await.unwrap();
    assert!(report.accepted.is_empty());
    assert!(report.rejected.len() >= 2, "{report:?}");
    assert!(
        report
            .rejected
            .iter()
            .all(|reason| reason.contains("only dream_suggest is available")),
        "{report:?}"
    );
    assert!(!marker.exists(), "dream file call took effect");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "dream A2A call took effect");
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    server.abort();
}
