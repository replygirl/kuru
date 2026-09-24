use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Output,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[cfg(unix)]
use std::{process::Command as ProcessCommand, time::Duration};

use async_trait::async_trait;
use axum::{Router, body::Bytes, extract::State, routing::any};
use kuru_connectors::{DemoProvider, Provider, ToolHost};
use kuru_core::{
    CompletionRequest, Config, ConfigSnapshot, InvocationOverrides, McpConfig, ModelInfo,
    ProjectPreferences,
};
use kuru_delivery::command::BlockingCommand as Command;
use kuru_memory::MemoryStore;
use kuru_runtime::{DreamProposal, Harness, Topology};
use serde_json::{Value, json};

#[path = "support/memory.rs"]
mod memory;

#[cfg(unix)]
#[allow(dead_code)]
#[path = "support/terminal.rs"]
mod terminal;

// Windows PowerShell 5.1 engine/host cold start (module/format data load, first
// runspace build) can stall well past a single `tool shell` call's own budget
// before any script line runs. Warm the exact ToolHost stock-shell launch path
// once per test-binary process, ahead of the first real `tool shell` command,
// so that stall lands here (with its own distinct, generous bound) instead of
// inside a test's real assertion window. See `kuru_connectors::shell_warmup`.
#[cfg(windows)]
fn ensure_powershell_warm() {
    // `Sandbox::new()` runs from both plain `fn` tests and from inside
    // `#[tokio::test(flavor = "multi_thread", ...)]` async tests already
    // driving a Tokio runtime; the shared helper is safe from either call
    // site (see `kuru_connectors::shell_warmup::block_on_dedicated_thread`).
    kuru_connectors::shell_warmup::ensure_stock_powershell_warm();
}

struct Sandbox {
    root: memory::ServiceCleanup,
    project: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new(config: &str) -> Self {
        #[cfg(windows)]
        ensure_powershell_warm();
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(project.join(".kuru")).unwrap();
        std::fs::create_dir(root.path().join("config")).unwrap();
        std::fs::write(project.join(".kuru/config.toml"), config).unwrap();
        Self {
            root: memory::ServiceCleanup::new(root, &data),
            project,
            data,
        }
    }

    fn command(&self) -> Command {
        self.command_at(&self.project)
    }

    fn command_at(&self, project: &std::path::Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        #[cfg(windows)]
        command.fixture_allow_independent_service();
        command
            .arg("-C")
            .arg(project)
            .arg("--data-dir")
            .arg(&self.data)
            .env("XDG_CONFIG_HOME", self.root.path().join("config"));
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn success(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn write_config(&self, config: &str) {
        std::fs::write(self.project.join(".kuru/config.toml"), config).unwrap();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn safe_source_label(path: &std::path::Path) -> String {
    path.canonicalize()
        .unwrap()
        .as_os_str()
        .to_string_lossy()
        .chars()
        .flat_map(char::escape_default)
        .take(160)
        .collect()
}

struct RecordingDemo {
    instructions: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl Provider for RecordingDemo {
    async fn models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        DemoProvider.models().await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        sink: &mut dyn kuru_connectors::ProviderSink,
    ) -> anyhow::Result<()> {
        self.instructions
            .lock()
            .unwrap()
            .push(request.instructions.clone());
        DemoProvider.stream(request, sink).await
    }
}

struct HttpMcpFixture {
    url: String,
    requests: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct HttpMcpState {
    requests: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl HttpMcpFixture {
    async fn new() -> Self {
        let requests = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .fallback(any(http_mcp))
            .with_state(HttpMcpState {
                requests: requests.clone(),
                calls: calls.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            requests,
            calls,
            task,
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for HttpMcpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn http_mcp(State(state): State<HttpMcpState>, body: Bytes) -> axum::Json<Value> {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let response = match request["method"].as_str() {
        Some("initialize") => json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "protocolVersion": "2025-11-25",
                "capabilities": { "tools": {} }
            }
        }),
        Some("tools/list") => json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": { "tools": [
                {
                    "name": "projected_result",
                    "description": "isolated projection fixture",
                    "inputSchema": {"type":"object","additionalProperties":false}
                },
                {
                    "name": "projected_failure",
                    "description": "isolated projected error fixture",
                    "inputSchema": {"type":"object","additionalProperties":false}
                }
            ] }
        }),
        Some("tools/call") => {
            state.calls.fetch_add(1, Ordering::SeqCst);
            let result = if request["params"]["name"] == "projected_failure" {
                json!({
                    "content": [{"type":"text","text":"MCP fixture refused openai_api_key=sk-proj-abcdefghijklmnop0123456789; ordinary-control-remains-exact"}],
                    "isError": true
                })
            } else {
                json!({
                    "content": [{"type":"text","text":"openai_api_key=sk-proj-abcdefghijklmnop0123456789; ordinary-control-remains-exact"}],
                    "isError": false
                })
            };
            json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
        }
        _ => json!({}),
    };
    axum::Json(response)
}

struct NativeMcpFixture {
    directory: tempfile::TempDir,
    command: String,
    args: Vec<String>,
}

impl NativeMcpFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();

        #[cfg(unix)]
        let (command, args) = {
            let source = directory.path().join("stdio_peer.rs");
            let command = directory.path().join("stdio_peer");
            std::fs::write(
                &source,
                include_str!("../../../packages/kuru-connectors/tests/fixtures/stdio_peer.rs"),
            )
            .unwrap();
            let output = std::process::Command::new(
                std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()),
            )
            .args([
                "--edition=2024",
                "--forbid",
                "unsafe_code",
                "-C",
                "opt-level=1",
            ])
            .arg(&source)
            .arg("-o")
            .arg(&command)
            .output()
            .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o500)).unwrap();
            std::fs::write(
                command.with_extension("plan"),
                concat!(
                    "read\n",
                    "write {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{\"tools\":{}}}}\n",
                    "read\n",
                    "read\n",
                    "write {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\n",
                    "eof\n",
                ),
            )
            .unwrap();
            (command.to_str().unwrap().to_owned(), Vec::new())
        };

        #[cfg(windows)]
        let (command, args) = {
            let started = powershell_literal(&directory.path().join("stdio.started"));
            let done = powershell_literal(&directory.path().join("stdio.done"));
            let script = format!(
                "$ErrorActionPreference='Stop'; [IO.File]::WriteAllText({started},'started'); $null=[Console]::In.ReadLine(); [Console]::Out.WriteLine('{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{{\"tools\":{{}}}}}}}}'); $null=[Console]::In.ReadLine(); $null=[Console]::In.ReadLine(); [Console]::Out.WriteLine('{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"tools\":[]}}}}'); $null=[Console]::In.ReadLine(); [IO.File]::WriteAllText({done},'done')"
            );
            (
                "powershell.exe".to_owned(),
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    script,
                ],
            )
        };

        Self {
            directory,
            command,
            args,
        }
    }

    fn started(&self) -> usize {
        self.markers("started")
    }

    fn completed(&self) -> usize {
        self.markers("done")
    }

    fn conversations(&self) -> Vec<Vec<Value>> {
        let mut files: Vec<_> = std::fs::read_dir(self.directory.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|value| value == "requests"))
            .collect();
        files.sort();
        files
            .into_iter()
            .map(|path| {
                std::fs::read_to_string(path)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            })
            .collect()
    }

    fn markers(&self, extension: &str) -> usize {
        std::fs::read_dir(self.directory.path())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == extension)
            })
            .count()
    }
}

