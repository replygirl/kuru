#![cfg(unix)]

use std::{
    io,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use kuru_delivery::command::{Command, bounded_output};
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, oneshot},
    task::JoinHandle,
    time::timeout,
};

#[path = "support/memory.rs"]
mod memory;

const CLI_TIMEOUT: Duration = Duration::from_secs(45);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const CAPTURE_LIMIT: usize = 128 * 1024;
const SHELL_MARKER: &str = "SHELL_TURN_RECEIPT";
const FINAL_TEXT: &str = "SHELL_TURN_FINAL_TEXT";
const PROVIDER_SECRET: &str = "sk-proj-tracing-provider-secret-0123456789";

struct Sandbox {
    root: tempfile::TempDir,
    project: PathBuf,
    data: PathBuf,
    provider_config: PathBuf,
}

impl Sandbox {
    fn new(provider_url: &str) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project)?;
        memory::configuration(root.path())?;
        let provider_config = root.path().join("fixture-responses.toml");
        std::fs::write(
            &provider_config,
            format!("api_base = {provider_url:?}\napi_key_env = 'KURU_SHELL_TURN_FIXTURE_KEY'\n"),
        )?;
        Ok(Self {
            root,
            project,
            data,
            provider_config,
        })
    }

    fn command(&self, debug: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        command
            .arg("-C")
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.data)
            .arg("--config")
            .arg(&self.provider_config)
            .args([
                "--provider",
                "responses",
                "--model",
                "fixture",
                "--allow-shell",
                "--no-dream",
                "--trust-workspace-once",
                "run",
                "exercise the built-in shell once",
                "--json",
            ]);
        if debug {
            command.arg("--debug");
        }
        command
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("KURU_SHELL_TURN_FIXTURE_KEY", "fixture-key")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

#[derive(Default)]
struct ProviderState {
    requests: Mutex<Vec<Value>>,
    completion_attempts: AtomicUsize,
    failing_shell: bool,
}

async fn complete(State(state): State<Arc<ProviderState>>, Json(request): Json<Value>) -> Response {
    let attempt = state.completion_attempts.fetch_add(1, Ordering::SeqCst);
    state.requests.lock().await.push(request.clone());
    if attempt == 0 {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":{"message":PROVIDER_SECRET}})),
        )
            .into_response();
    }
    let shell_is_available = request["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "shell"));
    let is_continuation = request["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    let output = if is_continuation {
        json!([{"type":"message","content":[{"type":"output_text","text":FINAL_TEXT}]}])
    } else if shell_is_available {
        json!([{
            "type":"function_call",
            "call_id":"shell-turn",
            "name":"shell",
            "arguments": serde_json::to_string(&json!({
                "command": if state.failing_shell {
                    format!("printf {PROVIDER_SECRET} >&2; exit 7")
                } else {
                    format!("printf {SHELL_MARKER}")
                }
            })).expect("fixture arguments serialize"),
        }])
    } else {
        json!([{"type":"message","content":[{"type":"output_text","text":"fixture peer contribution"}]}])
    };
    Json(json!({
        "status":"completed",
        "output":output,
        "usage":{"input_tokens":8,"output_tokens":5},
    }))
    .into_response()
}

struct Server {
    url: String,
    state: Arc<ProviderState>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), io::Error>>,
}

impl Server {
    async fn start() -> Result<Self> {
        Self::start_with_shell_failure(false).await
    }

