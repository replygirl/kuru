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
    sync::{Mutex, Notify, oneshot},
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};

#[path = "support/memory.rs"]
mod memory;

const CLI_TIMEOUT: Duration = Duration::from_secs(45);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const CAPTURE_LIMIT: usize = 128 * 1024;
const SHELL_MARKER: &str = "SHELL_TURN_RECEIPT";
const FINAL_TEXT: &str = "SHELL_TURN_FINAL_TEXT";
const PROVIDER_SECRET: &str = "sk-proj-tracing-provider-secret-0123456789";
const ROTATION_TOOL_CALLS: usize = 768;
const DIAGNOSTIC_FILE_BYTES: u64 = 64 * 1024;
const DIAGNOSTIC_FILE_COUNT: usize = 4;

struct Sandbox {
    root: tempfile::TempDir,
    project: PathBuf,
    data: PathBuf,
    provider_config: PathBuf,
}

impl Sandbox {
    fn new(provider_url: &str) -> Result<Self> {
        Self::with_config(provider_url, "")
    }

    fn with_config(provider_url: &str, extra_config: &str) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project)?;
        memory::configuration(root.path())?;
        let provider_config = root.path().join("fixture-responses.toml");
        std::fs::write(
            &provider_config,
            format!(
                "api_base = {provider_url:?}\napi_key_env = 'KURU_SHELL_TURN_FIXTURE_KEY'\n{extra_config}"
            ),
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

struct Gate {
    started: AtomicUsize,
    released: AtomicUsize,
    release: Notify,
}

impl Gate {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            release: Notify::new(),
        }
    }

    async fn wait(&self) {
        while self.released.load(Ordering::Acquire) == 0 {
            let notified = self.release.notified();
            if self.released.load(Ordering::Acquire) != 0 {
                return;
            }
            notified.await;
        }
    }

    fn release(&self) {
        self.released.store(1, Ordering::Release);
        self.release.notify_waiters();
    }
}

struct ProviderState {
    requests: Mutex<Vec<Value>>,
    completion_attempts: AtomicUsize,
    failing_shell: bool,
    rotation_tool_calls: usize,
    rotation_issued: AtomicUsize,
    gate: Option<Arc<Gate>>,
}