#[cfg(windows)]
fn powershell_literal(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn mcp_config(stdio: &NativeMcpFixture, http: &HttpMcpFixture) -> String {
    let mcp = BTreeMap::from([
        (
            "stdio".to_owned(),
            McpConfig {
                command: Some(stdio.command.clone()),
                args: stdio.args.clone(),
                url: None,
                env: BTreeMap::new(),
                ..Default::default()
            },
        ),
        (
            "web".to_owned(),
            McpConfig {
                command: None,
                args: Vec::new(),
                url: Some(http.url.clone()),
                env: BTreeMap::new(),
                ..Default::default()
            },
        ),
    ]);
    toml::to_string(&BTreeMap::from([("mcp", mcp)])).unwrap()
}

fn http_mcp_config(http: &HttpMcpFixture) -> String {
    let mcp = BTreeMap::from([(
        "http".to_owned(),
        McpConfig {
            command: None,
            args: Vec::new(),
            url: Some(http.url.clone()),
            env: BTreeMap::new(),
            ..Default::default()
        },
    )]);
    toml::to_string(&BTreeMap::from([("mcp", mcp)])).unwrap()
}

fn mixed_manifest_config(stdio: &NativeMcpFixture, http: &HttpMcpFixture) -> String {
    let mut config = Config {
        provider: "responses".into(),
        model: "fixture-model".into(),
        api_base: http.url.clone(),
        api_key_env: "KURU_MATRIX_KEY".into(),
        allow_write: true,
        allow_shell: true,
        mcp: BTreeMap::from([
            (
                "stdio".into(),
                McpConfig {
                    command: Some(stdio.command.clone()),
                    args: stdio.args.clone(),
                    url: None,
                    env: BTreeMap::new(),
                    ..Default::default()
                },
            ),
            (
                "web".into(),
                McpConfig {
                    command: None,
                    args: Vec::new(),
                    url: Some(http.url.clone()),
                    env: BTreeMap::new(),
                    ..Default::default()
                },
            ),
        ]),
        external_agents: BTreeMap::from([("reviewer".into(), format!("{}/external", http.url))]),
        ..Config::default()
    };
    config.memory.dolt_binary = Some(PathBuf::from(env!("CARGO_BIN_EXE_kuru")));
    config.memory.cache_dir = Some(stdio.directory.path().join("memory-cache"));
    toml::to_string(&config).unwrap()
}

const CLAIM_LABELS: &[&str] = &[
    "workspace write",
    "shell",
    "stdio MCP",
    "HTTP MCP",
    "memory executable",
    "memory cache",
    "Responses route",
    "external agent",
    "project instructions",
];

fn assert_claim_labels(output: &Output, expected: &[&str]) {
    assert!(!output.status.success());
    let error = text(&output.stderr);
    assert!(
        error.contains("workspace authority is not approved"),
        "{error}"
    );
    for label in CLAIM_LABELS {
        assert_eq!(
            error.contains(label),
            expected.contains(label),
            "unexpected claim label {label:?} for {expected:?}: {error}"
        );
    }
}

#[test]
fn status_revoke_and_config_are_non_creating_and_redacted() {
    let sandbox = Sandbox::new(
        r#"
allow_shell = true

[mcp.private]
command = "/do/not/disclose/program"
args = ["--secret-argument"]

[mcp.private.env]
KURU_FAKE_SECRET = "credential-value"
"#,
    );

    let status = sandbox.success(&["trust", "status"]);
    let shown = text(&status.stdout);
    assert!(shown.contains("not approved"));
    assert!(shown.contains("shell enabled"));
    assert!(
        shown.contains("stdio MCP") && shown.contains("private"),
        "{shown}"
    );
    for secret in [
        "/do/not/disclose/program",
        "--secret-argument",
        "KURU_FAKE_SECRET",
        "credential-value",
    ] {
        assert!(!shown.contains(secret), "trust output disclosed {secret:?}");
    }
    assert!(!sandbox.data.exists());

    let revoked = sandbox.success(&["trust", "revoke"]);
    assert!(text(&revoked.stdout).contains("No workspace approval"));
    assert!(!sandbox.data.exists());

    let config = sandbox.success(&["config"]);
    let parsed: toml::Value = toml::from_slice(&config.stdout).unwrap();
    assert_eq!(
        parsed["mcp"]["private"]["env"]["KURU_FAKE_SECRET"].as_str(),
        Some("[redacted]")
    );
    assert!(!text(&config.stdout).contains("credential-value"));
    assert!(text(&config.stderr).contains("saved project"));
    assert!(!sandbox.data.join("trust").exists());
    assert!(!sandbox.data.join("memory").exists());
}

#[test]
fn persistent_approval_replaces_inspects_and_revokes_its_record() {
    let sandbox = Sandbox::new("allow_shell = true\n");

    sandbox.success(&["trust", "approve", "--yes"]);
    let first_status = sandbox.success(&["trust", "status"]);
    assert!(text(&first_status.stdout).contains("Status: approved"));

    // A second explicit approval replaces the held existing record.
    sandbox.success(&["trust", "approve", "--yes"]);
    let second_status = sandbox.success(&["trust", "status"]);
    assert!(text(&second_status.stdout).contains("Status: approved"));

    let revoked = sandbox.success(&["trust", "revoke"]);
    assert!(text(&revoked.stdout).contains("Workspace approval revoked"));
    let final_status = sandbox.success(&["trust", "status"]);
    assert!(text(&final_status.stdout).contains("Status: not approved"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdio_and_http_mcp_require_cli_approval_before_activation() {
    let stdio = NativeMcpFixture::new();
    let http = HttpMcpFixture::new().await;
    let sandbox = Sandbox::new(&mcp_config(&stdio, &http));

    let refused = tokio::task::block_in_place(|| sandbox.run(&["tools"]));
    assert!(!refused.status.success());
    assert!(
        text(&refused.stderr).contains("workspace authority is not approved"),
        "{}",
        text(&refused.stderr)
    );
    assert_eq!(stdio.started(), 0, "unapproved stdio MCP started");
    assert_eq!(http.requests(), 0, "unapproved HTTP MCP connected");
    assert!(!sandbox.data.exists());

    let approved =
        tokio::task::block_in_place(|| sandbox.run(&["--trust-workspace-once", "tools"]));
    assert!(approved.status.success(), "{}", text(&approved.stderr));
    let tools: Value = serde_json::from_slice(&approved.stdout).unwrap();
    assert!(tools.as_array().is_some());
    assert_eq!(stdio.started(), 1, "approved stdio MCP did not start once");
    assert_eq!(
        stdio.completed(),
        1,
        "approved stdio MCP did not complete its protocol"
    );
    assert_eq!(
        http.requests(),
        3,
        "approved HTTP MCP did not complete initialize, notification, and discovery"
    );
    assert!(!sandbox.data.join("trust").exists());
}

#[cfg(unix)]
#[test]
fn direct_tools_keeps_stdout_json_and_reports_filtered_failed_stdio() {
    const SECRET: &str = "sk-proj-mcp-stderr-sentinel0123456789";
    let failed = NativeMcpFixture::new();
    std::fs::write(
        PathBuf::from(&failed.command).with_extension("plan"),
        format!("stderr api_key={SECRET} \u{1b}[31m\nread\nwrite not JSON\nread\n"),
    )
    .unwrap();
    let config = toml::to_string(&BTreeMap::from([(
        "mcp",
        BTreeMap::from([(
            "failed",
            McpConfig {
                command: Some(failed.command.clone()),
                ..McpConfig::default()
            },
        )]),
    )]))
    .unwrap();
    let sandbox = Sandbox::new(&config);
    let output = sandbox.success(&["--trust-workspace-once", "tools"]);
    let tools: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "file_read")
    );
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("MCP failed: configured server unavailable"),
        "{stderr}"
    );
    assert!(stderr.contains("[REDACTED:recognized-secret]"), "{stderr}");
    assert!(stderr.contains("\\x1b"), "{stderr}");
    assert!(!stderr.contains(SECRET));
    assert!(!stderr.contains(&failed.command));

    let builtin = sandbox.success(&[
        "--trust-workspace-once",
        "tool",
        "file_list",
        "--args",
        "{}",
    ]);
    assert!(
        serde_json::from_slice::<Value>(&builtin.stdout)
            .unwrap()
            .is_object()
    );
    let builtin_stderr = text(&builtin.stderr);
    assert!(builtin_stderr.contains("MCP failed: configured server unavailable"));
    assert!(builtin_stderr.contains("[REDACTED:recognized-secret]"));
    assert!(!builtin_stderr.contains(SECRET));
    assert!(!builtin_stderr.contains(&failed.command));

    let plain = sandbox.success(&[
        "--trust-workspace-once",
        "--provider",
        "demo",
        "run",
        "plain MCP degradation",
    ]);
    assert!(text(&plain.stdout).contains("demo"));
    assert!(!text(&plain.stdout).contains(SECRET));
    assert!(!text(&plain.stderr).contains(SECRET));
    assert!(!text(&plain.stderr).contains(&failed.command));

    let json_output = sandbox.success(&[
        "--trust-workspace-once",
        "--provider",
        "demo",
        "run",
        "JSON MCP degradation",
        "--json",
    ]);
    let turn: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    assert!(turn["text"].as_str().is_some());
    assert!(turn["events"].as_array().unwrap().iter().any(|event| {
        event["kind"] == "mcp"
            && event["actor"] == "failed"
            && event["detail"] == "configured server unavailable"
    }));
    for output in [&plain, &json_output] {
        let projected = format!("{}{}", text(&output.stdout), text(&output.stderr));
        assert!(!projected.contains(SECRET));
        assert!(!projected.contains(&failed.command));
        assert!(!projected.contains("[REDACTED:recognized-secret]"));
    }
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "kuru",
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
    });
    assert_eq!(failed.conversations(), vec![vec![initialize]; 4]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_cli_mcp_tool_results_are_projected_before_stdout() {
    const TOOL_TOKEN: &str = "sk-proj-abcdefghijklmnop0123456789";
    const ORDINARY_CONTROL: &str = "ordinary-control-remains-exact";
    const MARKER: &str = "[REDACTED:recognized-secret]";

    let http = HttpMcpFixture::new().await;
    let sandbox = Sandbox::new(&http_mcp_config(&http));
    tokio::task::block_in_place(|| sandbox.success(&["trust", "approve", "--yes"]));
    let tools = tokio::task::block_in_place(|| sandbox.success(&["tools"]));
    let tools: Value = serde_json::from_slice(&tools.stdout).unwrap();
    let name = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| {
            tool["description"] == "MCP http/projected_result: isolated projection fixture"
        })
        .and_then(|tool| tool["name"].as_str())
        .unwrap();
    let output = tokio::task::block_in_place(|| sandbox.success(&["tool", name, "--args", "{}"]));
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        output["content"][0]["text"],
        format!("openai_api_key={MARKER}; {ORDINARY_CONTROL}"),
    );
    assert_eq!(output["isError"], false);
    assert_eq!(
        http.calls(),
        1,
        "actual CLI did not call the isolated MCP tool"
    );
    assert!(!output.to_string().contains(TOOL_TOKEN));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_cli_mcp_tool_failures_are_projected_before_stderr() {
    const TOOL_TOKEN: &str = "sk-proj-abcdefghijklmnop0123456789";
    const MARKER: &str = "[REDACTED:recognized-secret]";

    let http = HttpMcpFixture::new().await;
    let sandbox = Sandbox::new(&http_mcp_config(&http));
    tokio::task::block_in_place(|| sandbox.success(&["trust", "approve", "--yes"]));
    let tools = tokio::task::block_in_place(|| sandbox.success(&["tools"]));
    let tools: Value = serde_json::from_slice(&tools.stdout).unwrap();
    let name = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| {
            tool["description"] == "MCP http/projected_failure: isolated projected error fixture"
        })
        .and_then(|tool| tool["name"].as_str())
        .unwrap();
    let output = tokio::task::block_in_place(|| sandbox.run(&["tool", name, "--args", "{}"]));
    let stderr = text(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        output.stdout.is_empty(),
        "unexpected stdout: {:?}",
        output.stdout
    );
    assert!(stderr.contains("MCP tool application error"), "{stderr}");
    assert!(stderr.contains(MARKER), "{stderr}");
    assert!(!stderr.contains(TOOL_TOKEN), "{stderr}");
    assert_eq!(
        http.calls(),
        1,
        "actual CLI did not call the isolated MCP error tool"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_cli_commands_enforce_the_documented_claim_matrix_before_side_effects() {
    let stdio = NativeMcpFixture::new();
    let http = HttpMcpFixture::new().await;
    let sandbox = Sandbox::new(&mixed_manifest_config(&stdio, &http));
    std::fs::write(
        sandbox.project.join("AGENTS.md"),
        "matrix automatic instructions",
    )
    .unwrap();
    let mutation = sandbox.project.join("matrix-mutation");
    let file_args =
        json!({"path": "matrix-mutation", "content": "must not be written"}).to_string();
    let all = CLAIM_LABELS;
    let tools = &CLAIM_LABELS[..6];
    let memory = &CLAIM_LABELS[4..6];
    let cases = [
        (vec!["run", "do not run"], all),
        (vec!["dream"], all),
        (vec!["serve"], all),
        (vec!["tools"], tools),
        (
            vec!["tool", "file_write", "--args", file_args.as_str()],
            tools,
        ),
        (
            vec!["models"],
            &["memory executable", "memory cache", "Responses route"][..],
        ),
        (vec!["sessions"], memory),
        (vec!["memory", "status"], memory),
        (vec!["undo-dream"], memory),
        (vec!["auth"], &["Responses route"][..]),
        (
            vec!["--allow-shell", "tools"],
            &[
                "workspace write",
                "stdio MCP",
                "HTTP MCP",
                "memory executable",
                "memory cache",
            ][..],
        ),
        (
            vec!["--provider", "demo", "models"],
            &["memory executable", "memory cache"][..],
        ),
    ];
    for (args, expected) in cases {
        let output = tokio::task::block_in_place(|| sandbox.run(&args));
        assert_claim_labels(&output, expected);
        assert!(!mutation.exists(), "{args:?} mutated the workspace");
        assert_eq!(stdio.started(), 0, "{args:?} started stdio MCP");
        assert_eq!(http.requests(), 0, "{args:?} opened an HTTP route");
        assert!(!sandbox.data.exists(), "{args:?} created private state");
    }

    let status = tokio::task::block_in_place(|| sandbox.run(&["trust", "status"]));
    assert!(status.status.success(), "{}", text(&status.stderr));
    let shown = text(&status.stdout);
    for label in CLAIM_LABELS {
        assert!(shown.contains(label), "status omitted {label:?}: {shown}");
    }
    let config = tokio::task::block_in_place(|| sandbox.run(&["config"]));
    assert!(config.status.success(), "{}", text(&config.stderr));
    toml::from_slice::<toml::Value>(&config.stdout).unwrap();
    let revoke = tokio::task::block_in_place(|| sandbox.run(&["trust", "revoke"]));
    assert!(revoke.status.success(), "{}", text(&revoke.stderr));
    assert!(text(&revoke.stdout).contains("No workspace approval"));
    let update = tokio::task::block_in_place(|| sandbox.run(&["update"]));
    assert!(!update.status.success());
    assert!(
        text(&update.stderr).contains("provide --version VERSION"),
        "{}",
        text(&update.stderr)
    );
    assert!(!text(&update.stderr).contains("workspace authority"));
    assert!(!mutation.exists());
    assert_eq!(stdio.started(), 0);
    assert_eq!(http.requests(), 0);
    assert!(!sandbox.data.exists());

    let ordinary = Sandbox::new("max_rounds = 4\n");
    let output = tokio::task::block_in_place(|| ordinary.run(&["--provider", "demo", "models"]));
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(!text(&output.stderr).contains("workspace authority"));
    assert!(!ordinary.data.exists());
}

#[test]
fn automatic_instruction_sources_are_ordered_safe_and_stale_complete_approval() {
    let sandbox = Sandbox::new("");
    let outer = sandbox.root.path().join("AGENTS.md");
    let root = sandbox.project.join("AGENTS.md");
    std::fs::write(&outer, "OUTER-INSTRUCTION-CONTENT").unwrap();
    std::fs::write(&root, "ROOT-INSTRUCTION-CONTENT").unwrap();

    let status = sandbox.success(&["trust", "status"]);
    let shown = text(&status.stdout);
    assert!(shown.contains("project instructions: 2 ordered automatic sources"));
    let outer_label = safe_source_label(&outer);
    let root_label = safe_source_label(&root);
    assert!(shown.find(&outer_label).unwrap() < shown.find(&root_label).unwrap());
    assert!(!shown.contains("OUTER-INSTRUCTION-CONTENT"));
    assert!(!shown.contains("ROOT-INSTRUCTION-CONTENT"));

    // Commands that do not construct peer prompts do not consume instruction
    // authority, and they do not create private state merely to inspect it.
    for args in [
        &["config"][..],
        &["--provider", "demo", "models"][..],
        &["tools"][..],
        &["--provider", "demo", "auth"][..],
    ] {
        let output = sandbox.success(args);
        assert!(!text(&output.stderr).contains("workspace authority"));
    }
    assert!(!sandbox.data.exists());

    let refused = sandbox.run(&["--provider", "demo", "run", "must not activate"]);
    assert_claim_labels(&refused, &["project instructions"]);
    assert!(!sandbox.data.exists());
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let mut command = sandbox.command();
        command
            .env("OPENAI_API_KEY", OsString::from_vec(vec![0xff, 0xfe]))
            .args([
                "--provider",
                "responses",
                "--model",
                "fixture-model",
                "run",
                "must not construct provider",
            ]);
        let refused = command.output().unwrap();
        assert_claim_labels(&refused, &["project instructions"]);
        assert!(!text(&refused.stderr).contains("valid text"));
        assert!(!sandbox.data.exists());
    }

    sandbox.success(&["trust", "approve", "--yes"]);
    assert!(text(&sandbox.success(&["trust", "status"]).stdout).contains("Status: approved"));

    std::fs::write(&root, "ROOT-CHANGED").unwrap();
    assert!(text(&sandbox.success(&["trust", "status"]).stdout).contains("does not match"));
    sandbox.success(&["trust", "approve", "--yes"]);

    std::fs::remove_file(&outer).unwrap();
    assert!(text(&sandbox.success(&["trust", "status"]).stdout).contains("does not match"));
    sandbox.success(&["trust", "approve", "--yes"]);

    std::fs::write(&outer, "OUTER-RETURNED").unwrap();
    assert!(text(&sandbox.success(&["trust", "status"]).stdout).contains("does not match"));
    sandbox.success(&["trust", "approve", "--yes"]);

    let parked = sandbox.project.join("parked-agents");
    std::fs::rename(&root, &parked).unwrap();
    std::fs::write(&root, "ROOT-CHANGED").unwrap();
    assert!(
        text(&sandbox.success(&["trust", "status"]).stdout).contains("does not match"),
        "same-byte native replacement must stale approval"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_injects_only_the_instruction_bytes_owned_by_the_reviewed_snapshot() {
    let sandbox = Sandbox::new("");
    let outer = sandbox.root.path().join("AGENTS.md");
    let root = sandbox.project.join("AGENTS.md");
    std::fs::write(&outer, "OUTER-REVIEWED-BYTES").unwrap();
    std::fs::write(&root, "ROOT-REVIEWED-BYTES").unwrap();
    let snapshot = ConfigSnapshot::parse(
        None,
        &sandbox.project,
        None,
        InvocationOverrides {
            provider: Some("demo".into()),
            model: Some("demo".into()),
            no_dream: true,
            ..InvocationOverrides::default()
        },
    )
    .unwrap();
    let reviewed = snapshot.instructions().to_owned();
    let config = snapshot.finalize(&ProjectPreferences::default()).unwrap();

    std::fs::write(&outer, "OUTER-UNREVIEWED-BYTES").unwrap();
    std::fs::write(&root, "ROOT-UNREVIEWED-BYTES").unwrap();

    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = Arc::new(RecordingDemo {
        instructions: requests.clone(),
    });
    let memory = MemoryStore::temporary().await.unwrap();
    let tools = ToolHost::new(&sandbox.project, &config).unwrap();
    let mut harness = Harness::with_tool_host_and_instructions(
        config,
        &sandbox.project,
        reviewed,
        memory.clone(),
        provider,
        None,
        tools,
    )
    .await
    .unwrap();
    harness
        .run("verify reviewed instruction bytes")
        .await
        .unwrap();
    harness.shutdown(true).await.unwrap();
    memory.close().await.unwrap();

    let requests = requests.lock().unwrap();
    assert!(!requests.is_empty());
    for instructions in requests.iter() {
        assert!(instructions.contains("OUTER-REVIEWED-BYTES"));
        assert!(instructions.contains("ROOT-REVIEWED-BYTES"));
        assert!(!instructions.contains("UNREVIEWED-BYTES"));
        assert!(
            instructions.find("OUTER-REVIEWED-BYTES").unwrap()
                < instructions.find("ROOT-REVIEWED-BYTES").unwrap()
        );
    }
}

#[cfg(unix)]
#[test]
fn one_shot_is_subset_only_and_persistent_approval_binds_the_full_manifest() {
    let sandbox = Sandbox::new("allow_shell = true\nmax_rounds = 3\n");
    let marker = sandbox.root.path().join("shell-marker");
    let arguments = serde_json::json!({
        "command": format!("printf reached > '{}'", marker.display())
    })
    .to_string();

    let refused = sandbox.run(&["tool", "shell", "--args", &arguments]);
    assert!(!refused.status.success());
    assert!(text(&refused.stderr).contains("--trust-workspace-once"));
    assert!(!marker.exists());
    assert!(!sandbox.data.exists());

    sandbox.success(&[
        "--trust-workspace-once",
        "tool",
        "shell",
        "--args",
        &arguments,
    ]);
    assert!(marker.exists());
    assert!(!sandbox.data.join("trust").exists());

    sandbox.success(&["trust", "approve", "--yes"]);
    let store = sandbox.data.join("trust/workspaces");
    let records: Vec<_> = std::fs::read_dir(&store)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|value| value == "json"))
        .collect();
    assert_eq!(records.len(), 1);
    let record = std::fs::read(&records[0]).unwrap();
    assert!(
        !record
            .windows("allow_shell".len())
            .any(|part| part == b"allow_shell")
    );
    assert!(
        !record
            .windows(marker.as_os_str().as_encoded_bytes().len())
            .any(|part| { part == marker.as_os_str().as_encoded_bytes() })
    );

    std::fs::remove_file(&marker).unwrap();
    sandbox.success(&["tool", "shell", "--args", &arguments]);
    assert!(marker.exists());

    // Ordinary settings do not change the authority manifest.
    sandbox.write_config("allow_shell = true\nmax_rounds = 4\n");
    std::fs::remove_file(&marker).unwrap();
    sandbox.success(&["tool", "shell", "--args", &arguments]);
    assert!(marker.exists());

    // Any authority change invalidates the complete stored grant, even when
    // this invocation would otherwise use only the unchanged shell claim.
    sandbox.write_config("allow_shell = true\nallow_write = true\nmax_rounds = 4\n");
    std::fs::remove_file(&marker).unwrap();
    let invalidated = sandbox.run(&["tool", "shell", "--args", &arguments]);
    assert!(!invalidated.status.success());
    assert!(!marker.exists());
    assert!(text(&invalidated.stderr).contains("does not match"));
}

