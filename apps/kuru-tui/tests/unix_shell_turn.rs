#![cfg(unix)]

use std::{io, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::State,
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

    fn command(&self) -> Command {
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
            ])
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
}

async fn complete(
    State(state): State<Arc<ProviderState>>,
    Json(request): Json<Value>,
) -> Json<Value> {
    state.requests.lock().await.push(request.clone());
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
            "arguments": serde_json::to_string(&json!({"command": format!("printf {SHELL_MARKER}")})).expect("fixture arguments serialize"),
        }])
    } else {
        json!([{"type":"message","content":[{"type":"output_text","text":"fixture peer contribution"}]}])
    };
    Json(json!({
        "status":"completed",
        "output":output,
        "usage":{"input_tokens":8,"output_tokens":5},
    }))
}

struct Server {
    url: String,
    state: Arc<ProviderState>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), io::Error>>,
}

impl Server {
    async fn start() -> Result<Self> {
        let state = Arc::new(ProviderState::default());
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

async fn run_cli(provider_url: String) -> Result<std::process::Output> {
    tokio::task::spawn_blocking(move || {
        // If this test future is cancelled, Tokio leaves the started blocking
        // worker running. It retains the complete sandbox through bounded
        // helper completion, including any reported cleanup failure.
        let sandbox = Sandbox::new(&provider_url)?;
        let mut command = sandbox.command();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create retained CLI fixture runtime")?
            .block_on(bounded_output(&mut command, CLI_TIMEOUT, CAPTURE_LIMIT))
            .context("run bounded kuru CLI fixture")
    })
    .await
    .context("retained CLI fixture worker panicked")?
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
        let output = run_cli(server.url.clone()).await?;
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
            "expected one shell call request, got {}",
            shell_requests.len()
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
        Ok(())
    }
    .await;
    let shutdown = server.shutdown().await;
    result?;
    shutdown
}
