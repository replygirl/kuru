use kuru_memory::MemoryStore;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use kuru_connectors::{CheckpointState, CheckpointStore, Provider, ToolHost};
use kuru_core::{
    Completion, CompletionRequest, Config, ConfigSnapshot, InvocationOverrides, McpConfig, Mode,
    ModelInfo, RelationshipKind, ToolCall,
};
#[cfg(unix)]
use kuru_core::{HookCommand, LifecycleHooks};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::{
    CancellationToken, DreamProposal, Harness, INTERRUPTION_ROLE, INTERRUPTION_TEXT,
    turn_was_cancelled,
};

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
        let result = (self.respond)(&request);
        self.requests.lock().unwrap().push(request);
        result
    }
}

fn reply(text: &str) -> Completion {
    Completion::from_legacy(text, vec![], 0, 0)
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
#[cfg(unix)]
fn cancelled_post_tool_hook() -> LifecycleHooks {
    LifecycleHooks {
        post_tool: vec![HookCommand {
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "cat > cancelled-post-tool.json; printf '%s' '{\"decision\":\"observe\"}'".into(),
            ],
            ..HookCommand::default()
        }],
        ..LifecycleHooks::default()
    }
}

#[cfg(unix)]
fn assert_cancelled_post_tool(project: &TempDir, events: &[crate::Event], call_id: &str) {
    let request: Value = serde_json::from_slice(
        &std::fs::read(project.path().join("cancelled-post-tool.json"))
            .expect("post-tool hook did not run after cancelled settlement"),
    )
    .unwrap();
    assert_eq!(request["event"], "post_tool");
    assert_eq!(request["call_id"], call_id);
    assert_eq!(request["payload"]["is_error"], true);
    assert_eq!(request["payload"]["result"], "ERROR: turn cancelled");
    assert_eq!(
        events
            .iter()
            .filter(
                |event| matches!(event, crate::Event::Hook { observation, .. }
                if observation.event == "post_tool"
                    && observation.call_id.as_deref() == Some(call_id)
                    && observation.outcome == "observed")
            )
            .count(),
        1,
    );
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

async fn checkpoint_fixture(
    config: Config,
    provider: Arc<dyn Provider>,
) -> (TempDir, TempDir, Harness) {
    let project = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let project_path = project.path().canonicalize().unwrap();
    let root = Arc::new(
        Directory::open(&project_path, Privacy::Inherited, NameRetention::Movable).unwrap(),
    );
    let tools = ToolHost::with_retained_root(root.clone(), &config)
        .unwrap()
        .with_checkpoint_store(Arc::new(
            CheckpointStore::new(&data.path().join("checkpoints"), root).unwrap(),
        ))
        .unwrap();
    let harness = Harness::with_tool_host(
        config,
        &project_path,
        MemoryStore::temporary().await.unwrap(),
        provider,
        None,
        tools,
    )
    .await
    .unwrap();
    (project, data, harness)
}

#[tokio::test]
async fn captured_instruction_graph_reaches_provider_in_reviewed_order() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(root.path().join("AGENTS.md"), "OUTER-INSTRUCTION\n").unwrap();
    std::fs::write(project.join("AGENTS.md"), "LOCAL-INSTRUCTION\n").unwrap();
    std::fs::write(
        project.join("CLAUDE.md"),
        "@AGENTS.md\n@details.md\n@omitted.md\nCLAUDE-INSTRUCTION\n",
    )
    .unwrap();
    std::fs::write(project.join("details.md"), "IMPORTED-INSTRUCTION\n").unwrap();
    std::fs::write(project.join("omitted.md"), "OMITTED-BODY".repeat(22_000)).unwrap();
    let snapshot =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let first_config = config(Mode::Freudian);
    let tools = ToolHost::new(&project, &first_config).unwrap();
    let harness = Harness::with_tool_host_and_instructions(
        first_config,
        &project,
        snapshot.instructions().to_owned(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
        tools,
    )
    .await
    .unwrap();
    harness
        .ask(
            &harness.topology.parts[0].id,
            vec![kuru_core::Message::text("user", "inspect")],
            "inspect",
            vec![],
        )
        .await
        .unwrap();
    {
        let requests = provider.requests.lock().unwrap();
        let instructions = &requests.last().unwrap().instructions;
        let outer = instructions.find("OUTER-INSTRUCTION").unwrap();
        let local = instructions.find("LOCAL-INSTRUCTION").unwrap();
        let imported = instructions.find("IMPORTED-INSTRUCTION").unwrap();
        let claude = instructions.find("CLAUDE-INSTRUCTION").unwrap();
        assert!(outer < local && local < imported && imported < claude);
        assert_eq!(instructions.matches("LOCAL-INSTRUCTION").count(), 1);
        assert!(!instructions.contains("@details.md"));
        assert!(!instructions.contains("OMITTED-BODY"));
        assert!(instructions.contains("256 KiB file limit"));
    }

    std::fs::write(project.join("details.md"), "CHANGED-INSTRUCTION\n").unwrap();
    std::fs::write(project.join("later.md"), "LATER-INSTRUCTION\n").unwrap();
    std::fs::write(
        project.join("CLAUDE.md"),
        "@AGENTS.md\n@details.md\n@later.md\n@omitted.md\nCLAUDE-INSTRUCTION\n",
    )
    .unwrap();
    assert!(snapshot.instructions().contains("IMPORTED-INSTRUCTION"));
    assert!(!snapshot.instructions().contains("CHANGED-INSTRUCTION"));
    assert!(!snapshot.instructions().contains("LATER-INSTRUCTION"));
    let changed =
        ConfigSnapshot::parse(None, &project, None, InvocationOverrides::default()).unwrap();
    assert!(changed.instructions().contains("CHANGED-INSTRUCTION"));
    assert!(changed.instructions().contains("LATER-INSTRUCTION"));
    assert_ne!(
        snapshot.manifest().full_digest(),
        changed.manifest().full_digest()
    );
    let config = config(Mode::Freudian);
    let tools = ToolHost::new(&project, &config).unwrap();
    let changed_harness = Harness::with_tool_host_and_instructions(
        config,
        &project,
        changed.instructions().to_owned(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
        tools,
    )
    .await
    .unwrap();
    changed_harness
        .ask(
            &changed_harness.topology.parts[0].id,
            vec![kuru_core::Message::text("user", "inspect new graph")],
            "inspect",
            vec![],
        )
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let instructions = &requests.last().unwrap().instructions;
    assert!(instructions.contains("CHANGED-INSTRUCTION"));
    assert!(instructions.contains("LATER-INSTRUCTION"));
    assert!(!instructions.contains("IMPORTED-INSTRUCTION"));
}

struct OneToolProvider {
    tool: Mutex<Option<ToolCall>>,
    receipt: Mutex<Option<tokio::sync::oneshot::Sender<CompletionRequest>>>,
    issued: AtomicUsize,
}

impl OneToolProvider {
    fn new(tool: ToolCall) -> (Arc<Self>, tokio::sync::oneshot::Receiver<CompletionRequest>) {
        let (receipt, observed) = tokio::sync::oneshot::channel();
        (
            Arc::new(Self {
                tool: Mutex::new(Some(tool)),
                receipt: Mutex::new(Some(receipt)),
                issued: AtomicUsize::new(0),
            }),
            observed,
        )
    }
}

#[async_trait]
impl Provider for OneToolProvider {
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
        if request.instructions.contains("Phase: deliberate") {
            return Ok(reply("ready to use the requested tool"));
        }
        let has_tool_receipt = request
            .messages
            .iter()
            .any(|message| message.role == "tool");
        if has_tool_receipt {
            let receipt = self.receipt.lock().unwrap().take();
            if let Some(receipt) = receipt {
                let _ = receipt.send(request);
                std::future::pending().await
            }
        }
        if let Some(tool) = self.tool.lock().unwrap().take() {
            self.issued.fetch_add(1, Ordering::SeqCst);
            return Ok(Completion::from_legacy("", vec![tool], 0, 0));
        }
        Ok(reply("the later turn completed"))
    }
}

struct HeldBeforeToolProvider {
    ready: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

#[async_trait]
impl Provider for HeldBeforeToolProvider {
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
        if request.instructions.contains("Phase: deliberate") {
            return Ok(reply("ready to write"));
        }
        if let Some(ready) = self.ready.lock().unwrap().take() {
            let _ = ready.send(());
        }
        if let Some(release) = self.release.lock().await.take() {
            let _ = release.await;
        }
        Ok(Completion::from_legacy(
            "",
            vec![call(
                "cancelled-file-write",
                "file_write",
                json!({"path":"not-created.txt","content":"must not be written"}),
            )],
            0,
            0,
        ))
    }
}

#[derive(Clone)]
struct HeldHttpMcp {
    requests: Arc<tokio::sync::Mutex<Vec<(axum::http::Method, Value)>>>,
    call_seen: Arc<tokio::sync::Notify>,
    fail_delete: bool,
}

async fn held_http_mcp(
    axum::extract::State(state): axum::extract::State<HeldHttpMcp>,
    method: axum::http::Method,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    state
        .requests
        .lock()
        .await
        .push((method.clone(), body.clone()));
    if method == axum::http::Method::DELETE {
        state.call_seen.notify_one();
        if state.fail_delete {
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        return axum::Json(json!({})).into_response();
    }
    let result = match body["method"].as_str() {
        Some("initialize") => {
            let mut response = axum::Json(json!({
                "jsonrpc":"2.0",
                "id":body["id"],
                "result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}
            }))
            .into_response();
            response
                .headers_mut()
                .insert("mcp-session-id", "journal-fixture".parse().unwrap());
            return response;
        }
        Some("tools/list") => json!({
            "jsonrpc":"2.0",
            "id":body["id"],
            "result":{"tools":[{
                "name":"mutate",
                "description":"accepted mutation fixture",
                "inputSchema":{"type":"object"}
            }]}
        }),
        Some("tools/call") => {
            state.call_seen.notify_one();
            return std::future::pending().await;
        }
        _ => json!({}),
    };
    axum::Json(result).into_response()
}

struct McpToolProvider {
    issued: AtomicBool,
}

#[async_trait]
impl Provider for McpToolProvider {
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
        if request.instructions.contains("Phase: deliberate") {
            return Ok(reply("ready to call the configured server"));
        }
        if !self.issued.swap(true, Ordering::SeqCst) {
            let tool = request
                .tools
                .iter()
                .find(|tool| tool.description.contains("MCP accepted/mutate"))
                .expect("configured MCP tool must reach the speaking request");
            return Ok(Completion::from_legacy(
                "",
                vec![call(
                    "accepted-mcp-call",
                    &tool.name,
                    json!({"write":"once"}),
                )],
                0,
                0,
            ));
        }
        Ok(reply("the later turn completed"))
    }
}

#[tokio::test]
async fn cancellation_before_shared_tool_dispatch_runs_no_file_mutation() {
    let (ready, entered) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    let provider = Arc::new(HeldBeforeToolProvider {
        ready: Mutex::new(Some(ready)),
        release: tokio::sync::Mutex::new(Some(held)),
    });
    let (project, mut harness) = fixture(
        Config {
            allow_write: true,
            ..config(Mode::Freudian)
        },
        provider,
    )
    .await;
    let target = harness.topology.parts[0].id.clone();
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "cancel before the tool",
                Some(&target),
                "before-tool",
                &controlled,
            )
            .await;
        (harness, result)
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), entered)
        .await
        .expect("speaking provider did not reach the held tool response")
        .unwrap();
    cancellation.cancel();
    let _ = release.send(());
    let (mut harness, result) = tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .expect("cancelled pre-dispatch turn did not settle")
        .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    assert!(!project.path().join("not-created.txt").exists());
    let history = harness.history().await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].text_projection(), "cancel before the tool");
    assert_eq!(history[1].role, INTERRUPTION_ROLE);
    assert_eq!(history[1].text_projection(), INTERRUPTION_TEXT);
    let retry = harness
        .run_controlled(
            "cancel before the tool",
            Some(&harness.topology.parts[0].id.clone()),
            "before-tool",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    assert!(!project.path().join("not-created.txt").exists());
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

#[tokio::test]
async fn accepted_file_mutation_survives_cancellation_without_replay() {
    let (provider, receipt) = OneToolProvider::new(call(
        "accepted-file-write",
        "file_write",
        json!({"path":"accepted.txt","content":"one durable write"}),
    ));
    let (project, _checkpoint_data, mut harness) = checkpoint_fixture(
        Config {
            allow_write: true,
            ..config(Mode::Freudian)
        },
        provider.clone(),
    )
    .await;
    let target = harness.topology.parts[0].id.clone();
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "publish one file",
                Some(&target),
                "accepted-file",
                &controlled,
            )
            .await;
        (harness, result, target)
    });
    let receipt = tokio::time::timeout(std::time::Duration::from_secs(30), receipt)
        .await
        .expect("provider did not observe the accepted file receipt")
        .unwrap();
    let receipt = receipt
        .messages
        .iter()
        .find_map(|message| {
            (message.role == "tool")
                .then(|| crate::test_receipt(message).ok())
                .flatten()
        })
        .expect("accepted file effect did not produce a tool receipt");
    assert_eq!(receipt["call_id"], "accepted-file-write");
    assert_eq!(receipt["is_error"], false);
    let output = receipt["output"].as_str().unwrap();
    let checkpoint = output
        .strip_prefix("file_write completed; checkpoint ")
        .expect("accepted file receipt does not name its durable checkpoint")
        .to_owned();
    assert_eq!(
        std::fs::read_to_string(project.path().join("accepted.txt")).unwrap(),
        "one durable write"
    );
    cancellation.cancel();
    let (mut harness, result, target) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("cancelled post-file turn did not settle")
            .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    let checkpoint = harness
        .file_checkpoint(&checkpoint)
        .unwrap()
        .expect("accepted file checkpoint disappeared after cancellation");
    assert_eq!(checkpoint.state, CheckpointState::Applied);
    assert_eq!(checkpoint.path, "accepted.txt");
    assert_eq!(provider.issued.load(Ordering::SeqCst), 1);
    let retry = harness
        .run_controlled(
            "publish one file",
            Some(&target),
            "accepted-file",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    assert_eq!(provider.issued.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_to_string(project.path().join("accepted.txt")).unwrap(),
        "one durable write"
    );
    let later = harness
        .run_controlled(
            "continue after accepted file",
            Some(&target),
            "after-file",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(later.text, "the later turn completed");
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

#[tokio::test]
async fn accepted_cognitive_writes_reconcile_before_cancellation_stops_peer_work() {
    let identities = Arc::new(Mutex::new(Vec::<String>::new()));
    let configured = identities.clone();
    let issued = Arc::new(AtomicBool::new(false));
    let provider_issued = issued.clone();
    let provider = RecordingProvider::new(move |request| {
        if request.instructions.contains("Phase: deliberate")
            && !provider_issued.swap(true, Ordering::SeqCst)
        {
            let identities = configured.lock().unwrap();
            return reply("cognitive work").with_calls(vec![
                call(
                    "accepted-note",
                    "remember",
                    json!({"text":"retain this accepted note once"}),
                ),
                call(
                    "accepted-state",
                    "state_report",
                    json!({"activation":0.75,"note":"accepted before cancellation"}),
                ),
                call(
                    "blocked-peer",
                    "peer_send",
                    json!({"to":identities[1],"message":"must not be delivered"}),
                ),
            ]);
        }
        reply("the later cognitive turn completed")
    });
    let settings = Config {
        #[cfg(unix)]
        hooks: cancelled_post_tool_hook(),
        ..config(Mode::Freudian)
    };
    let (project, mut harness) = fixture(settings, provider).await;
    #[cfg(not(unix))]
    let _ = &project;
    *identities.lock().unwrap() = harness
        .topology
        .parts
        .iter()
        .map(|part| part.id.clone())
        .collect();
    let target = harness.topology.parts[0].id.clone();
    let mut events = harness.subscribe();
    let (written, release) = harness.pause_after_next_memory_write();
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "accept cognitive writes",
                Some(&target),
                "accepted-cognitive",
                &controlled,
            )
            .await;
        (harness, result, target)
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), written)
        .await
        .expect("accepted state write did not reach publication")
        .unwrap();
    cancellation.cancel();
    release.send(()).unwrap();
    let (mut harness, result, target) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("cancelled cognitive turn did not settle")
            .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    #[cfg(unix)]
    assert_cancelled_post_tool(&project, &events, "accepted-state");
    assert_eq!(harness.topology.states[&target].activation, 0.75);
    assert_eq!(
        harness
            .memory
            .history(&format!("{}/notes", harness.namespace(&target)), 10)
            .await
            .unwrap()
            .iter()
            .filter(|message| message.text_projection() == "retain this accepted note once")
            .count(),
        1
    );
    assert!(
        events.iter().all(|event| event.kind() != "peer"),
        "cognitive calls after observed cancellation must not start"
    );
    let retry = harness
        .run_controlled(
            "accept cognitive writes",
            Some(&target),
            "accepted-cognitive",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    assert_eq!(
        harness
            .memory
            .history(&format!("{}/notes", harness.namespace(&target)), 10)
            .await
            .unwrap()
            .iter()
            .filter(|message| message.text_projection() == "retain this accepted note once")
            .count(),
        1
    );
    harness.set_effort(Some("high".into())).await.unwrap();
    let later = harness
        .run_controlled(
            "continue cognitive work",
            Some(&target),
            "after-cognitive",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(later.text, "the later cognitive turn completed");
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_shell_turn_reaps_the_observed_owned_process_without_replay() {
    use nix::{errno::Errno, sys::signal, unistd::Pid};

    let (provider, _unused_receipt) = OneToolProvider::new(call(
        "held-shell",
        "shell",
        json!({
            "command":"printf '%s' \"$$\" > shell-owner.pid; exec /bin/sleep 120",
            "timeout_ms":120000
        }),
    ));
    let (project, mut harness) = fixture(
        Config {
            allow_shell: true,
            hooks: cancelled_post_tool_hook(),
            ..config(Mode::Freudian)
        },
        provider.clone(),
    )
    .await;
    let target = harness.topology.parts[0].id.clone();
    let mut events = harness.subscribe();
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "start one owned shell",
                Some(&target),
                "owned-shell",
                &controlled,
            )
            .await;
        (harness, result, target)
    });
    let pid_path = project.path().join("shell-owner.pid");
    let pid = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(&pid_path)
                && let Ok(pid) = value.parse::<i32>()
                && pid > 0
            {
                break pid;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("shell never published its admitted process identity");
    cancellation.cancel();
    let (mut harness, result, target) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("cancelled shell turn did not settle")
            .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(event, crate::Event::ToolSettled { observation, .. }
                    if observation.call_id == "held-shell"
                        && observation.outcome == crate::ToolOutcome::Cancelled
                        && observation.result_sha256.is_none())
            })
            .count(),
        1,
        "the cancelled external invocation settles exactly once"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, crate::Event::ToolSettled { .. }))
            .count(),
        1,
        "no second settlement can be hidden behind a different outcome"
    );
    assert_cancelled_post_tool(&project, &events, "held-shell");
    let retry = harness
        .run_controlled(
            "start one owned shell",
            Some(&target),
            "owned-shell",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    assert_eq!(provider.issued.load(Ordering::SeqCst), 1);
    tokio::time::timeout(std::time::Duration::from_secs(10), harness.shutdown(false))
        .await
        .expect("owned shell cleanup exceeded the shutdown bound")
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match signal::kill(Pid::from_raw(pid), None) {
                Ok(()) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                Err(Errno::ESRCH) => break,
                Err(error) => panic!("cannot inspect owned shell PID {pid}: {error}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("owned shell PID {pid} remained live after successful shutdown"));
    harness.memory.close().await.unwrap();
}

#[tokio::test]
async fn disabled_file_write_emits_a_structured_denied_observation() {
    let issued = Arc::new(AtomicBool::new(false));
    let issued_for_provider = issued.clone();
    let provider = RecordingProvider::new(move |request| {
        if request.instructions.contains("Phase: deliberate") {
            reply("ready to attempt the requested write")
        } else if request
            .messages
            .iter()
            .any(|message| message.role == "tool")
        {
            reply("the denied write was reported")
        } else if !issued_for_provider.swap(true, Ordering::SeqCst) {
            Completion::from_legacy(
                "",
                vec![call(
                    "denied-write",
                    "file_write",
                    json!({"path":"not-created.txt","content":"no mutation"}),
                )],
                0,
                0,
            )
        } else {
            reply("the tool was already attempted")
        }
    });
    let (project, mut harness) = fixture(config(Mode::Freudian), provider).await;
    let target = harness.topology.parts[0].id.clone();
    let output = harness
        .run_controlled(
            "attempt one denied write",
            Some(&target),
            "denied-write",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!project.path().join("not-created.txt").exists());
    assert_eq!(
        output
            .events
            .iter()
            .filter(|event| {
                matches!(event, crate::Event::ToolSettled { observation, .. }
                    if observation.call_id == "denied-write"
                        && observation.outcome == crate::ToolOutcome::Denied
                        && observation.result_bytes == 0
                        && observation.result_sha256.is_none())
            })
            .count(),
        1,
        "the denied external invocation settles exactly once"
    );
    assert_eq!(
        output
            .events
            .iter()
            .filter(|event| matches!(event, crate::Event::ToolSettled { .. }))
            .count(),
        1,
        "no second settlement can be hidden behind a different outcome"
    );
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_http_mcp_mutation_is_received_once_and_session_is_closed() {
    use axum::{Router, routing::any};

    let state = HeldHttpMcp {
        requests: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        call_seen: Arc::new(tokio::sync::Notify::new()),
        fail_delete: false,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .fallback(any(held_http_mcp))
                .with_state(server_state),
        )
        .await
        .unwrap();
    });
    let provider = Arc::new(McpToolProvider {
        issued: AtomicBool::new(false),
    });
    let mut settings = config(Mode::Freudian);
    settings.mcp.insert(
        "accepted".into(),
        McpConfig {
            url: Some(endpoint),
            ..McpConfig::default()
        },
    );
    let (_project, mut harness) = fixture(settings, provider.clone()).await;
    let target = harness.topology.parts[0].id.clone();
    let call_seen = state.call_seen.notified();
    tokio::pin!(call_seen);
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "send one MCP mutation",
                Some(&target),
                "accepted-http-mcp",
                &controlled,
            )
            .await;
        (harness, result, target)
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), &mut call_seen)
        .await
        .expect("HTTP MCP fixture did not receive the mutation");
    cancellation.cancel();
    let (mut harness, result, target) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("cancelled HTTP MCP turn did not settle")
            .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    let retry = harness
        .run_controlled(
            "send one MCP mutation",
            Some(&target),
            "accepted-http-mcp",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    assert!(provider.issued.load(Ordering::SeqCst));
    assert_eq!(
        state
            .requests
            .lock()
            .await
            .iter()
            .filter(|(_, body)| body["method"] == "tools/call")
            .count(),
        1
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), harness.shutdown(false))
        .await
        .expect("HTTP MCP session cleanup exceeded the shutdown bound")
        .unwrap();
    let requests = state.requests.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|(method, _)| *method == axum::http::Method::DELETE)
            .count(),
        1
    );
    drop(requests);
    harness.memory.close().await.unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