#[test]
fn approval_is_bound_to_the_exact_path() {
    let sandbox = Sandbox::new("allow_shell = true\n");
    sandbox.success(&["trust", "approve", "--yes"]);
    let other = sandbox.root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    std::fs::create_dir(other.join(".kuru")).unwrap();
    std::fs::write(other.join(".kuru/config.toml"), "allow_shell = true\n").unwrap();
    let output = sandbox
        .command_at(&other)
        .args(["trust", "status"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("not approved"));
}

#[cfg(unix)]
#[test]
fn approval_is_bound_to_native_identity_at_the_same_path() {
    let sandbox = Sandbox::new("allow_shell = true\n");
    sandbox.success(&["trust", "approve", "--yes"]);
    let displaced = sandbox.root.path().join("displaced-project");
    std::fs::rename(&sandbox.project, &displaced).unwrap();
    std::fs::create_dir(&sandbox.project).unwrap();
    std::fs::create_dir(sandbox.project.join(".kuru")).unwrap();
    std::fs::write(
        sandbox.project.join(".kuru/config.toml"),
        "allow_shell = true\n",
    )
    .unwrap();
    let status = sandbox.success(&["trust", "status"]);
    assert!(text(&status.stdout).contains("does not match"));
}

#[test]
fn malformed_configuration_diagnostics_do_not_echo_values_or_controls() {
    let sandbox = Sandbox::new("allow_shell = \"fake-secret-123\u{1b}[31m\n");
    let revoked = sandbox.success(&["trust", "revoke"]);
    assert!(text(&revoked.stdout).contains("No workspace approval"));
    let mut logout = sandbox.command();
    logout.args(["--provider", "responses", "logout"]);
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        logout.env("OPENAI_API_KEY", OsString::from_vec(vec![0xff, 0xfe]));
    }
    let logout = logout.output().unwrap();
    assert!(logout.status.success(), "{}", text(&logout.stderr));
    assert!(text(&logout.stdout).contains("Signed out"));
    let output = sandbox.run(&["config"]);
    assert!(!output.status.success());
    let error = text(&output.stderr);
    assert!(error.contains("configuration parse error"), "{error}");
    assert!(!error.contains("fake-secret-123"), "{error}");
    assert!(!error.contains('\u{1b}'), "{error:?}");
    assert!(!sandbox.data.join("trust").exists());
    assert!(!sandbox.data.join("memory").exists());
}