    async fn start_with_shell_failure(failing_shell: bool) -> Result<Self> {
        let state = Arc::new(ProviderState {
            requests: Mutex::new(vec![]),
            completion_attempts: AtomicUsize::new(0),
            failing_shell,
        });
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(complete))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/v1", listener.local_addr()?);
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await
        });
        Ok(Self {
            url,
            state,
            stop: Some(stop),
            task,
        })
    }

    async fn shutdown(mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match timeout(CLEANUP_TIMEOUT, &mut self.task).await {
            Ok(result) => {
                result.context("fake Responses server task panicked")??;
                Ok(())
            }
            Err(_) => {
                self.task.abort();
                let _ = timeout(CLEANUP_TIMEOUT, &mut self.task)
                    .await
                    .context("aborted fake Responses server did not stop")?;
                anyhow::bail!(
                    "fake Responses server did not stop within {CLEANUP_TIMEOUT:?}; aborted and reaped"
                );
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct CliRun {
    output: std::process::Output,
    sandbox: Sandbox,
}

async fn run_cli(provider_url: String, debug: bool) -> Result<CliRun> {
    tokio::task::spawn_blocking(move || {
        // If this test future is cancelled, Tokio leaves the started blocking
        // worker running. It retains the complete sandbox through bounded
        // helper completion, including any reported cleanup failure.
        let sandbox = Sandbox::new(&provider_url)?;
        let mut command = sandbox.command(debug);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create retained CLI fixture runtime")?
            .block_on(bounded_output(&mut command, CLI_TIMEOUT, CAPTURE_LIMIT))
            .context("run bounded kuru CLI fixture")
            .map(|output| CliRun { output, sandbox })
    })
    .await
    .context("retained CLI fixture worker panicked")?
}

fn diagnostics(run: &CliRun) -> Result<String> {
    let diagnostics = run.sandbox.data.join("diagnostics");
    std::fs::read_dir(&diagnostics)
        .context("read private diagnostics root")?
        .flatten()
        .map(|entry| entry.path())
        .flat_map(|scope| {
            std::fs::read_dir(scope)
                .into_iter()
                .flat_map(|entries| entries.flatten())
        })
        .map(|entry| std::fs::read_to_string(entry.path()))
        .collect::<std::io::Result<String>>()
        .map_err(Into::into)
}

fn expected_receipt() -> String {
    json!({
        "exit_code": 0,
        "success": true,
        "stdout": SHELL_MARKER,
        "stderr": "",
    })
    .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kuru_run_uses_the_owned_shell_and_preserves_the_responses_continuation() -> Result<()> {
    let server = Server::start().await?;
    let result = async {
        let run = run_cli(server.url.clone(), true).await?;
        let output = &run.output;
        ensure!(
            output.status.success(),
            "kuru run failed with {}; stdout {:?}; stderr {:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let turn: Value = serde_json::from_slice(&output.stdout).context("parse kuru run JSON")?;
        ensure!(
            turn["text"] == FINAL_TEXT,
            "unexpected terminal turn: {turn}"
        );
        let requests = server.state.requests.lock().await.clone();
        let shell_requests = requests
            .iter()
            .filter(|request| {
                request["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "shell"))
                    && !request["input"].as_array().is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["type"] == "function_call_output")
                    })
            })
            .collect::<Vec<_>>();
        ensure!(
            shell_requests.len() == 1,
            "expected one successful shell call request, got {}",
            shell_requests.len()
        );
        ensure!(
            requests.len() >= 3,
            "expected provider work plus a continuation, got {} requests",
            requests.len()
        );
        let continuations = requests
            .iter()
            .filter_map(|request| request["input"].as_array())
            .flat_map(|items| items.iter())
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        ensure!(
            continuations.len() == 1,
            "expected one shell continuation, got {}",
            continuations.len()
        );
        ensure!(continuations[0]["call_id"] == "shell-turn");
        ensure!(
            continuations[0]["output"] == expected_receipt(),
            "unexpected typed shell receipt: {}",
            continuations[0],
        );
        ensure!(
            server.state.completion_attempts.load(Ordering::SeqCst) >= 3,
            "expected a retried provider request and a shell continuation"
        );
        let logs = diagnostics(&run)?;
        ensure!(logs.contains("\"target\":\"kuru.runtime\""));
        ensure!(logs.contains("\"target\":\"kuru.actor\""));
        ensure!(logs.contains("\"target\":\"kuru.provider\""));
        ensure!(logs.contains("\"status\":503"));
        ensure!(logs.contains("\"target\":\"kuru.tool\""));
        ensure!(
            logs.contains("\"session\":")
                && logs.contains("\"turn\":")
                && logs.contains("\"actor\":")
        );
        ensure!(logs.contains("\"status\":\"ok\""));
        ensure!(logs.contains("span_open") && logs.contains("span_close"));
        ensure!(
            !logs.contains(PROVIDER_SECRET),
            "provider error secret leaked to diagnostics: {logs}"
        );
        ensure!(
            !logs.contains("fixture-key"),
            "provider API key leaked to diagnostics: {logs}"
        );
        let normal = run_cli(server.url.clone(), false).await?;
        ensure!(
            normal.output.status.success() && normal.output.stderr.is_empty(),
            "normal kuru run changed diagnostics presentation: stdout {:?}; stderr {:?}",
            String::from_utf8_lossy(&normal.output.stdout),
            String::from_utf8_lossy(&normal.output.stderr),
        );
        let mut debug_turn = turn.clone();
        let mut normal_turn: Value =
            serde_json::from_slice(&normal.output.stdout).context("parse normal kuru run JSON")?;
        debug_turn
            .as_object_mut()
            .context("debug turn must be an object")?
            .remove("session");
        normal_turn
            .as_object_mut()
            .context("normal turn must be an object")?
            .remove("session");
        ensure!(
            normal_turn == debug_turn,
            "--debug changed the JSON turn beyond its newly generated session identity"
        );
        let normal_logs = diagnostics(&normal)?;
        ensure!(
            !normal_logs.contains("span_open") && !normal_logs.contains("span_close"),
            "normal diagnostics included debug span rows"
        );
        Ok(())
    }
    .await;
    let shutdown = server.shutdown().await;
    result?;
    shutdown
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn normal_cli_trace_redacts_a_failing_owned_shell_receipt() -> Result<()> {
    let server = Server::start_with_shell_failure(true).await?;
    let result = async {
        let run = run_cli(server.url.clone(), false).await?;
        ensure!(
            run.output.status.success(),
            "kuru run failed with {}; stdout {:?}; stderr {:?}",
            run.output.status,
            String::from_utf8_lossy(&run.output.stdout),
            String::from_utf8_lossy(&run.output.stderr),
        );
        ensure!(
            run.output.stderr.is_empty(),
            "normal diagnostics wrote to stderr"
        );
        let turn: Value =
            serde_json::from_slice(&run.output.stdout).context("parse kuru run JSON")?;
        ensure!(
            turn["text"] == FINAL_TEXT,
            "unexpected terminal turn: {turn}"
        );
        let requests = server.state.requests.lock().await.clone();
        let receipts = requests
            .iter()
            .filter_map(|request| request["input"].as_array())
            .flat_map(|items| items.iter())
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        ensure!(receipts.len() == 1, "expected one failing shell receipt");
        let receipt = receipts[0]["output"]
            .as_str()
            .context("shell receipt must remain typed text")?;
        ensure!(
            receipt.contains("[REDACTED:recognized-secret]"),
            "receipt did not preserve the complete shell redaction marker: {receipt}"
        );
        ensure!(
            !receipt.contains(PROVIDER_SECRET),
            "raw shell secret reached provider continuation"
        );
        let logs = diagnostics(&run)?;
        ensure!(logs.contains("\"target\":\"kuru.tool\""));
        ensure!(logs.contains("\"status\":\"error\""));
        ensure!(!logs.contains("span_open") && !logs.contains("span_close"));
        ensure!(
            !logs.contains(PROVIDER_SECRET),
            "shell secret leaked to normal diagnostics: {logs}"
        );
        ensure!(
            !logs.contains("printf"),
            "shell command leaked to normal diagnostics: {logs}"
        );
        Ok(())
    }
    .await;
    let shutdown = server.shutdown().await;
    result?;
    shutdown
}