async fn complete(State(state): State<Arc<ProviderState>>, Json(request): Json<Value>) -> Response {
    let attempt = state.completion_attempts.fetch_add(1, Ordering::SeqCst);
    state.requests.lock().await.push(request.clone());
    if let Some(gate) = &state.gate
        && gate
            .started
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        gate.wait().await;
    }
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
    } else if state.rotation_tool_calls > 0
        && state
            .rotation_issued
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        json!(
            (0..state.rotation_tool_calls)
                .map(|index| json!({
                    "type":"function_call",
                    "call_id":format!("ring-rotation-{index}"),
                    "name":"fixture_unknown_tool",
                    "arguments":"{}",
                }))
                .collect::<Vec<_>>()
        )
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
        Self::start_with(false, 0, None).await
    }

    async fn start_with_shell_failure(failing_shell: bool) -> Result<Self> {
        Self::start_with(failing_shell, 0, None).await
    }

    async fn start_gated() -> Result<(Self, Arc<Gate>)> {
        let gate = Arc::new(Gate::new());
        Ok((Self::start_with(false, 0, Some(gate.clone())).await?, gate))
    }

    async fn start_with_rotation() -> Result<Self> {
        Self::start_with(false, ROTATION_TOOL_CALLS, None).await
    }

    async fn start_with(
        failing_shell: bool,
        rotation_tool_calls: usize,
        gate: Option<Arc<Gate>>,
    ) -> Result<Self> {
        let state = Arc::new(ProviderState {
            requests: Mutex::new(vec![]),
            completion_attempts: AtomicUsize::new(0),
            failing_shell,
            rotation_tool_calls,
            rotation_issued: AtomicUsize::new(0),
            gate,
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

async fn run_sandbox(sandbox: Sandbox, debug: bool) -> Result<CliRun> {
    tokio::task::spawn_blocking(move || {
        // If this test future is cancelled, Tokio leaves the started blocking
        // worker running. It retains the complete sandbox through bounded
        // helper completion, including any reported cleanup failure.
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

async fn run_cli(provider_url: String, debug: bool) -> Result<CliRun> {
    run_sandbox(Sandbox::new(&provider_url)?, debug).await
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

fn reported_debug_ring(output: &std::process::Output) -> Result<PathBuf> {
    let stderr = std::str::from_utf8(&output.stderr).context("debug stderr is not UTF-8")?;
    let rings = stderr
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| value["debug_ring"].as_str().map(PathBuf::from))
        .collect::<Vec<_>>();
    ensure!(
        rings.len() == 1,
        "debug stderr must report exactly one ring directory: {stderr:?}"
    );
    Ok(rings.into_iter().next().expect("checked one ring"))
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

fn assert_expected_startup_notice(stderr: &[u8]) -> Result<()> {
    let stderr = std::str::from_utf8(stderr).context("startup stderr is not UTF-8")?;
    let mut lines = stderr.lines();
    let expected = "Memory: waiting for project ownership…";
    ensure!(
        lines.next() == Some(expected),
        "normal kuru run changed startup frame {expected:?}: {stderr:?}"
    );
    let runtime = lines
        .next()
        .context("normal kuru run omitted runtime-version startup frame")?;
    if runtime == "Memory: waiting for verified runtime cache…" {
        ensure!(
            lines.next() == Some("Memory: extracting embedded runtime…"),
            "cold kuru run changed extraction startup frame: {stderr:?}"
        );
    } else {
        ensure!(
            runtime == "Memory: verifying cached runtime…",
            "warm kuru run changed verification startup frame: {stderr:?}"
        );
    }
    let next = lines
        .next()
        .context("normal kuru run omitted runtime-version startup frame")?;
    ensure!(
        next == "Memory: checking runtime version…",
        "normal kuru run changed runtime-version startup frame: {stderr:?}"
    );
    for expected in [
        "Memory: preparing database…",
        "Memory: opening database…",
        "Memory: ready.",
    ] {
        ensure!(
            lines.next() == Some(expected),
            "normal kuru run changed startup frame {expected:?}: {stderr:?}"
        );
    }
    let notice = lines
        .next()
        .context("normal kuru run omitted first-run notice")?;
    ensure!(
        notice.starts_with("Memory is ready at ")
            && notice.contains("Memory and chat do not expire automatically.")
            && notice.contains("`kuru memory notes ID`")
            && notice.contains("`kuru memory export --format json --output PATH`")
            && notice.contains("`kuru memory forget ID --note SEQUENCE`")
            && notice.contains("`kuru memory purge --help`"),
        "normal kuru run changed first-run notice: {notice:?}"
    );
    ensure!(
        lines.next().is_none(),
        "normal kuru run emitted unexpected stderr after startup and notice: {stderr:?}"
    );
    Ok(())
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
        let hash = kuru_runtime::project_scope(&run.sandbox.project)?
            .strip_prefix("project/")
            .context("fixture project scope lacks prefix")?
            .to_owned();
        let expected_ring = run
            .sandbox
            .data
            .join("diagnostics")
            .join(hash)
            .canonicalize()?;
        ensure!(
            reported_debug_ring(output)?.canonicalize()? == expected_ring,
            "debug stderr reported a different project ring"
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
            normal.output.status.success(),
            "normal kuru run changed diagnostics presentation: stdout {:?}; stderr {:?}",
            String::from_utf8_lossy(&normal.output.stdout),
            String::from_utf8_lossy(&normal.output.stderr),
        );
        assert_expected_startup_notice(&normal.output.stderr)?;
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
        assert_expected_startup_notice(&run.output.stderr)?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_cli_rotates_the_fixed_private_diagnostic_ring() -> Result<()> {
    let server = Server::start_with_rotation()
        .await
        .context("start rotating fake Responses server")?;
    let result = async {
        let sandbox = Sandbox::with_config(
            &server.url,
            &format!("max_tool_calls = {ROTATION_TOOL_CALLS}\n"),
        )
        .context("prepare bounded tool-call CLI sandbox")?;
        let run = run_sandbox(sandbox, true)
            .await
            .context("run bounded rotation CLI")?;
        ensure!(
            run.output.status.success(),
            "rotation fixture kuru run failed with {}; stdout {:?}; stderr {:?}",
            run.output.status,
            String::from_utf8_lossy(&run.output.stdout),
            String::from_utf8_lossy(&run.output.stderr),
        );
        let turn: Value = serde_json::from_slice(&run.output.stdout)
            .context("parse rotation fixture kuru run JSON")?;
        ensure!(
            turn["text"].as_str().is_some_and(|text| !text.is_empty())
                && !turn["limited"].as_bool().unwrap_or(true),
            "rotation fixture did not finish an ordinary unbounded turn: {turn}"
        );
        ensure!(
            server.state.rotation_issued.load(Ordering::Acquire) == 1,
            "the fake provider did not issue the bounded rotation tool batch"
        );

        let root = run.sandbox.data.join("diagnostics");
        let scopes = std::fs::read_dir(&root)
            .context("read private diagnostics root after rotation")?
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        ensure!(
            scopes.len() == 1,
            "diagnostics created {} per-project directories instead of one: {scopes:?}",
            scopes.len()
        );
        ensure!(
            reported_debug_ring(&run.output)?.canonicalize()? == scopes[0].canonicalize()?,
            "rotation run reported a different diagnostics ring"
        );
        for index in 0..DIAGNOSTIC_FILE_COUNT {
            let trace = scopes[0].join(format!("trace-{index}.jsonl"));
            let metadata = std::fs::metadata(&trace)
                .with_context(|| format!("read rotated diagnostic slot {}", trace.display()))?;
            ensure!(
                metadata.len() > 0 && metadata.len() <= DIAGNOSTIC_FILE_BYTES,
                "rotated diagnostic slot {} has {} bytes, outside the fixed bound",
                trace.display(),
                metadata.len()
            );
        }
        let logs = diagnostics(&run)?;
        ensure!(logs.contains("\"status\":\"error\""));
        ensure!(
            !logs.contains(PROVIDER_SECRET),
            "provider sentinel leaked to rotated diagnostics: {logs}"
        );
        Ok(())
    }
    .await;
    let shutdown = server.shutdown().await;
    result?;
    shutdown
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_setup_refusal_is_bounded_before_provider_work() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let sandbox = Sandbox::new("http://127.0.0.1:9/v1")?;
    std::fs::create_dir(&sandbox.data)?;
    std::fs::set_permissions(&sandbox.data, std::fs::Permissions::from_mode(0o700))?;
    let replacement = sandbox.root.path().join("outside-diagnostics");
    std::fs::create_dir(&replacement)?;
    symlink(&replacement, sandbox.data.join("diagnostics"))?;

    let run = run_sandbox(sandbox, true).await?;
    ensure!(
        !run.output.status.success(),
        "unsafe diagnostics path unexpectedly opened"
    );
    ensure!(
        run.output.stdout.is_empty(),
        "setup failure wrote machine stdout"
    );
    let stderr = String::from_utf8_lossy(&run.output.stderr);
    ensure!(
        stderr.contains("cannot create the private project diagnostics directory"),
        "unexpected setup refusal: {stderr}"
    );
    ensure!(!stderr.contains(PROVIDER_SECRET));
    Ok(())
}

async fn trace_file(data: &std::path::Path) -> Result<PathBuf> {
    let deadline = Instant::now() + CLEANUP_TIMEOUT;
    loop {
        if let Ok(scopes) = std::fs::read_dir(data.join("diagnostics")) {
            for scope in scopes.flatten() {
                let trace = scope.path().join("trace-0.jsonl");
                if trace.exists() {
                    return Ok(trace);
                }
            }
        }
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for diagnostic trace file"
        );
        sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_provider_gate(gate: &Gate, worker: &JoinHandle<Result<CliRun>>) -> Result<()> {
    let deadline = Instant::now() + CLI_TIMEOUT;
    while gate.started.load(Ordering::Acquire) == 0 {
        ensure!(
            !worker.is_finished(),
            "CLI worker finished before reaching the fake provider gate"
        );
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for fake provider gate"
        );
        sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diagnostic_write_failure_keeps_a_completed_cli_turn_authoritative() -> Result<()> {
    let (server, gate) = Server::start_gated().await?;
    let sandbox = Sandbox::new(&server.url)?;
    let data = sandbox.data.clone();
    let mut worker = tokio::spawn(run_sandbox(sandbox, true));
    let observation = async {
        wait_for_provider_gate(&gate, &worker).await?;
        let trace = trace_file(&data).await?;
        let replacement = data.join("trace-replacement");
        std::fs::write(&replacement, b"replacement")?;
        std::fs::remove_file(&trace)?;
        std::fs::hard_link(&replacement, &trace)?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    gate.release();
    let run = timeout(CLI_TIMEOUT, &mut worker)
        .await
        .context("bounded CLI worker did not finish after diagnostic replacement")?
        .context("retained CLI worker panicked")??;
    let shutdown = server.shutdown().await;
    if let Err(observation) = observation {
        return Err(observation.context(format!(
            "fake provider readiness observation failed after CLI exit {}: stdout {:?}; stderr {:?}",
            run.output.status,
            String::from_utf8_lossy(&run.output.stdout),
            String::from_utf8_lossy(&run.output.stderr),
        )));
    }
    shutdown?;
    ensure!(
        run.output.status.success(),
        "diagnostic write failure displaced CLI success: stdout {:?}; stderr {:?}",
        String::from_utf8_lossy(&run.output.stdout),
        String::from_utf8_lossy(&run.output.stderr),
    );
    let turn: Value = serde_json::from_slice(&run.output.stdout)?;
    ensure!(turn["text"] == FINAL_TEXT);
    let stderr = String::from_utf8_lossy(&run.output.stderr);
    ensure!(stderr.contains("diagnostic cleanup failed; diagnostics may be incomplete"));
    ensure!(!stderr.contains(PROVIDER_SECRET));
    Ok(())
}