#[cfg(unix)]
#[test]
fn pending_responses_environment_is_not_read_before_approval() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let sandbox = Sandbox::new(
        r#"
provider = "responses"
model = "explicit-model"
api_base = "http://127.0.0.1:9/v1?hidden=query-secret"
api_key_env = "KURU_UNREADABLE_FAKE_KEY"
"#,
    );
    let mut command = sandbox.command();
    command
        .env(
            "KURU_UNREADABLE_FAKE_KEY",
            OsString::from_vec(vec![0xff, 0xfe]),
        )
        .arg("auth");
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let error = text(&output.stderr);
    assert!(
        error.contains("workspace authority is not approved"),
        "{error}"
    );
    assert!(!error.contains("query-secret"), "{error}");
    assert!(!error.contains("KURU_UNREADABLE_FAKE_KEY"), "{error}");
    assert!(!error.contains("valid text"), "{error}");
    assert!(!sandbox.data.exists());

    let mut command = sandbox.command();
    command
        .env(
            "KURU_UNREADABLE_FAKE_KEY",
            OsString::from_vec(vec![0xff, 0xfe]),
        )
        .args(["--trust-workspace-once", "auth"]);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    let error = text(&output.stderr);
    assert!(error.contains("must contain valid text"), "{error}");
    assert!(!error.contains("query-secret"), "{error}");
    assert!(!error.contains("KURU_UNREADABLE_FAKE_KEY"), "{error}");
    assert!(!sandbox.data.exists());
}