struct HeldShutdownDream {
    started: tokio::sync::Notify,
}

#[async_trait]
impl Provider for HeldShutdownDream {
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
            self.started.notify_one();
            std::future::pending().await
        }
        Ok(reply("answer before shutdown"))
    }
}

struct HeldPeerConsultation {
    recipient: Mutex<String>,
    issued: AtomicBool,
    calls: AtomicUsize,
    started: tokio::sync::Notify,
}

#[async_trait]
impl Provider for HeldPeerConsultation {
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
        if request.instructions.contains("Phase: peer consultation") {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            std::future::pending().await
        }
        if request.instructions.contains("Phase: speak")
            && !self.issued.swap(true, Ordering::SeqCst)
        {
            return Ok(reply("request peer input").with_calls(vec![call(
                "accepted-peer",
                "peer_send",
                json!({
                    "to":self.recipient.lock().unwrap().clone(),
                    "message":"consult exactly once"
                }),
            )]));
        }
        Ok(reply("the later peer turn completed"))
    }
}

#[tokio::test]
async fn cancelled_admitted_peer_consultation_is_not_replayed() {
    let provider = Arc::new(HeldPeerConsultation {
        recipient: Mutex::new(String::new()),
        issued: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        started: tokio::sync::Notify::new(),
    });
    let settings = Config {
        #[cfg(unix)]
        hooks: cancelled_post_tool_hook(),
        ..config(Mode::Freudian)
    };
    let (project, mut harness) = fixture(settings, provider.clone()).await;
    #[cfg(not(unix))]
    let _ = &project;
    let target = harness.topology.parts[0].id.clone();
    *provider.recipient.lock().unwrap() = harness.topology.parts[1].id.clone();
    let mut events = harness.subscribe();
    let consultation = provider.started.notified();
    tokio::pin!(consultation);
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "consult one peer",
                Some(&target),
                "accepted-peer",
                &controlled,
            )
            .await;
        (harness, result, target)
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), &mut consultation)
        .await
        .expect("peer consultation was not admitted");
    cancellation.cancel();
    let (mut harness, result, target) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("cancelled peer consultation did not settle")
            .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(event, crate::Event::ToolSettled { observation, .. }
                    if observation.call_id == "accepted-peer"
                        && observation.outcome == crate::ToolOutcome::Cancelled
                        && observation.result_sha256.is_none())
            })
            .count(),
        1,
        "the admitted peer invocation settles as cancelled after its consultation"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, crate::Event::ToolSettled { .. }))
            .count(),
        1,
        "the initial cognitive result must not settle before consultation completes"
    );
    #[cfg(unix)]
    assert_cancelled_post_tool(&project, &events, "accepted-peer");
    let retry = harness
        .run_controlled(
            "consult one peer",
            Some(&target),
            "accepted-peer",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let later = harness
        .run_controlled(
            "continue after peer cancellation",
            Some(&target),
            "after-peer",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(later.text, "the later peer turn completed");
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
}

#[tokio::test]
async fn shutdown_reports_dream_deadline_and_mcp_cleanup_failure_together() {
    use axum::{Router, routing::any};

    let state = HeldHttpMcp {
        requests: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        call_seen: Arc::new(tokio::sync::Notify::new()),
        fail_delete: true,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .fallback(any(held_http_mcp))
                .with_state(server_state),
        )
        .await
        .unwrap();
    });
    let provider = Arc::new(HeldShutdownDream {
        started: tokio::sync::Notify::new(),
    });
    let mut settings = config(Mode::Freudian);
    settings.dream_on_exit = true;
    settings.mcp.insert(
        "cleanup-fails".into(),
        McpConfig {
            url: Some(endpoint),
            ..McpConfig::default()
        },
    );
    let (_project, mut harness) = fixture(settings, provider.clone()).await;
    let target = harness.topology.parts[0].id.clone();
    harness
        .run_controlled(
            "complete before shutdown",
            Some(&target),
            "before-shutdown",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let dream_started = provider.started.notified();
    tokio::pin!(dream_started);
    let cleanup_seen = state.call_seen.notified();
    tokio::pin!(cleanup_seen);
    let task = tokio::spawn(async move {
        let result = harness.shutdown(true).await;
        (harness, result)
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), dream_started)
        .await
        .expect("shutdown dream did not start");
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    tokio::time::resume();
    tokio::time::timeout(std::time::Duration::from_secs(10), cleanup_seen)
        .await
        .expect("ToolHost cleanup did not attempt the negotiated MCP DELETE");
    let (harness, result) = tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .expect("aggregate shutdown did not settle after MCP cleanup failure")
        .unwrap();
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("shutdown dream exceeded 30 seconds"),
        "{error}"
    );
    assert!(error.contains("tool cleanup failed"), "{error}");
    assert!(
        error.contains("MCP session DELETE failed: 500 Internal Server Error"),
        "{error}"
    );
    assert_eq!(
        state
            .requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| *method == axum::http::Method::DELETE)
            .count(),
        1
    );
    harness.memory.close().await.unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn cancelled_outbound_a2a_request_is_received_once_without_replay() {
    use axum::{Json, Router, routing::post};

    let requests = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let observed = Arc::new(tokio::sync::Notify::new());
    let server_requests = requests.clone();
    let server_observed = observed.clone();
    let app = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let requests = server_requests.clone();
            let observed = server_observed.clone();
            async move {
                requests.lock().await.push(request);
                observed.notify_one();
                std::future::pending::<Json<Value>>().await
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let issued = Arc::new(AtomicBool::new(false));
    let provider_issued = issued.clone();
    let provider = RecordingProvider::new(move |request| {
        if request.instructions.contains("Phase: speak")
            && !provider_issued.swap(true, Ordering::SeqCst)
        {
            return reply("sending once").with_calls(vec![call(
                "accepted-a2a",
                "a2a_send",
                json!({"agent":"held","message":"perform this once"}),
            )]);
        }
        reply("the later outbound turn completed")
    });
    let mut settings = config(Mode::Freudian);
    settings.external_agents.insert("held".into(), endpoint);
    let (_project, mut harness) = fixture(settings, provider).await;
    let target = harness.topology.parts[0].id.clone();
    let request_seen = observed.notified();
    tokio::pin!(request_seen);
    let cancellation = CancellationToken::new();
    let controlled = cancellation.clone();
    let task = tokio::spawn(async move {
        let result = harness
            .run_controlled(
                "send one external request",
                Some(&target),
                "accepted-a2a",
                &controlled,
            )
            .await;
        (harness, result, target)
    });
    tokio::time::timeout(std::time::Duration::from_secs(30), &mut request_seen)
        .await
        .expect("outbound A2A fixture did not receive the request");
    cancellation.cancel();
    let (mut harness, result, target) =
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("cancelled outbound A2A turn did not settle")
            .unwrap();
    assert!(turn_was_cancelled(&result.unwrap_err()));
    let retry = harness
        .run_controlled(
            "send one external request",
            Some(&target),
            "accepted-a2a",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(retry.to_string().contains("may have reached external work"));
    let requests = requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "SendMessage");
    assert_eq!(
        requests[0]["params"]["message"]["parts"][0]["text"],
        "perform this once"
    );
    drop(requests);
    let later = harness
        .run_controlled(
            "continue outbound work",
            Some(&target),
            "after-a2a",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(later.text, "the later outbound turn completed");
    assert!(issued.load(Ordering::SeqCst));
    harness.shutdown(false).await.unwrap();
    harness.memory.close().await.unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn tool_only_deliberation_proceeds_to_a_useful_speaking_turn() {
    let provider = RecordingProvider::new(|request| {
        if request.instructions.contains("Phase: deliberate") {
            Completion::from_legacy(
                "",
                vec![call(
                    "state",
                    "state_report",
                    json!({"activation":0.5,"note":"Ready to help"}),
                )],
                0,
                0,
            )
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
        crate::project_scope(workspace.path()).unwrap(),
    )
    .unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(config, workspace.path(), memory, provider.clone(), None)
        .await
        .unwrap();
    let output = harness.run("Continue with available tools").await.unwrap();
    assert!(output.events.iter().any(|event| {
        event.kind() == "mcp"
            && event.actor() == "failed-fixture"
            && event.detail() == "configured server unavailable"
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
    harness.memory.clone().close().await.unwrap();
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
    let provider = RecordingProvider::new(|_| {
        Completion::from_legacy(
            "Useful private summary",
            vec![
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
            0,
            0,
        )
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
            .map(|message| crate::test_receipt(&message).unwrap())
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
async fn different_actors_share_leading_instructions_before_private_identity() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Shared synthetic project rule for both actors.",
    )
    .unwrap();
    let mut harness = Harness::new(
        config(Mode::Freudian),
        dir.path(),
        MemoryStore::temporary().await.unwrap(),
        provider.clone(),
        None,
    )
    .await
    .unwrap();
    let first = harness.topology.parts[0].id.clone();
    let second = harness.topology.parts[1].id.clone();
    harness.run_for("First task", Some(&first)).await.unwrap();
    harness.run_for("Second task", Some(&second)).await.unwrap();
    fn before_identity(instructions: &str) -> &str {
        instructions
            .split_once("\nYou are ")
            .expect("actor identity follows common rules")
            .0
    }
    let common = {
        let requests = provider.requests.lock().unwrap();
        let first_request = requests
            .iter()
            .find(|request| request.actor.ends_with(&first))
            .unwrap();
        let second_request = requests
            .iter()
            .find(|request| request.actor.ends_with(&second))
            .unwrap();
        assert_eq!(
            before_identity(&first_request.instructions),
            before_identity(&second_request.instructions)
        );
        assert!(
            before_identity(&first_request.instructions).contains("Shared synthetic project rule")
        );
        assert!(first_request.instructions.contains(&format!("ID {first}")));
        assert!(
            second_request
                .instructions
                .contains(&format!("ID {second}"))
        );
        assert!(
            first_request
                .instructions
                .find(&format!("ID {first}"))
                .unwrap()
                > before_identity(&first_request.instructions).len()
        );
        assert_eq!(first_request.tools, second_request.tools);
        let first_actor_phases = requests
            .iter()
            .filter(|request| request.actor.ends_with(&first))
            .collect::<Vec<_>>();
        assert!(
            first_actor_phases
                .iter()
                .any(|request| request.instructions.contains("Phase: deliberate"))
        );
        assert!(
            first_actor_phases
                .iter()
                .any(|request| request.instructions.contains("Phase: speak"))
        );
        for request in first_actor_phases {
            assert_eq!(
                before_identity(&request.instructions),
                before_identity(&first_request.instructions)
            );
        }
        before_identity(&first_request.instructions).to_owned()
    };
    harness.topology.parts[0].name.push_str(" renamed");
    harness
        .ask(
            &first,
            vec![kuru_core::Message::text("user", "Topology-only probe")],
            "speak",
            vec![],
        )
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let renamed = requests.last().unwrap();
    assert_eq!(before_identity(&renamed.instructions), common);
    assert!(renamed.instructions.contains(" renamed"));
}

#[tokio::test]
async fn shared_transcript_keeps_whole_unicode_rows_without_byte_slicing() {
    let provider = RecordingProvider::new(|_| reply("Ready"));
    let (_dir, harness) = fixture(config(Mode::Freudian), provider.clone()).await;
    crate::public_test_support::settled_public_turn(
        &harness.memory,
        &harness.scope,
        &harness.session.id,
        "whole-unicode-public",
        &harness.topology.parts[0].id,
        "Remember the whole Unicode answer",
        &format!("a{}", "🪶".repeat(20_000)),
    )
    .await
    .unwrap();
    harness
        .ask(
            &harness.topology.parts[0].id,
            vec![kuru_core::Message::text("user", "inspect")],
            "inspect",
            vec![],
        )
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    let instructions = &requests.last().unwrap().instructions;
    assert!(instructions.contains(&"🪶".repeat(20_000)));
    assert!(!instructions.contains("[truncated]"));
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
        harness.memory_for(&retiring).await.unwrap()[0].text_projection(),
        "retained part insight"
    );
    assert_eq!(
        harness.memory_for(&relation.id).await.unwrap()[0].text_projection(),
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
        harness.memory_for(&added).await.unwrap()[0].text_projection(),
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
        Completion::from_legacy(
            "",
            (0..64)
                .map(|i| {
                    call(
                        &format!("note-{i}"),
                        "remember",
                        json!({"text":format!("note {i}")}),
                    )
                })
                .collect(),
            0,
            0,
        )
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
    let before_revision = harness.memory.revision().await.unwrap();
    let proposal = DreamProposal::Add {
        name: "Experiment".into(),
        role: "id".into(),
        instruction: "Explore".into(),
    };
    harness.memory.reject_next_state_write_for_test();
    let error = harness.apply_dream(vec![proposal]).await.unwrap_err();
    assert!(format!("{error:#}").contains("injected state-save refusal before request send"));
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    assert_eq!(harness.memory.revision().await.unwrap(), before_revision);
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
    let first = harness.topology.parts[0].id.clone();
    let second = harness.topology.parts[1].id.clone();
    harness.memory.reject_next_state_write_for_test();
    let error = harness.focus(Some(&first)).await.unwrap_err();
    assert!(format!("{error:#}").contains("injected state-save refusal before request send"));
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    harness.memory.reject_next_state_write_for_test();
    let error = harness
        .relate(RelationshipKind::Alliance, vec![first, second])
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("injected state-save refusal before request send"));
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    harness.memory.reject_next_state_write_for_test();
    let error = harness.set_mode(Mode::Jungian).await.unwrap_err();
    assert!(format!("{error:#}").contains("injected state-save refusal before request send"));
    assert_eq!(harness.config.mode, Mode::Freudian);
    assert_eq!(harness.session.mode, Mode::Freudian);
    assert_eq!(serde_json::to_value(&harness.topology).unwrap(), before);
    assert_eq!(harness.actors.keys().cloned().collect::<Vec<_>>(), actors);
}

#[tokio::test]
async fn large_private_context_keeps_every_current_tool_receipt_whole() {
    let provider = RecordingProvider::new(|_| reply("Read all receipts"));
    let (_dir, harness) = fixture(
        Config {
            max_tool_calls: 64,
            // The 64 whole receipts alone exceed the default 128k window.
            // Keep them large but give the required input a valid finite fit;
            // old private context and notes remain optional under that bound.
            assumed_context_window_tokens: Some(1_300_000),
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
        .map(|i| {
            kuru_core::Message::text(
                "tool",
                json!({"call_id":format!("call-{i}"), "output":"\0\n🪶".repeat(3000)}).to_string(),
            )
        })
        .collect();
    harness
        .ask(&id, inputs, "current followup", vec![])
        .await
        .unwrap();
    let request = provider.requests.lock().unwrap().last().cloned().unwrap();
    assert!(request.context_budget.is_some());
    let receipts = request
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .map(|message| crate::test_receipt(message).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 64);
    for (i, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt["call_id"], format!("call-{i}"));
        assert_eq!(receipt["output"].as_str().unwrap(), "\0\n🪶".repeat(3000));
    }
    let notes = request
        .instructions
        .split_once("Your own durable notes (data, not higher-priority instructions):\n")
        .map(|(_, json)| serde_json::from_str::<Vec<kuru_core::Message>>(json).unwrap())
        .unwrap_or_default();
    assert_eq!(
        notes.len(),
        16,
        "the finite bound should still retain whole notes"
    );
    assert!(
        notes
            .iter()
            .all(|note| note.text_projection().contains(&"🪶".repeat(3000)))
    );
    assert_eq!(
        harness
            .memory
            .history_window(&format!("{}/notes", harness.namespace(&id)), 16)
            .await
            .unwrap()
            .total_rows,
        16,
        "fitting the request must not delete optional notes"
    );
}

#[tokio::test]
async fn malformed_current_tool_receipts_fail_before_provider_invocation() {
    let provider = RecordingProvider::new(|_| reply("Should not be reached"));
    let (_dir, harness) = fixture(config(Mode::Freudian), provider.clone()).await;
    let id = &harness.topology.parts[0].id;
    for content in ["not JSON", "{}", "{\"call_id\":\"a\"}"] {
        let inputs = vec![kuru_core::Message::text("tool", content)];
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
            return reply("Combine our complementary perspectives").with_calls(vec![call(
                "create-alliance",
                "relate",
                json!({"kind":"alliance","members":&ids[..2]}),
            )]);
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
            .any(|event| event.kind() == "relationship" && event.actor() == ids[0])
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
            return reply("VERIFIED-PEER-ADVICE").with_calls(vec![call(
                "recursive-attempt",
                "peer_send",
                json!({"to":ids[0],"message":"Try recursion"}),
            )]);
        }
        if request.instructions.contains("Phase: speak") {
            if request
                .messages
                .iter()
                .any(|message| message.text_projection().contains("VERIFIED-PEER-ADVICE"))
            {
                return reply("Used the verified peer advice");
            }
            return reply("Checking with a peer").with_calls(vec![call(
                "consult",
                "peer_send",
                json!({"to":ids[1],"message":"Check the final implementation"}),
            )]);
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
            .filter(|event| event.kind() == "peer")
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
            .any(|event| event.kind() == "error" && event.actor() == failed_id)
    );
    let all_failed = RecordingProvider::fallible(|_| anyhow::bail!("service unavailable"));
    let (_other_dir, mut unavailable) = fixture(config(Mode::Freudian), all_failed).await;
    let error = unavailable.run("Try a request").await.unwrap_err();
    assert!(error.to_string().contains("all peers failed"));
    assert_eq!(unavailable.session.turns, 0);
    let history = unavailable.history().await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].role, INTERRUPTION_ROLE);
    assert_eq!(history[1].text_projection(), INTERRUPTION_TEXT);
}

#[tokio::test]
async fn dreaming_accepts_a_valid_model_proposal_and_failed_undo_remains_recoverable() {
    let proposer = Arc::new(Mutex::new(String::new()));
    let selected = proposer.clone();
    let provider = RecordingProvider::new(move |request| {
        if request.actor.ends_with(selected.lock().unwrap().as_str()) {
            return reply("The project would benefit from broader options").with_calls(vec![call(
                    "new-peer",
                    "dream_suggest",
                    json!({"action":"add","name":"Possibility","role":"id","instruction":"Explore practical alternatives as an equal peer"}),
                )]);
        }
        reply("Consolidated private project knowledge")
    });
    let (_dir, mut harness) = fixture(config(Mode::Freudian), provider).await;
    *proposer.lock().unwrap() = harness.topology.parts[0].id.clone();
    let report = harness.dream().await.unwrap();
    assert_eq!(report.accepted.len(), 1);
    assert!(report.rejected.is_empty());
    let new_part = harness.resolve("Possibility").unwrap();
    harness.memory.reject_next_state_write_for_test();
    let error = harness.undo_dream().await.unwrap_err();
    assert!(format!("{error:#}").contains("injected state-save refusal before request send"));
    assert!(harness.resolve(&new_part).is_ok());
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
            if request.messages.iter().any(|message| {
                message
                    .text_projection()
                    .contains("EXTERNAL-REVIEW-COMPLETE")
            }) {
                return reply("Applied the external review");
            }
            return reply("Requesting a review").with_calls(vec![call(
                "external",
                "a2a_send",
                json!({"agent":"reviewer","message":"Review this explicitly shared question"}),
            )]);
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
        starts: AtomicUsize,
        entered: Mutex<Option<oneshot::Sender<CompletionRequest>>>,
        dropped: Mutex<Option<oneshot::Sender<()>>>,
    }
    #[async_trait]
    impl Provider for Cancellable {
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
            self.starts.fetch_add(1, Ordering::SeqCst);
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
        starts: AtomicUsize::new(0),
        entered: Mutex::new(Some(entered)),
        dropped: Mutex::new(Some(dropped)),
    });
    let (_dir, harness) = fixture(
        Config {
            max_parallel: 2,
            ..config(Mode::Freudian)
        },
        provider.clone(),
    )
    .await;
    let initial_peers = harness
        .topology
        .parts
        .iter()
        .filter(|part| part.active)
        .count();
    assert!(initial_peers > 2, "fixture needs a queued peer");
    let started_work = harness
        .actors
        .values()
        .map(crate::actor::Actor::started_work_counter)
        .collect::<Vec<_>>();
    let permits = harness.permits.clone();
    let shared = Arc::new(tokio::sync::Mutex::new(harness));
    let running = shared.clone();
    let mut task = tokio::spawn(async move {
        running
            .lock()
            .await
            .run_controlled(
                "Start a task",
                None,
                "aborted-owned-id",
                &CancellationToken::new(),
            )
            .await
    });
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
            .any(|message| message.role == "user" && message.text_projection() == "Start a task")
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while provider.active.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("two parallel providers must enter while another peer remains queued");
    tokio::time::timeout(Duration::from_secs(10), async {
        while started_work
            .iter()
            .map(|counter| counter.load(Ordering::SeqCst))
            .sum::<u64>()
            < initial_peers as u64
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("every initial peer Work must enter its actor, including the permit waiter");
    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    assert!(
        permits.try_acquire_many(2).is_err(),
        "parallel providers must hold both permits"
    );

    // Both parallel providers must drop; the queued third peer must never
    // begin the cancelled invocation before the next turn is admitted.
    let permit = tokio::time::timeout(Duration::from_secs(2), async {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        cancellation
            .await
            .expect("stalled provider must be dropped");
        permits
            .acquire_many(2)
            .await
            .expect("actor pool remains open")
    })
    .await
    .expect("cancellation must drop parallel provider work and release both permits within 2s");
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
    assert_eq!(
        provider.starts.load(Ordering::SeqCst),
        2 + initial_peers + 1,
        "the cancelled queued peer must not reach the provider; the new turn runs normally"
    );
    assert_eq!(
        shared
            .lock()
            .await
            .session_usage()
            .await
            .unwrap()
            .invocation_count,
        (2 + initial_peers + 1) as u64,
        "only the entered cancelled invocations and the new turn are accounted"
    );
    let calls_before_retry = provider.starts.load(Ordering::SeqCst);
    let original_retry = shared
        .lock()
        .await
        .run_controlled(
            "Start a task",
            None,
            "aborted-owned-id",
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(
        original_retry
            .to_string()
            .contains("may have reached external work")
    );
    assert_eq!(provider.starts.load(Ordering::SeqCst), calls_before_retry);
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
            assert_eq!(
                notes[0].text_projection(),
                "Retain this useful private summary"
            );
        }
    }
    harness.shutdown(false).await.unwrap();
}