#[test]
fn cli_type_and_semantic_validation_errors_are_coarse_and_redacted() {
    for (config, category, secrets) in [
        (
            "model = \"control\\u001bTYPE_MODEL_SECRET\"\nallow_shell = \"TYPE_VALUE_SECRET\"\n",
            "configuration type error",
            ["TYPE_MODEL_SECRET", "TYPE_VALUE_SECRET"],
        ),
        (
            "model = \"control\\u001bVALIDATION_MODEL_SECRET\"\n[memory]\nstartup_timeout_secs = 0\n",
            "configuration validation error",
            ["VALIDATION_MODEL_SECRET", "startup_timeout_secs"],
        ),
    ] {
        let sandbox = Sandbox::new(config);
        let output = sandbox.run(&["config"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = text(&output.stderr);
        assert!(error.contains(category), "{error}");
        assert!(!error.contains('\u{1b}'), "{error:?}");
        for secret in secrets {
            assert!(
                !error.contains(secret),
                "diagnostic disclosed {secret:?}: {error}"
            );
        }
        assert!(!sandbox.data.exists());
    }
}

#[cfg(unix)]
#[test]
fn inactive_codex_auth_does_not_read_the_configured_responses_environment() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let sandbox = Sandbox::new("provider = 'codex'\napi_key_env = 'KURU_DORMANT_RESPONSES_KEY'\n");
    let mut command = sandbox.command();
    command
        .env(
            "KURU_DORMANT_RESPONSES_KEY",
            OsString::from_vec(vec![0xff, 0xfe]),
        )
        .arg("auth");
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["api_key_available"], false);
    assert!(text(&output.stderr).contains("was not checked"));
    assert!(!text(&output.stderr).contains("KURU_DORMANT_RESPONSES_KEY"));
    assert!(!sandbox.data.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reached_undo_uses_only_approved_memory_authority_and_no_provider_route() {
    let http = HttpMcpFixture::new().await;
    let sandbox = Sandbox::new("");
    let scope = kuru_runtime::project_scope(&sandbox.project).unwrap();
    let options =
        kuru_memory::test_support::open_options(sandbox.data.clone(), scope.clone()).unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    let mut harness = Harness::new(
        Config {
            provider: "demo".into(),
            model: "demo".into(),
            dream_every: 0,
            dream_on_exit: false,
            ..Config::default()
        },
        &sandbox.project,
        memory.clone(),
        Arc::new(DemoProvider),
        None,
    )
    .await
    .unwrap();
    let role = harness.topology.parts[0].role.clone();
    harness
        .apply_dream(vec![DreamProposal::Add {
            name: "Later observer".into(),
            role,
            instruction: "Preserve later private context".into(),
        }])
        .await
        .unwrap();
    let added = harness.resolve("Later observer").unwrap();
    let namespace = harness.namespace(&added);
    memory
        .append(&namespace, "user", "later private conversation")
        .await
        .unwrap();
    harness.run("later session conversation").await.unwrap();
    let session = harness.session.id.clone();
    let sessions = memory.get(&format!("{scope}/sessions")).await.unwrap();
    let before_revision = memory.revision().await.unwrap();
    harness.shutdown(false).await.unwrap();
    drop(harness);
    memory.close().await.unwrap();

    let mut config = Config {
        provider: "responses".into(),
        model: "fixture-model".into(),
        api_base: http.url.clone(),
        api_key_env: "KURU_UNREADABLE_UNDO_KEY".into(),
        ..Config::default()
    };
    config.memory.cache_dir = Some(kuru_memory::test_support::cache_dir());
    config.memory.offline = true;
    sandbox.write_config(&toml::to_string(&config).unwrap());

    let refused =
        tokio::task::block_in_place(|| sandbox.run(&["--resume", &session, "undo-dream"]));
    assert!(!refused.status.success());
    assert!(
        text(&refused.stderr).contains("workspace authority is not approved"),
        "{}",
        text(&refused.stderr)
    );
    assert_eq!(http.requests(), 0, "refused undo contacted the provider");
    assert!(!sandbox.data.join("trust").exists());

    let mut command = sandbox.command();
    command.args(["--trust-workspace-once", "--resume", &session, "undo-dream"]);
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        command.env(
            "KURU_UNREADABLE_UNDO_KEY",
            OsString::from_vec(vec![0xff, 0xfe]),
        );
    }
    let approved = tokio::task::block_in_place(|| command.output().unwrap());
    assert!(approved.status.success(), "{}", text(&approved.stderr));
    assert!(text(&approved.stdout).contains("Previous membership restored"));
    assert_eq!(http.requests(), 0, "approved undo contacted the provider");
    assert!(!sandbox.data.join("trust").exists());

    // The approved CLI command owns the intentionally warm managed service.
    // This final phase only inspects its committed result; a direct writable
    // reopen would compete with that owner instead of testing the CLI state.
    let mut observed_options = options;
    observed_options.read_only = true;
    let project = sandbox.project.canonicalize().unwrap();
    let (_, opening) = MemoryStore::open_managed_observed(
        observed_options,
        project,
        PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
    );
    let memory = opening.await.unwrap();
    assert_ne!(memory.revision().await.unwrap(), before_revision);
    assert_eq!(
        memory.get(&format!("{scope}/sessions")).await.unwrap(),
        sessions
    );
    assert!(
        memory
            .history(&namespace, 10)
            .await
            .unwrap()
            .iter()
            .any(|message| message.plain_text() == Some("later private conversation"))
    );
    let topology: Topology = serde_json::from_value(
        memory
            .get(&format!("{scope}/ifs/topology"))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        !topology
            .parts
            .iter()
            .find(|part| part.id == added)
            .unwrap()
            .active
    );
    memory.close().await.unwrap();
}

#[cfg(unix)]
#[test]
fn tools_and_models_gate_configured_memory_before_legacy_probe_or_engine_launch() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("");
    let engine = sandbox.root.path().join("fake-dolt");
    let marker = sandbox.root.path().join("engine-started");
    std::fs::write(
        &engine,
        format!(
            "#!/bin/sh\nprintf started > '{}'\nexit 1\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o700)).unwrap();
    sandbox.write_config(&format!(
        "provider = 'demo'\n[memory]\ndolt_binary = {:?}\n",
        engine.to_str().unwrap()
    ));
    std::fs::create_dir(&sandbox.data).unwrap();
    let legacy = sandbox.data.join("memory.sqlite3");
    let sentinel = b"not-a-database-until-approved";
    std::fs::write(&legacy, sentinel).unwrap();

    for command in [["models"].as_slice(), ["tools"].as_slice()] {
        let output = sandbox.run(command);
        assert!(!output.status.success(), "{command:?}");
        assert!(
            text(&output.stderr).contains("workspace authority is not approved"),
            "{}",
            text(&output.stderr)
        );
        assert!(
            !marker.exists(),
            "{command:?} launched the configured engine"
        );
        assert_eq!(std::fs::read(&legacy).unwrap(), sentinel);
        assert!(!sandbox.data.join("memory").exists());
        assert!(!sandbox.data.join("trust").exists());
    }
}

#[cfg(unix)]
fn terminal_command(sandbox: &Sandbox) -> ProcessCommand {
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_kuru"));
    command
        .arg("-C")
        .arg(&sandbox.project)
        .arg("--data-dir")
        .arg(&sandbox.data)
        .env("XDG_CONFIG_HOME", sandbox.root.path().join("config"))
        .env("TERM", "xterm-256color");
    command
}

#[cfg(unix)]
fn wait_for_refusal(terminal: &mut terminal::Terminal) {
    let error = terminal
        .wait("trust command exits", Duration::from_secs(5), |_| Ok(false))
        .unwrap_err();
    assert!(error.to_string().contains("process exited"), "{error}");
    assert!(
        !terminal
            .output
            .windows(8)
            .any(|bytes| bytes == b"\x1b[?1049h"),
        "trust preflight entered the alternate screen"
    );
}

#[cfg(unix)]
#[test]
fn native_login_bypasses_workspace_responses_config_and_cancels_without_state() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let sandbox = Sandbox::new("allow_shell = \"malformed-login-secret\u{1b}[31m\n");
    let mut command = terminal_command(&sandbox);
    command
        .env("OPENAI_API_KEY", OsString::from_vec(vec![0xff, 0xfe]))
        .args(["--provider", "responses", "login", "--no-browser"]);
    let mut terminal = terminal::Terminal::spawn(command, 20, 120).unwrap();
    terminal
        .wait_text(
            &["Sign in to ChatGPT", "auth.openai.com/oauth/authorize"],
            &[],
        )
        .unwrap();
    assert!(!sandbox.data.exists());
    terminal.interrupt().unwrap();
    let error = terminal
        .wait("login cancellation exits", Duration::from_secs(5), |_| {
            Ok(false)
        })
        .unwrap_err();
    assert!(error.to_string().contains("process exited"), "{error}");
    assert!(
        String::from_utf8_lossy(&terminal.output).contains("login cancelled"),
        "{}",
        terminal.screen()
    );
    assert!(
        !terminal
            .output
            .windows(8)
            .any(|bytes| bytes == b"\x1b[?1049h")
    );
    assert!(!sandbox.data.exists());
}

#[cfg(unix)]
#[test]
fn real_pty_refusal_eof_and_persistent_choice_precede_the_alternate_screen() {
    for response in [b"3\r".as_slice(), b"\x04".as_slice()] {
        let sandbox = Sandbox::new("allow_shell = true\n");
        let mut terminal = terminal::Terminal::spawn(terminal_command(&sandbox), 35, 120).unwrap();
        terminal
            .wait_text(
                &["Continue once", "Approve this complete configuration"],
                &[],
            )
            .unwrap();
        assert!(!sandbox.data.exists());
        terminal.send(response).unwrap();
        wait_for_refusal(&mut terminal);
        assert!(!sandbox.data.exists());
    }

    // A persistent pre-TUI choice records the complete manifest, then this
    // deliberately unavailable provider route fails before alternate-screen
    // entry. The subsequent status command proves the full record was stored.
    let sandbox = Sandbox::new(
        "allow_shell = true\nprovider = 'responses'\nmodel = 'fixture-model'\napi_key_env = 'KURU_ABSENT_PTY_KEY'\n",
    );
    let mut command = terminal_command(&sandbox);
    command.env_remove("KURU_ABSENT_PTY_KEY");
    let mut terminal = terminal::Terminal::spawn(command, 35, 120).unwrap();
    terminal
        .wait_text(
            &["Continue once", "Approve this complete configuration"],
            &[],
        )
        .unwrap();
    terminal.send(b"2\r").unwrap();
    wait_for_refusal(&mut terminal);
    let status = sandbox.success(&["trust", "status"]);
    assert!(text(&status.stdout).contains("Status: approved"));
}
