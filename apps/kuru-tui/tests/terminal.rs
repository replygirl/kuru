#![cfg(unix)]

use kuru_memory::MemoryStore;

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::header::CONTENT_TYPE,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures::stream;
use kuru_core::{
    Config, HookCommand, LifecycleHooks, McpConfig, McpOAuthConfig, Mode, ModeProfile,
    PermissionAction, PermissionRule, PermissionSelector, SelectionOverrides,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{
    Connection, MySqlConnection,
    mysql::{MySqlConnectOptions, MySqlSslMode},
};
use tokio::sync::watch;

#[path = "support/mcp_oauth_https.rs"]
mod mcp_oauth_https;
#[path = "support/memory.rs"]
mod memory;
#[path = "support/terminal.rs"]
mod terminal;
use kuru_memory::test_budgets::OPERATION_TIMEOUT;
use mcp_oauth_https::HttpsMcpFixture;
use terminal::{FRAME_ALLOWANCE, IO_TIMEOUT, READY_TIMEOUT, Terminal, startup_timeout};

/// Bound for a PTY child to exit after `/quit` (or after a fixture's own last
/// step). `/quit` runs the runtime's `shutdown(true)` (src/ui.rs; kuru-runtime
/// `Harness::shutdown` in src/engine.rs), on the managed Remote backend
/// (src/cli.rs `open_memory`). Its sequential waits:
/// - `reconcile()` before and after the actors stop, two calls, each one
///   Remote `ViewOperation::Reconcile` (kuru-memory `MemoryStore::reconcile`,
///   src/facade.rs) under the reply deadline `OPERATION_TIMEOUT`. With no
///   candidate or uncertain write pending, the recovery steps around it
///   (`reopen_after_checked_recovery`, `recover_candidate_begin` and
///   `recover_candidate_unit`) return without a call;
/// - no exit dream: `Sandbox::command` passes `--no-dream`, which sets
///   `dream_on_exit = false` (kuru-core `apply_overrides`);
/// - `tools.shutdown()`, whose MCP join is bounded by `IO_TIMEOUT`
///   (kuru-connectors src/mcp.rs:1203) and dominates the concurrent shell
///   cleanup (5 s, src/unix_shell.rs `CLEANUP_ALLOWANCE`) and hook quiesce
///   (10 s, src/hooks.rs `QUIESCE`); four tests below configure MCP.
///
/// The CLI then calls `memory.close()`, which on Remote only drains the
/// client's attachments (`RemoteSession::close`, kuru-memory src/facade.rs)
/// and does not wait for the owner's retirement, so the owner's
/// `close_budget()` is not on this path. The `/quit` keystroke and the
/// closing frame add one `FRAME_ALLOWANCE`.
const EXIT_TIMEOUT: Duration = OPERATION_TIMEOUT
    .saturating_mul(2)
    .saturating_add(IO_TIMEOUT)
    .saturating_add(FRAME_ALLOWANCE);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_tty_argument_never_reads_stdin_at_120_and_80() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        for width in [120, 80] {
            let mut command = sandbox.command("demo");
            command.args(["run", "TTY_ARGUMENT_ONLY", "--json"]);
            let mut terminal = Terminal::spawn(command, 30, width)?;
            // The terminal stays open and receives no input or EOF.
            terminal.wait_exit(sandbox.startup_timeout + EXIT_TIMEOUT)?;
            ensure!(String::from_utf8_lossy(&terminal.output).contains("TTY_ARGUMENT_ONLY"));
            terminal.assert_restored()?;
        }
        Ok(())
    })
    .await
}

/// Prove cancellation against actual OS pipes and the default exit-dream
/// provider boundary, then recover only the exact completed journal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_held_output_broken_pipe_and_exit_dream_preserve_completed_retry() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use kuru_platform::unix::{OwnedProcessGroup, Reap, RootState, StdioPlan, StdioSlot};
        use nix::{sys::signal::{Signal, kill}, unistd::Pid};
        use sha2::{Digest, Sha256};

        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(AtomicUsize::new(0));
        let dream_started = Arc::new(tokio::sync::Notify::new());
        let models_started = Arc::new(tokio::sync::Notify::new());
        let hold_models = Arc::new(AtomicBool::new(false));
        let (release, released) = watch::channel(false);
        let answer = format!("PUBLIC_COMPLETED_{}", "x".repeat(128 * 1024));
        let app = Router::new()
            .route("/v1/models", get({
                let models_started = models_started.clone();
                let hold_models = hold_models.clone();
                let released = released.clone();
                move || {
                    let models_started = models_started.clone();
                    let hold_models = hold_models.clone();
                    let mut released = released.clone();
                    async move {
                        if hold_models.load(Ordering::SeqCst) {
                            models_started.notify_one();
                            let _ = released.wait_for(|ready| *ready).await;
                        }
                        Json(json!({"data":[{"id":"fixture"}]}))
                    }
                }
            }))
            .route("/v1/responses", post({
                let requests = requests.clone();
                let dream_started = dream_started.clone();
                let answer = answer.clone();
                move |Json(request): Json<Value>| {
                    let requests = requests.clone();
                    let dream_started = dream_started.clone();
                    let answer = answer.clone();
                    let mut released = released.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        let instructions = request["instructions"].as_str().unwrap_or("");
                        if instructions.contains("Phase: dream") {
                            // Reaching this request requires an already completed
                            // turn and candidate admission, not a timer guess.
                            dream_started.notify_one();
                            let _ = released.wait_for(|ready| *ready).await;
                        }
                        let text = if instructions.contains("Phase: speak and act") {
                            &answer
                        } else { "PRIVATE_HEADLESS_PEER_SENTINEL" };
                        ([(CONTENT_TYPE, "text/event-stream")], format!("data: {}\n\n", json!({
                            "type":"response.completed", "response":{
                                "id":"headless-owned-output", "status":"completed",
                                "output":[{"type":"message","content":[{"type":"output_text","text":text}]}],
                                "usage":{"input_tokens":8,"output_tokens":8}
                            }
                        }))).into_response()
                    }
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("headless-owned-output.toml");
        std::fs::write(&config, format!(
            "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\ndream_every=0\n",
            listener.local_addr()?
        ))?;
        let _server = Server(tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }));
        for case in ["held-output", "broken-output", "exit-dream", "setup-models"] {
            release.send_replace(false);
            hold_models.store(case == "setup-models", Ordering::SeqCst);
            let requests_before_case = requests.load(Ordering::SeqCst);
            let mut command = sandbox.command("responses");
            if case == "exit-dream" {
                // Restore the product default; Sandbox normally disables it.
                let args = command.get_args().filter(|arg| *arg != "--no-dream").map(std::ffi::OsStr::to_os_string).collect::<Vec<_>>();
                let mut with_dream = Command::new(command.get_program());
                with_dream.args(args);
                for (key, value) in command.get_envs() {
                    if let Some(value) = value { with_dream.env(key,value); } else { with_dream.env_remove(key); }
                }
                command = with_dream;
            }
            if case == "held-output" {
                command.arg("--debug");
            }
            command.args(["--model","fixture","--config"]).arg(&config).env("KURU_FIXTURE_KEY","fixture")
                .args(["run",case,"--turn-id",case,"--json"]);
            let pid_file = sandbox.root.path().join(format!("{case}.pid"));
            let mut launch = Command::new("/bin/sh");
            launch.args(["-c","printf '%s' \"$$\" > \"$1\"; shift; exec \"$@\"","headless-owned-output"])
                .arg(&pid_file).arg(command.get_program()).args(command.get_args());
            for (key,value) in command.get_envs() {
                if let Some(value) = value { launch.env(key,value); } else { launch.env_remove(key); }
            }
            let mut child = OwnedProcessGroup::spawn(launch,StdioPlan::new(StdioSlot::Null,StdioSlot::Pipe,StdioSlot::Pipe))?;
            let mut stdout: Option<Box<dyn Read + Send>> = Some(Box::new(child.take_stdout()?));
            let mut stderr = child.take_stderr()?;
            let (stderr_sender, stderr_receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = stderr.by_ref().take(1024 * 1024 + 1).read_to_end(&mut bytes).map(|_|bytes);
                let _ = stderr_sender.send(result);
            });
            let mut prefix = Vec::new();
            let mut session = None;
            let outcome = async {
                if case == "setup-models" {
                    tokio::time::timeout(sandbox.startup_timeout + READY_TIMEOUT,models_started.notified()).await
                        .context("setup did not enter the held model listing")?;
                    let pid: i32 = std::fs::read_to_string(&pid_file)?.parse()?;
                    ensure!(pid > 1 && matches!(child.root_state(),RootState::Running));
                    kill(Pid::from_raw(pid),Signal::SIGINT)?;
                    // Only then finish setup: the input-registered listener
                    // must retain this queued signal before user admission.
                    release.send_replace(true);
                } else {
                if case == "exit-dream" {
                    tokio::time::timeout(sandbox.startup_timeout + READY_TIMEOUT,dream_started.notified()).await
                        .context("default exit dream never reached the held provider")?;
                } else {
                    let mut pipe = stdout.take().expect("stdout");
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    std::thread::spawn(move || {
                        let mut first = [0];
                        let result = pipe.read_exact(&mut first).map(|_| (first,pipe));
                        let _ = sender.send(result);
                    });
                    let (first,pipe) = tokio::time::timeout(sandbox.startup_timeout + EXIT_TIMEOUT,receiver).await
                        .context("final stdout never began")???;
                    prefix.extend(first);
                    if case == "held-output" { stdout = Some(pipe); } else { drop(pipe); }
                }
                // Inspect the exact durable result before signalling. The
                // large final write already began in output cases, so the
                // held reader actually competes with final delivery.
                let mut options = memory_options(&sandbox)?;
                options.read_only = true;
                let inspector = MemoryStore::open_managed_observed(options,sandbox.project.canonicalize()?,PathBuf::from(env!("CARGO_BIN_EXE_kuru"))).1.await?;
                let inspect_result = async {
                    let id = kuru_runtime::Harness::continuation_session(&inspector,&sandbox.project).await?;
                    let mut digest = Sha256::new(); digest.update(b"kuru.turn-journal.v1\0"); digest.update(case.as_bytes());
                    let suffix = digest.finalize().iter().map(|byte|format!("{byte:02x}")).collect::<String>();
                    let key = format!("{}/session/{id}/turn/{suffix}",kuru_runtime::project_scope(&sandbox.project)?);
                    let journal = inspector.get(&key).await?.context("completed journal missing")?;
                    ensure!(journal["id"] == case && journal["prompt"] == case && journal["output"]["text"] == answer,
                        "wrong completion proof for {case}");
                    Ok::<_,anyhow::Error>(id)
                }.await;
                let close = inspector.close().await;
                close?;
                session = Some(inspect_result?);
                if case != "broken-output" {
                    let pid: i32 = std::fs::read_to_string(&pid_file)?.parse()?;
                    ensure!(pid > 1 && matches!(child.root_state(),RootState::Running),"owned root not running before SIGINT");
                    // No await or reap between owned observation and signal.
                    kill(Pid::from_raw(pid),Signal::SIGINT)?;
                }
                }
                let deadline = Instant::now() + EXIT_TIMEOUT;
                loop {
                    match child.root_state() {
                        RootState::Exited => break,
                        RootState::Running | RootState::Interrupted => {},
                        state => anyhow::bail!("owned output root state: {state:?}"),
                    }
                    ensure!(Instant::now() < deadline,"{case} did not settle after output interruption");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                if case == "setup-models" {
                    let mut options = memory_options(&sandbox)?;
                    options.read_only = true;
                    let inspector = MemoryStore::open_managed_observed(options,sandbox.project.canonicalize()?,PathBuf::from(env!("CARGO_BIN_EXE_kuru"))).1.await?;
                    let inspected = async {
                        let id = kuru_runtime::Harness::continuation_session(&inspector,&sandbox.project).await?;
                        let scope = kuru_runtime::project_scope(&sandbox.project)?;
                        let mut digest = Sha256::new(); digest.update(b"kuru.turn-journal.v1\0"); digest.update(case.as_bytes());
                        let suffix = digest.finalize().iter().map(|byte|format!("{byte:02x}")).collect::<String>();
                        ensure!(inspector.get(&format!("{scope}/session/{id}/turn/{suffix}")).await?.is_none(),"setup SIGINT admitted a user turn");
                        let saved = inspector.get(&format!("{scope}/session/{id}")).await?.context("setup session absent")?;
                        ensure!(saved["turns"] == 0 && requests.load(Ordering::SeqCst) == requests_before_case,"setup SIGINT started inference");
                        Ok::<(),anyhow::Error>(())
                    }.await;
                    let closed = inspector.close().await;
                    closed?; inspected?;
                }
                Ok::<(),anyhow::Error>(())
            }.await;
            child.terminate_before_reap();
            child.wait_pre_reap(Duration::from_millis(10),Instant::now()+EXIT_TIMEOUT).await;
            let status = child.reap_if_exited();
            release.send_replace(true);
            let stderr = stderr_receiver.recv_timeout(IO_TIMEOUT).context("owned-output stderr did not drain")??;
            if let Some(mut pipe) = stdout {
                pipe.by_ref().take(1024 * 1024 + 1).read_to_end(&mut prefix)?;
            }
            outcome?;
            let Reap::Reaped(status) = status else { anyhow::bail!("output root was not reaped: {status:?}") };
            ensure!(status.code() == Some(if case == "broken-output" {141} else {130}),"{case}: {status:?}: {}",String::from_utf8_lossy(&stderr));
            ensure!(!String::from_utf8_lossy(&prefix).contains("PRIVATE_HEADLESS_PEER_SENTINEL"));
            ensure!(!String::from_utf8_lossy(&stderr).contains("PRIVATE_HEADLESS_PEER_SENTINEL"));
            if case == "held-output" {
                ensure!(prefix.first() == Some(&b'{'), "debug output displaced the final JSON");
                ensure!(String::from_utf8_lossy(&stderr).contains("\"debug_ring\":"), "debug ring notice absent from stderr");
                ensure!(sandbox.data.join("diagnostics").is_dir(), "debug ring absent");
            }
            if case == "setup-models" {
                ensure!(prefix.is_empty(),"unadmitted setup cancellation printed an answer");
                continue;
            }
            let before_retry = requests.load(Ordering::SeqCst);
            let mut retry = sandbox.command("responses");
            retry.args(["--model","fixture","--config"]).arg(&config).env("KURU_FIXTURE_KEY","fixture")
                .args(["--resume",session.as_deref().context("session absent")?,"run",case,"--turn-id",case,"--json"]);
            let output = headless_piped_output(retry,Vec::new(),sandbox.startup_timeout+EXIT_TIMEOUT).await?;
            ensure!(output.status.success(),"retry {case}: {}",String::from_utf8_lossy(&output.stderr));
            let result: Value = serde_json::from_slice(&output.stdout)?;
            ensure!(result["text"] == answer && requests.load(Ordering::SeqCst) == before_retry,"retry replayed {case}");
        }
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_http_stream_keeps_tool_effects_private_and_denial_nonfatal() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let mode = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app = Router::new()
            .route("/v1/models",get(||async{Json(json!({"data":[{"id":"fixture"}]}))}))
            .route("/v1/responses",post({
                let mode = mode.clone(); let requests = requests.clone();
                move |Json(request):Json<Value>| {
                    let mode = mode.clone(); let requests = requests.clone();
                    async move {
                        requests.lock().unwrap().push(request.clone());
                        let speaking = request["instructions"].as_str().is_some_and(|value|value.contains("Phase: speak and act"));
                        let followup = request["input"].as_array().is_some_and(|input|input.iter().any(|item|item["type"]=="function_call_output"));
                        if speaking && followup && mode.load(Ordering::SeqCst)==2 {
                            return (axum::http::StatusCode::SERVICE_UNAVAILABLE,"PRIVATE_PROVIDER_ERROR_SENTINEL").into_response();
                        }
                        let output = if speaking && !followup {
                            json!([{"type":"function_call","call_id":"headless-effect","name":"file_write",
                                "arguments":json!({"path":"effect.txt","content":"PRIVATE_TOOL_BODY_SENTINEL"}).to_string()}])
                        } else if speaking && followup {
                            json!([
                                {"id":"reasoning","type":"reasoning","summary":[{"type":"summary_text","text":"PRIVATE_REASONING_SENTINEL"}],"encrypted_content":"PRIVATE_COGNITIVE_SENTINEL"},
                                {"id":"message","type":"message","content":[{"type":"output_text","text":"PUBLIC_HTTP_ANSWER"}]}
                            ])
                        } else {
                            json!([{"type":"message","content":[{"type":"output_text","text":
                                if speaking {"PUBLIC_HTTP_ANSWER"} else {"PRIVATE_COGNITIVE_SENTINEL"}}]}])
                        };
                        let completed = format!("data: {}\n\n",json!({"type":"response.completed","response":{
                            "id":"headless-http","status":"completed","output":output,
                            "usage":{"input_tokens":8,"output_tokens":8}}}));
                        if speaking && followup {
                            let initial = format!("data: {}\n\ndata: {}\n\n",
                                json!({"type":"response.reasoning_summary_text.delta","item_id":"reasoning","output_index":0,"summary_index":0,"delta":"PRIVATE_REASONING_SENTINEL"}),
                                json!({"type":"response.output_text.delta","item_id":"message","output_index":1,"content_index":0,"delta":"PUBLIC_HTTP_ANSWER"}));
                            return ([(CONTENT_TYPE,"text/event-stream")],Body::from_stream(stream::iter([
                                Ok::<_,io::Error>(initial),Ok(completed)
                            ]))).into_response();
                        }
                        ([(CONTENT_TYPE,"text/event-stream")],completed).into_response()
                    }
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let _server = Server(tokio::spawn(async move{axum::serve(listener,app).await.unwrap()}));
        for case in 0..3 {
            mode.store(case,Ordering::SeqCst);
            let sandbox = Sandbox::warmed().await?;
            let config = sandbox.root.path().join("headless-http.toml");
            std::fs::write(&config,format!("api_base='http://{address}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n"))?;
            let mut command = sandbox.command("responses");
            command.args(["--model","fixture","--config"]).arg(&config).env("KURU_FIXTURE_KEY","fixture");
            if case!=1 {command.arg("--allow-write");}
            command.args(["run","EXACT_ARGV_HTTP","--turn-id","headless-http","--output-format","stream-json"]);
            let before = requests.lock().unwrap().len();
            let output = headless_piped_output(command,b"PIPE_LITERAL_HTTP".to_vec(),sandbox.startup_timeout+EXIT_TIMEOUT).await?;
            ensure!(output.status.code()==Some(if case==2 {1}else{0}),"HTTP case{case}: {}",String::from_utf8_lossy(&output.stderr));
            let text = std::str::from_utf8(&output.stdout)?;
            for private in ["PRIVATE_TOOL_BODY_SENTINEL","PRIVATE_COGNITIVE_SENTINEL","PRIVATE_REASONING_SENTINEL","PRIVATE_PROVIDER_ERROR_SENTINEL"] {
                ensure!(!text.contains(private),"public stream disclosed {private}");
            }
            let records = text.lines().map(serde_json::from_str::<Value>).collect::<std::result::Result<Vec<_>,_>>()?;
            ensure!(records.iter().all(|record|record["version"]==1 && ["started","status","snapshot","gap","terminal"].contains(&record["kind"].as_str().unwrap_or(""))));
            ensure!(records.windows(2).all(|pair|pair[0]["seq"].as_u64()<pair[1]["seq"].as_u64()));
            ensure!(records.iter().filter(|record|record["kind"]=="terminal").count()==1);
            let terminal = records.last().context("HTTP terminal absent")?;
            ensure!(terminal["detail"]["status"]==if case==2 {"failed"}else{"completed"});
            if case!=2 {ensure!(terminal["detail"]["answer"]=="PUBLIC_HTTP_ANSWER");}
            if case==1 {ensure!(!sandbox.project.join("effect.txt").exists(),"denied write took effect");}
            else {ensure!(std::fs::read(sandbox.project.join("effect.txt"))?==b"PRIVATE_TOOL_BODY_SENTINEL");}
            {
                let captured = requests.lock().unwrap();
                ensure!(captured[before..].iter().any(|request| request.to_string().contains("EXACT_ARGV_HTTP\\n\\nPiped input (literal data):\\nPIPE_LITERAL_HTTP")),"HTTP omitted exact composed prompt");
            }
            if case==0 {
                let before_retry = requests.lock().unwrap().len();
                let mut retry = sandbox.command("responses");
                retry.args(["--model","fixture","--config"]).arg(&config).env("KURU_FIXTURE_KEY","fixture")
                    .args(["--allow-write","--resume",terminal["detail"]["session"].as_str().context("HTTP session absent")?,
                        "run","EXACT_ARGV_HTTP","--turn-id","headless-http","--json"]);
                let retried = headless_piped_output(retry,b"PIPE_LITERAL_HTTP".to_vec(),sandbox.startup_timeout+EXIT_TIMEOUT).await?;
                ensure!(retried.status.success(),"HTTP retry: {}",String::from_utf8_lossy(&retried.stderr));
                let final_json:Value=serde_json::from_slice(&retried.stdout)?;
                ensure!(final_json["text"]=="PUBLIC_HTTP_ANSWER" && final_json.get("version").is_none() && final_json["events"].is_array());
                ensure!(requests.lock().unwrap().len()==before_retry && std::fs::read(sandbox.project.join("effect.txt"))?==b"PRIVATE_TOOL_BODY_SENTINEL");
            }
        }
        let untrusted = Sandbox::warmed().await?;
        std::fs::create_dir(untrusted.project.join(".kuru"))?;
        std::fs::write(untrusted.project.join(".kuru/config.toml"),"allow_shell=true\n")?;
        let mut command = untrusted.command("demo");
        command.args(["run","workspace denied","--json"]);
        let output = headless_piped_output(command,Vec::new(),untrusted.startup_timeout+EXIT_TIMEOUT).await?;
        ensure!(output.status.code()==Some(3) && output.stdout.is_empty() && !untrusted.data.exists(),"trust refusal: {}",String::from_utf8_lossy(&output.stderr));
        Ok(())
    }).await
}

#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_held_stdin_sigint_precedes_every_authority_boundary() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use kuru_platform::unix::{OwnedProcessGroup, Reap, RootState, StdioPlan, StdioSlot};
        use nix::{
            sys::signal::{Signal, kill},
            unistd::Pid,
        };
        let sandbox = Sandbox::warmed().await?;
        let invalid = sandbox.root.path().join("must-not-load.toml");
        std::fs::write(&invalid, "[")?;
        let pid_file = sandbox.root.path().join("held-stdin.pid");
        let mut command = sandbox.command("demo");
        command
            .arg("--config")
            .arg(&invalid)
            .args(["run", "--json"])
            .env("KURU_HEADLESS_STDIN_READY_FIXTURE", "1");
        let mut launch = Command::new("/bin/sh");
        launch
            .args([
                "-c",
                "printf '%s' \"$$\" > \"$1\"; shift; exec \"$@\"",
                "headless-stdin-fixture",
            ])
            .arg(&pid_file)
            .arg(command.get_program())
            .args(command.get_args());
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                launch.env(key, value);
            } else {
                launch.env_remove(key);
            }
        }
        let mut child = OwnedProcessGroup::spawn(
            launch,
            StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Pipe),
        )?;
        // This retained writer prevents EOF until after the signalled root
        // settles. No authority can have activated while stdin is incomplete.
        let stdin = child.take_stdin()?;
        let mut stdout = child.take_stdout()?;
        let mut stderr = child.take_stderr()?;
        let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
        let (stderr_sender, stderr_receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut ready_sender = Some(ready_sender);
            let mut bytes = Vec::new();
            let result = (|| -> io::Result<Vec<u8>> {
                let mut byte = [0];
                while stderr.read(&mut byte)? != 0 {
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"headless stdin signal ready\n")
                        && let Some(sender) = ready_sender.take()
                    {
                        let _ = sender.send(());
                    }
                    if bytes.len() > 1024 * 1024 {
                        return Err(io::Error::other("stdin stderr capture bound"));
                    }
                }
                Ok(bytes)
            })();
            let _ = stderr_sender.send(result);
        });
        let (stdout_sender, stdout_receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = stdout_sender.send(result);
        });
        let outcome = async {
            tokio::time::timeout(sandbox.startup_timeout + READY_TIMEOUT, ready_receiver)
                .await
                .context("held stdin never registered its signal listener")??;
            let pid: i32 = std::fs::read_to_string(&pid_file)?.parse()?;
            ensure!(pid > 1 && matches!(child.root_state(), RootState::Running));
            // The root remains owned/unreaped; no await separates this check
            // from its signal, and stdin is still held by this fixture.
            kill(Pid::from_raw(pid), Signal::SIGINT)?;
            let deadline = Instant::now() + EXIT_TIMEOUT;
            loop {
                match child.root_state() {
                    RootState::Exited => break,
                    RootState::Running | RootState::Interrupted => {}
                    state => anyhow::bail!("held stdin root state: {state:?}"),
                }
                ensure!(
                    Instant::now() < deadline,
                    "held stdin did not settle on SIGINT"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        child.terminate_before_reap();
        child
            .wait_pre_reap(Duration::from_millis(10), Instant::now() + EXIT_TIMEOUT)
            .await;
        let status = child.reap_if_exited();
        drop(stdin);
        let stdout = stdout_receiver
            .recv_timeout(IO_TIMEOUT)
            .context("held stdin stdout did not drain")??;
        let stderr = stderr_receiver
            .recv_timeout(IO_TIMEOUT)
            .context("held stdin stderr did not drain")??;
        outcome?;
        let Reap::Reaped(status) = status else {
            anyhow::bail!("held stdin root was not reaped: {status:?}")
        };
        ensure!(
            status.code() == Some(130) && stdout.is_empty() && !sandbox.data.exists(),
            "held stdin: {status:?}: {}",
            String::from_utf8_lossy(&stderr)
        );
        ensure!(
            !String::from_utf8_lossy(&stderr).contains("must-not-load.toml"),
            "configuration activated before stdin completed"
        );
        Ok(())
    })
    .await
}

/// Test-only one-shot pipe driver. Every launch is platform-owned; both output
/// pipes drain concurrently and the retained root is reaped on every outcome.
async fn headless_piped_output(
    mut command: Command,
    input: Vec<u8>,
    allowance: Duration,
) -> Result<std::process::Output> {
    use kuru_platform::unix::{OwnedProcessGroup, Reap, RootState, StdioPlan, StdioSlot};
    command.env_remove("KURU_REDUCED_MOTION");
    let mut child = OwnedProcessGroup::spawn(
        command,
        StdioPlan::new(StdioSlot::Pipe, StdioSlot::Pipe, StdioSlot::Pipe),
    )?;
    let mut stdin = child.take_stdin()?;
    let input_worker = std::thread::spawn(move || {
        let result = stdin.write_all(&input);
        drop(stdin);
        result
    });
    let capture = |mut pipe: Box<dyn Read + Send>| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        });
        receiver
    };
    let stdout = capture(Box::new(child.take_stdout()?));
    let stderr = capture(Box::new(child.take_stderr()?));
    let deadline = Instant::now() + allowance;
    let outcome = async {
        loop {
            match child.root_state() {
                RootState::Exited => return Ok::<(), anyhow::Error>(()),
                RootState::Running | RootState::Interrupted => {}
                state => anyhow::bail!("headless root state: {state:?}"),
            }
            ensure!(Instant::now() < deadline, "headless child did not settle");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    .await;
    child.terminate_before_reap();
    child
        .wait_pre_reap(Duration::from_millis(10), Instant::now() + EXIT_TIMEOUT)
        .await;
    let status = child.reap_if_exited();
    let input_result = input_worker
        .join()
        .map_err(|_| anyhow::anyhow!("stdin fixture worker panicked"))?;
    let stdout = stdout
        .recv_timeout(IO_TIMEOUT)
        .context("headless stdout did not drain")??;
    let stderr = stderr
        .recv_timeout(IO_TIMEOUT)
        .context("headless stderr did not drain")??;
    outcome?;
    if let Err(error) = input_result {
        ensure!(
            error.kind() == io::ErrorKind::BrokenPipe,
            "stdin fixture: {error}"
        );
    }
    ensure!(stdout.len() <= 1024 * 1024 && stderr.len() <= 1024 * 1024);
    let Reap::Reaped(status) = status else {
        anyhow::bail!("headless root was not reaped: {status:?}")
    };
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_piped_input_retry_and_pre_authority_rejection_are_exact() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let allowance = sandbox.startup_timeout + EXIT_TIMEOUT;
        let mut command = sandbox.command("demo");
        command.args([
            "run",
            "summarize",
            "--turn-id",
            "headless-exact",
            "--output-format",
            "stream-json",
        ]);
        let output = headless_piped_output(command, b"PIPE_FACT_ALPHA".to_vec(), allowance).await?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let records: Vec<Value> = std::str::from_utf8(&output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        ensure!(
            records
                .iter()
                .filter(|record| record["kind"] == "terminal")
                .count()
                == 1
        );
        ensure!(
            records
                .windows(2)
                .all(|pair| pair[0]["seq"].as_u64() < pair[1]["seq"].as_u64())
        );
        let terminal = records.last().context("terminal missing")?;
        ensure!(terminal["kind"] == "terminal" && terminal["detail"]["status"] == "completed");
        let session = terminal["detail"]["session"]
            .as_str()
            .context("session missing")?;
        for (input, expected, reused) in [
            (b"PIPE_FACT_ALPHA".as_slice(), 0, true),
            (b"PIPE_FACT_BETA".as_slice(), 2, false),
        ] {
            let mut command = sandbox.command("demo");
            command.args([
                "--resume",
                session,
                "run",
                "summarize",
                "--turn-id",
                "headless-exact",
                "--output-format",
                "stream-json",
            ]);
            let output = headless_piped_output(command, input.to_vec(), allowance).await?;
            ensure!(
                output.status.code() == Some(expected),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let records: Vec<Value> = std::str::from_utf8(&output.stdout)?
                .lines()
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?;
            let terminal = records.last().context("retry terminal missing")?;
            if reused {
                ensure!(terminal["detail"]["reused"] == true);
            } else {
                ensure!(terminal["detail"]["status"] == "rejected");
            }
        }
        for (argv, input, expected) in [
            (Some("ARGUMENT_BYTES"), Vec::new(), "ARGUMENT_BYTES"),
            (None, b"PIPE_ONLY_BYTES".to_vec(), "PIPE_ONLY_BYTES"),
        ] {
            let mut command = sandbox.command("demo");
            command.arg("run");
            if let Some(argv) = argv {
                command.arg(argv);
            }
            command.arg("--json");
            let output = headless_piped_output(command, input, allowance).await?;
            ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: Value = serde_json::from_slice(&output.stdout)?;
            ensure!(
                result["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(expected))
            );
            ensure!(result.get("version").is_none() && result["events"].is_array());
        }
        let rejected = Sandbox::warmed().await?;
        // Invalid configuration would fail if the input boundary were late.
        let invalid = rejected.root.path().join("invalid.toml");
        std::fs::write(&invalid, "[")?;
        for input in [Vec::new(), vec![0xff], vec![b'x'; 131_073]] {
            let mut command = rejected.command("demo");
            command
                .arg("--config")
                .arg(&invalid)
                .args(["run", "--output-format", "stream-json"]);
            let output = headless_piped_output(command, input, allowance).await?;
            ensure!(
                output.status.code() == Some(2),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            ensure!(output.stdout.is_empty() && !rejected.data.exists());
        }
        Ok(())
    })
    .await
}

#[test]
fn real_pty_file_checkpoint_inspect_and_selected_undo() -> Result<()> {
    let sandbox = Sandbox::new()?;
    let target = sandbox.project.join("checkpoint-note.txt");
    std::fs::write(&target, "before")?;
    let output = sandbox
        .command("demo")
        .args([
            "--allow-write",
            "tool",
            "file_write",
            "--args",
            r#"{"path":"checkpoint-note.txt","content":"after"}"#,
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "fixture file write failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout)?;
    let id = output
        .trim()
        .strip_prefix("file_write completed; checkpoint ")
        .context("fixture write omitted checkpoint ID")?;
    let mut command = sandbox.command("demo");
    command.arg("--allow-write");
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
    terminal.command(&format!("/file-inspect {id}"), None)?;
    terminal.wait_composer_frame(
        &["checkpoint-note.txt", "applied", "enter send"],
        READY_TIMEOUT,
    )?;
    terminal.command(&format!("/file-undo {id}"), None)?;
    terminal.wait_composer_frame(&["undo", "applied", "enter send"], READY_TIMEOUT)?;
    ensure!(
        std::fs::read(&target)? == b"before",
        "PTY undo did not restore the file"
    );
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_fresh_terminals_share_owner_and_keep_private_sessions_through_eof() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let first_held = Arc::new(AtomicBool::new(false));
        let second_held = Arc::new(AtomicBool::new(false));
        let first_release = Arc::new(tokio::sync::Notify::new());
        let second_release = Arc::new(tokio::sync::Notify::new());
        let (arrivals, mut arrived) = tokio::sync::mpsc::unbounded_channel();
        let app = Router::new()
            .route("/v1/models", get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }))
            .route("/v1/responses", post({
                let requests = requests.clone();
                let first_held = first_held.clone();
                let second_held = second_held.clone();
                let first_release = first_release.clone();
                let second_release = second_release.clone();
                move |Json(request): Json<Value>| {
                    let requests = requests.clone();
                    let first_held = first_held.clone();
                    let second_held = second_held.clone();
                    let first_release = first_release.clone();
                    let second_release = second_release.clone();
                    let arrivals = arrivals.clone();
                    async move {
                        let text = request.to_string();
                        requests.lock().unwrap().push(request);
                        if text.contains("FIRST_PRIVATE_SENTINEL") && !first_held.swap(true, Ordering::SeqCst) {
                            arrivals.send("first").unwrap();
                            first_release.notified().await;
                        } else if text.contains("SECOND_PRIVATE_SENTINEL") && !second_held.swap(true, Ordering::SeqCst) {
                            arrivals.send("second").unwrap();
                            second_release.notified().await;
                        }
                        ([(CONTENT_TYPE, "text/event-stream")], format!("data: {}\n\n", json!({
                            "type":"response.completed", "response": {
                                "id":"session-admission", "status":"completed",
                                "output":[{"type":"message","content":[{"type":"output_text","text":"COUNTED_SESSION_ANSWER"}]}],
                                "usage":{"input_tokens":3,"output_tokens":2}
                            }
                        }))).into_response()
                    }
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("concurrent-provider.toml");
        std::fs::write(&config, format!(
            "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\nmax_parallel=1\n",
            listener.local_addr()?
        ))?;
        let _server = Server(tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }));
        let command = || {
            let mut command = sandbox.command("responses");
            command.args(["--model", "fixture", "--config"]).arg(&config).env("KURU_FIXTURE_KEY", "fixture");
            command
        };
        let mut first = Terminal::spawn(command(), 40, 120)?;
        first.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        let mut second = Terminal::spawn(command(), 40, 80)?;
        second.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        let options = memory_options(&sandbox)?;
        let mut inspection = options.clone();
        inspection.read_only = true;
        let inspector = MemoryStore::open_managed_observed(
            inspection,
            sandbox.project.canonicalize()?,
            PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
        )
        .1
        .await?;
        let drivers = inspector.live_session_drivers().await?;
        ensure!(
            drivers.len() == 2,
            "two fresh processes did not retain distinct live claims: {drivers:?}"
        );
        ensure!(drivers[0].proof.session_id != drivers[1].proof.session_id);
        ensure!(
            drivers[0].proof.service_generation == drivers[1].proof.service_generation,
            "fresh processes elected different owners"
        );
        let ids = drivers
            .iter()
            .map(|driver| driver.proof.session_id.clone())
            .collect::<BTreeSet<_>>();

        first.submit("FIRST_PRIVATE_SENTINEL")?;
        second.submit("SECOND_PRIVATE_SENTINEL")?;
        let overlapping = tokio::time::timeout(READY_TIMEOUT, async {
            let mut seen = BTreeSet::new();
            while seen.len() != 2 {
                seen.insert(arrived.recv().await.context("provider arrival channel closed")?);
            }
            Ok::<_, anyhow::Error>(seen)
        }).await.context("two actual provider requests did not overlap")??;
        ensure!(overlapping == BTreeSet::from(["first", "second"]));
        ensure!(inspector.live_session_drivers().await?.len() == 2);
        first_release.notify_one();
        second_release.notify_one();
        first.wait_composer_frame(&["COUNTED_SESSION_ANSWER", "enter send"], READY_TIMEOUT)?;
        second.wait_composer_frame(&["COUNTED_SESSION_ANSWER", "enter send"], READY_TIMEOUT)?;
        {
            let captured = requests.lock().unwrap();
            ensure!(captured.len() >= 2);
            ensure!(captured.iter().all(|request| {
                let text = request.to_string();
                !(text.contains("FIRST_PRIVATE_SENTINEL") && text.contains("SECOND_PRIVATE_SENTINEL"))
            }), "actual provider requests concatenated unrelated private histories");
        }
        let scope = kuru_runtime::project_scope(&sandbox.project)?;
        let profile = ModeProfile::builtin(Mode::Ifs);
        let membership = inspector
            .get(&format!("{scope}/ifs/membership"))
            .await?
            .context("membership absent")?;
        let mut seen = BTreeSet::new();
        for id in &ids {
            let transcript = profile.memory.transcript_namespace(&scope, id);
            let history = inspector.history(&transcript, 64).await?;
            let owns_first = history
                .iter()
                .any(|message| message.plain_text() == Some("FIRST_PRIVATE_SENTINEL"));
            let owns_second = history
                .iter()
                .any(|message| message.plain_text() == Some("SECOND_PRIVATE_SENTINEL"));
            ensure!(
                owns_first != owns_second,
                "fresh session mixed or lost submitted transcripts"
            );
            let own = if owns_first {
                "FIRST_PRIVATE_SENTINEL"
            } else {
                "SECOND_PRIVATE_SENTINEL"
            };
            let foreign = if owns_first {
                "SECOND_PRIVATE_SENTINEL"
            } else {
                "FIRST_PRIVATE_SENTINEL"
            };
            seen.insert(own);
            let mut private_seen = false;
            for part in membership["parts"]
                .as_array()
                .context("membership parts absent")?
            {
                let actor = part["id"].as_str().context("part identity absent")?;
                let namespace = profile.memory.identity_namespace(&scope, Mode::Ifs, actor);
                let private = inspector.session_history_window(&namespace, id, 64).await?;
                ensure!(
                    private.messages.iter().all(|message| !message
                        .plain_text()
                        .is_some_and(|text| text.contains(foreign))),
                    "unrelated raw private history crossed sessions"
                );
                private_seen |= private
                    .messages
                    .iter()
                    .any(|message| message.plain_text().is_some_and(|text| text.contains(own)));
            }
            ensure!(
                private_seen,
                "own submitted context was not persisted for any participant"
            );
        }
        ensure!(seen.len() == 2);
        second.command("/sessions", Some("Sessions"))?;
        second.wait_text(&["live"], &[])?;
        second.close_picker(b"\x1b")?;

        first.send(b"/quit\r")?;
        first.wait_exit(EXIT_TIMEOUT)?;
        first.assert_restored()?;
        // Completed owner reads observe actual EOF handling; no elapsed grace
        // authorizes release of a claim or reuse of the native barrier.
        let remaining = tokio::time::timeout(READY_TIMEOUT, async {
            loop {
                let live = inspector.live_session_drivers().await?;
                if live.len() == 1 {
                    return Ok::<_, anyhow::Error>(live);
                }
            }
        })
        .await
        .context("normal close did not release its exact presence")??;
        ensure!(ids.contains(&remaining[0].proof.session_id));
        second.submit("SURVIVING_SESSION_STILL_WORKS")?;
        second.wait_composer_frame(
            &["SURVIVING_SESSION_STILL_WORKS", "COUNTED_SESSION_ANSWER", "enter send"],
            READY_TIMEOUT,
        )?;
        // An abrupt process exit exercises socket EOF rather than Close.
        second.close(EXIT_TIMEOUT)?;
        tokio::time::timeout(READY_TIMEOUT, async {
            loop {
                if inspector.live_session_drivers().await?.is_empty() {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await
        .context("abrupt exit left an orphan driver presence")??;
        inspector.close().await?;
        kuru_memory::test_support::await_managed_quiescence(&options).await?;
        Ok(())
    })
    .await
}

struct Sandbox {
    root: memory::ServiceCleanup,
    project: PathBuf,
    data: PathBuf,
    startup_timeout: Duration,
}

impl Sandbox {
    /// A sandbox for a plain `fn` test: warms the shared cache synchronously.
    fn new() -> Result<Self> {
        Self::with_cache(&kuru_memory::test_support::cache_dir()?)
    }

    /// A sandbox for a test inside a Tokio runtime.
    async fn warmed() -> Result<Self> {
        Self::with_cache(&kuru_memory::test_support::warmed_cache_dir().await?)
    }

    /// A sandbox with its own empty engine and store template cache, as on a
    /// machine where Kuru has never run, and that cache's path.
    fn fresh_cache() -> Result<(Self, PathBuf)> {
        // The cache, one level down, receives the store template.
        let root = kuru_memory::test_support::tempdir()?
            .with_depth_budget(1 + kuru_memory::test_support::TEMPLATE_DEPTH);
        let cache = root.path().join("fresh cache");
        Ok((Self::in_root(root, &cache)?, cache))
    }

    fn with_cache(cache: &std::path::Path) -> Result<Self> {
        Self::in_root(kuru_memory::test_support::tempdir()?, cache)
    }

    fn in_root(root: kuru_memory::test_support::TempDir, cache: &std::path::Path) -> Result<Self> {
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project)?;
        let configuration = memory::configuration_with(root.path(), cache)?;
        let config: Config = toml::from_str(&std::fs::read_to_string(
            configuration.join("kuru/config.toml"),
        )?)?;
        Ok(Self {
            root: memory::ServiceCleanup::new(root, &data),
            project,
            data,
            startup_timeout: startup_timeout(Duration::from_secs(
                config.memory.startup_timeout_secs,
            )),
        })
    }

    fn command(&self, provider: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        command
            .arg("-C")
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.data)
            .args(["--provider", provider, "--no-dream"])
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            // A failing test shows the stderr of every owner this elects.
            .env(
                kuru_memory::test_support::OWNER_DIAGNOSTIC_ENV,
                self.root.owner_diagnostic_path(),
            )
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env_remove("NO_COLOR")
            .env_remove("KURU_REDUCED_MOTION");
        command
    }

    fn config(&self) -> Result<Config> {
        let output = self.command("demo").arg("config").output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(toml::from_str(&String::from_utf8(output.stdout)?)?)
    }

    /// Inspect saved preferences through the product's read-only path. While
    /// a UI runs, it attaches to that UI's owner. After the UI exits, the
    /// owner retires and its store's Dolt endpoint stays published until the
    /// reap, so a direct open could borrow a Dolt that is stopping; the
    /// managed open instead waits on the owner lock, then opens its own.
    async fn preferences(&self) -> Result<kuru_core::ProjectPreferences> {
        let mut options = memory_options(self)?;
        options.read_only = true;
        let (_, opening) = MemoryStore::open_managed_observed(
            options,
            self.project.canonicalize()?,
            PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
        );
        let memory = opening.await?;
        let preferences = kuru_runtime::Harness::load_preferences(&memory, &self.project).await?;
        memory.close().await?;
        Ok(preferences)
    }

    async fn effective_config(&self) -> Result<Config> {
        let preferences = self.preferences().await?;
        Config::load_with_preferences(
            Some(&self.root.path().join("config/kuru/config.toml")),
            &self.project,
            None,
            &preferences,
            SelectionOverrides {
                provider: Some("demo"),
                ..SelectionOverrides::default()
            },
        )
    }

    fn sessions(&self) -> Result<Vec<kuru_runtime::Session>> {
        let output = self.command("demo").arg("sessions").output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

fn diagnostics(data: &std::path::Path) -> Result<String> {
    std::fs::read_dir(data.join("diagnostics"))
        .context("read PTY diagnostics root")?
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

// Re-executed by the terminal driver tests, with an explicit mode. Keeping this
// subprocess entry in the test binary avoids shipping a fixture executable.
#[test]
#[ignore = "subprocess entry exercised by terminal driver regressions"]
fn terminal_fixture_process() -> Result<()> {
    match std::env::var("KURU_TERMINAL_FIXTURE")?.as_str() {
        "pressure" => {
            std::thread::sleep(Duration::from_millis(700));
            let mut output = std::io::stdout().lock();
            output.write_all(b"PAYLOAD:")?;
            output.write_all(&vec![b'x'; 1_048_576])?;
            output.write_all(b":DONE")?;
            output.flush()?;
        }
        "stalled" => {
            println!("STALLED");
            std::io::stdout().flush()?;
            std::thread::sleep(Duration::from_secs(30));
        }
        "dimensions" => {
            let (cols, rows) = crossterm::terminal::size()?;
            println!("SIZE:{cols}x{rows}");
            std::io::stdout().flush()?;
        }
        "nested-dimensions" => nested_size_fixture("dimensions")?,
        // The nested child exits cleanly before it reports a size.
        "nested-early-exit" => nested_size_fixture("early-exit")?,
        "early-exit" => {
            println!("PARTIAL");
            std::io::stdout().flush()?;
        }
        // The second line waits for the parent's acknowledgment, which it sends
        // only after it has stopped reading, so LATE is still queued when the
        // parent observes the exit.
        "late-output" => {
            println!("EARLY");
            std::io::stdout().flush()?;
            let mut acknowledgment = String::new();
            std::io::stdin().read_line(&mut acknowledgment)?;
            println!("LATE");
            std::io::stdout().flush()?;
        }
        "fragmented-frame" => {
            crossterm::terminal::enable_raw_mode()?;
            let mut input = std::io::stdin().lock();
            let mut output = std::io::stdout().lock();
            // Each fragment waits for the parent's explicit acknowledgment.
            // Text and cursor visibility arrive before the final cursor move,
            // just as the real backend's separate flushes permit.
            output.write_all(b"\x1b[2J\x1b[Hfocus draft\x1b[?25h")?;
            output.flush()?;
            let mut acknowledgment = [0];
            input.read_exact(&mut acknowledgment)?;
            output.write_all(b"\x1b[3;")?;
            output.flush()?;
            input.read_exact(&mut acknowledgment)?;
            output.write_all(b"14H")?;
            output.flush()?;
            input.read_exact(&mut acknowledgment)?;
            output.write_all(b"\x1b[1;1Hanimated draft\x1b[?25h\x1b[3;14H")?;
            output.flush()?;
            input.read_exact(&mut acknowledgment)?;
            crossterm::terminal::disable_raw_mode()?;
        }
        // A dream's busy frame, then (after the parent's acknowledgment) its
        // settled frame. The activity entry stays in both, as in the TUI.
        "dream-settle" => {
            crossterm::terminal::enable_raw_mode()?;
            let mut input = std::io::stdin().lock();
            let mut output = std::io::stdout().lock();
            let frame = |status: &str, composer: &str, dock: &str| {
                format!(
                    "\x1b[2J\x1b[1;1HActivity\x1b[2;1Hdream \u{b7} pool \u{b7} parts are consolidating\
                     \x1b[4;3H{status}\x1b[5;1H.    .    .    .\x1b[6;3H\u{203a} {composer}\
                     \x1b[8;3H{dock}\x1b[?25h\x1b[6;5H"
                )
            };
            output.write_all(
                frame(
                    "\u{280b} dream \u{b7} pool  \u{b7}  3s",
                    "Keep your next thought here\u{2026}",
                    "esc cancel  \u{b7}  alt+enter newline",
                )
                .as_bytes(),
            )?;
            output.flush()?;
            let mut acknowledgment = [0];
            input.read_exact(&mut acknowledgment)?;
            output.write_all(
                frame(
                    "\u{25c6} last \u{2248}10+10/100 tokens \u{b7} pool dream \u{b7} assumed",
                    "What shall we explore or build?",
                    "enter send  \u{b7}  alt+enter newline",
                )
                .as_bytes(),
            )?;
            output.flush()?;
            input.read_exact(&mut acknowledgment)?;
            crossterm::terminal::disable_raw_mode()?;
        }
        "error-unwind" => {
            let report = std::path::PathBuf::from(std::env::var("KURU_TERMINAL_ERROR_REPORT")?);
            let mut session = kuru::ui::TerminalSession::enter(&mut std::io::stdout())?;
            // The parent waits for this completed frame before sending the
            // key below. The fixture owns one real EventStream until that
            // input is observed, then drops it before restoration.
            std::io::stdout().write_all(b"\x1b[2J\x1b[HEVENTSTREAM_READY\x1b[?25h\x1b[1;18H")?;
            std::io::stdout().flush()?;
            let observed = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()?
                .block_on(async {
                    use futures::StreamExt as _;

                    let mut input = crossterm::event::EventStream::new();
                    let event = tokio::time::timeout(Duration::from_secs(5), input.next())
                        .await
                        .context("timed out waiting for native EventStream input")?;
                    match event {
                        Some(Ok(crossterm::event::Event::Key(key)))
                            if key.code == crossterm::event::KeyCode::Char('x') =>
                        {
                            Ok(())
                        }
                        Some(Ok(event)) => {
                            anyhow::bail!("unexpected native EventStream event: {event:?}")
                        }
                        Some(Err(error)) => Err(error.into()),
                        None => anyhow::bail!("native EventStream closed before the test key"),
                    }
                });
            let original = match &observed {
                Ok(()) => anyhow::anyhow!("injected native terminal input failure"),
                Err(error) => anyhow::anyhow!("{error:#}"),
            };
            // Mirror `ui::run`: restoration is explicit after a loop error,
            // before the original error is returned to the caller.
            let restore = session.restore();
            std::fs::write(
                report,
                format!("event: {observed:?}\noriginal: {original:#}\nrestore: {restore:?}"),
            )?;
        }
        "co-ready-events" => {
            let mut session = kuru::ui::TerminalSession::enter(&mut std::io::stdout())?;
            let observed = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()?
                .block_on(async {
                    use futures::{Stream as _, StreamExt as _};
                    use std::{future::poll_fn, pin::Pin, task::Poll};

                    let mut input = crossterm::event::EventStream::new();
                    let prematurely_ready = poll_fn(|context| {
                        Poll::Ready(match Pin::new(&mut input).poll_next(context) {
                            Poll::Pending => None,
                            Poll::Ready(event) => Some(event),
                        })
                    })
                    .await;
                    ensure!(
                        prematurely_ready.is_none(),
                        "EventStream was ready before the co-ready probe: {prematurely_ready:?}"
                    );
                    std::io::stdout()
                        .write_all(b"\x1b[2J\x1b[HCO_READY_EVENTSTREAM\x1b[?25h\x1b[1;21H")?;
                    std::io::stdout().flush()?;
                    nix::sys::signal::kill(
                        nix::unistd::Pid::this(),
                        nix::sys::signal::Signal::SIGSTOP,
                    )?;

                    let first = tokio::time::timeout(Duration::from_secs(5), input.next())
                        .await
                        .context("timed out waiting for the first co-ready terminal event")?
                        .context("native EventStream closed before the first co-ready event")??;
                    let second = tokio::time::timeout(Duration::from_secs(5), input.next())
                        .await
                        .context("timed out waiting for the second co-ready terminal event")?
                        .context("native EventStream closed before the second co-ready event")??;
                    Ok::<_, anyhow::Error>((first, second))
                });
            let restore = session.restore();
            let (first, second) = observed?;
            println!("CO_READY_EVENTS:{first:?}|{second:?}");
            std::io::stdout().flush()?;
            restore?;
        }
        "parallel-tool-activity" => {
            let mut session = kuru::ui::TerminalSession::enter(&mut std::io::stdout())?;
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;
            let mut view = kuru::ui::View::from_initial(
                kuru::ui::InitialViewData {
                    transcript: Vec::new().into(),
                    session: "fixture-session".into(),
                    project: "fixture-project".into(),
                    motion: false,
                    runtime: kuru::ui::RuntimeSnapshot {
                        turns: 0,
                        mode: "freudian".into(),
                        model: "fixture".into(),
                        effort: "default".into(),
                        parts: vec![("facing".into(), "Facing · voice".into())],
                        relationships: Vec::new(),
                        focus: None,
                    },
                    usage: None,
                },
                Vec::new(),
            );
            view.busy = true;
            view.speaker_id = "facing".into();
            view.speaker = "Facing".into();
            view.preview = Some(kuru_runtime::FacingProgress {
                turn_id: "fixture-turn".into(),
                request_round: 1,
                seq: 1,
                text_tail: "Concurrent checked reads".into(),
                text_truncated: false,
                summary_tail: String::new(),
                summary_truncated: false,
                activity: "Calling tool".into(),
                activity_truncated: false,
            });
            view.event(kuru_runtime::Event::ToolStarted {
                actor: "facing".into(),
                call_id: "call-a".into(),
                name: "file_read".into(),
            });
            view.event(kuru_runtime::Event::ToolStarted {
                actor: "facing".into(),
                call_id: "call-b".into(),
                name: "file_read".into(),
            });
            terminal.draw(|frame| kuru::ui::draw(frame, &view))?;

            let settled = |call_id: &str| {
                kuru_runtime::ToolObservation::projected(
                    call_id,
                    "file_read",
                    json!({"path":"withheld"}),
                    kuru_runtime::ToolOutcome::Ok,
                    Some(json!("fixture result")),
                    Duration::from_millis(1),
                )
            };
            let mut input = std::io::stdin().lock();
            let mut next = [0];
            input.read_exact(&mut next)?;
            view.event(kuru_runtime::Event::ToolSettled {
                actor: "facing".into(),
                observation: settled("call-b"),
            });
            terminal.draw(|frame| kuru::ui::draw(frame, &view))?;

            input.read_exact(&mut next)?;
            view.event(kuru_runtime::Event::ToolSettled {
                actor: "facing".into(),
                observation: settled("call-a"),
            });
            terminal.draw(|frame| kuru::ui::draw(frame, &view))?;

            input.read_exact(&mut next)?;
            view.preview = None;
            view.busy = false;
            view.transcript
                .push(("Facing".into(), "FINAL_PARALLEL_RESULT".into()));
            terminal.draw(|frame| kuru::ui::draw(frame, &view))?;

            input.read_exact(&mut next)?;
            drop(terminal);
            session.restore()?;
        }
        other => anyhow::bail!("unknown fixture mode {other}"),
    }
    Ok(())
}

// Runs an inner terminal and reports its size. Only this PTY reaches the
// parent, so a failure carries the inner child's launch, exit state and
// complete output as well as its own error.
fn nested_size_fixture(inner: &str) -> Result<()> {
    let mut terminal = fixture_with_size(inner, 35, 120)?;
    if let Err(error) = terminal
        .wait_text(&["SIZE:120x35"], &[])
        .and_then(|()| terminal.wait_exit(EXIT_TIMEOUT))
    {
        return Err(error.context(format!(
            "nested {inner:?} fixture failed; inner {}",
            terminal.report()
        )));
    }
    println!("INNER_SIZE_OK");
    std::io::stdout().flush()?;
    Ok(())
}

fn fixture(mode: &str) -> Result<Terminal> {
    fixture_with_size(mode, 35, 120)
}

fn fixture_with_size(mode: &str, rows: u16, cols: u16) -> Result<Terminal> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "terminal_fixture_process",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KURU_TERMINAL_FIXTURE", mode);
    Terminal::spawn(command, rows, cols)
}

fn error_unwind_fixture(report: &std::path::Path) -> Result<Terminal> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "terminal_fixture_process",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KURU_TERMINAL_FIXTURE", "error-unwind")
        .env("KURU_TERMINAL_ERROR_REPORT", report);
    Terminal::spawn(command, 35, 120)
}

#[test]
fn terminal_driver_drains_backpressure_and_bounds_stalled_processes() -> Result<()> {
    let mut pressure = fixture("pressure")?;
    pressure.wait_exit(EXIT_TIMEOUT)?;
    let start = pressure
        .output
        .windows(8)
        .position(|bytes| bytes == b"PAYLOAD:")
        .context("missing payload marker")?
        + 8;
    assert_eq!(
        &pressure.output[start..start + 1_048_576],
        vec![b'x'; 1_048_576]
    );
    assert_eq!(
        &pressure.output[start + 1_048_576..start + 1_048_576 + 5],
        b":DONE"
    );

    let mut stalled = fixture("stalled")?;
    stalled.wait_text(&["STALLED"], &[])?;
    let error = stalled.wait_exit(Duration::from_millis(100)).unwrap_err();
    assert!(
        error.to_string().contains("process exits: timed out"),
        "{error}"
    );
    assert!(error.to_string().contains("STALLED"), "{error}");
    Ok(())
}

#[test]
fn terminal_timeouts_report_the_launch_and_a_process_tree_snapshot() -> Result<()> {
    let mut stalled = fixture("stalled")?;
    stalled.wait_text(&["STALLED"], &[])?;
    let exit = stalled.wait_exit(Duration::from_millis(100)).unwrap_err();
    let wait = stalled
        .wait("never satisfied", Duration::from_millis(100), |_| Ok(false))
        .unwrap_err();
    for error in [exit.to_string(), wait.to_string()] {
        for required in [
            "timed out after 100ms",
            "STALLED",
            "launch: program=",
            "--exact",
            "terminal_fixture_process",
            "LLVM_PROFILE_FILE=",
            "child running",
            "complete PTY output (",
            "process tree of ",
            "stat=",
        ] {
            assert!(error.contains(required), "missing {required}: {error}");
        }
        assert!(!error.contains("snapshot unavailable"), "{error}");
    }
    assert!(exit.to_string().contains("process exits: timed out"));
    Ok(())
}

#[test]
fn terminal_exit_drain_rejects_queued_bytes_and_completion_after_its_deadline() -> Result<()> {
    terminal::exit_drain_deadline_probe()
}

#[test]
fn terminal_wait_drains_the_line_written_just_before_exit() -> Result<()> {
    // The child writes LATE and exits at once. A wait that sees the exit must
    // not drop that last line from what the caller observes afterwards.
    let mut terminal = fixture("late-output")?;
    let mut paused = false;
    let error = terminal
        .wait("late output becomes visible", READY_TIMEOUT, |terminal| {
            // Release LATE only once reading has stopped, and hold the wait
            // until the child has exited, so LATE is queued unread at the exit.
            if !paused && terminal.output.windows(5).any(|bytes| bytes == b"EARLY") {
                paused = true;
                terminal.send(b"\n")?;
                terminal.wait_exited(READY_TIMEOUT)?;
            }
            Ok(false)
        })
        .unwrap_err()
        .to_string();
    assert!(paused, "{error}");
    // The wait drained the queued line into `output` and the screen, in order,
    // before it returned; no later read is needed to observe it.
    let output = String::from_utf8_lossy(&terminal.output);
    let early = output.find("EARLY").with_context(|| error.clone())?;
    let late = output
        .find("LATE")
        .with_context(|| format!("the line written just before exit was dropped: {error}"))?;
    assert!(early < late, "{error}");
    assert!(terminal.screen().contains("LATE"), "{error}");
    let (before, after) = error
        .split_once("output not yet read when the exit was seen")
        .with_context(|| format!("no late-output section: {error}"))?;
    assert!(before.contains("process exited"), "{error}");
    assert!(before.contains("code: 0"), "{error}");
    assert!(before.contains("EARLY"), "{error}");
    assert!(!before.contains("LATE"), "{error}");
    assert!(after.contains("LATE"), "{error}");
    assert!(after.contains("EOF reached=true"), "{error}");
    // The report's complete output includes what was still queued.
    let (_, complete) = after
        .split_once("complete PTY output (")
        .with_context(|| format!("no complete output: {error}"))?;
    assert!(
        complete.contains("EARLY") && complete.contains("LATE"),
        "{error}"
    );
    assert!(after.contains("child exited"), "{error}");
    Ok(())
}

#[test]
fn terminal_fixture_nested_early_exit_reports_the_inner_output_and_status() -> Result<()> {
    let mut outer = fixture("nested-early-exit")?;
    let error = outer.wait_exit(EXIT_TIMEOUT).unwrap_err().to_string();
    for required in [
        "child failed",
        "nested",
        "early-exit",
        "fixture failed; inner launch: program=",
        "process exited",
        "code: 0",
        "PARTIAL",
        "complete PTY output (",
    ] {
        assert!(error.contains(required), "missing {required}: {error}");
    }
    Ok(())
}

#[test]
fn terminal_fixture_uses_its_requested_controlling_dimensions() -> Result<()> {
    // The outer fixture deliberately owns a terminal too small to satisfy the
    // inner assertion. The inner spawn must establish its own controlling PTY
    // instead of inheriting these dimensions through `/dev/tty`.
    let mut outer = fixture_with_size("nested-dimensions", 7, 19)?;
    outer.wait(
        "nested terminal reports its own size",
        READY_TIMEOUT,
        |terminal| {
            Ok(terminal
                .output
                .windows(b"INNER_SIZE_OK".len())
                .any(|bytes| bytes == b"INNER_SIZE_OK"))
        },
    )?;
    outer.wait_exit(EXIT_TIMEOUT)
}

#[test]
fn terminal_reader_cleanup_is_bounded_while_its_source_remains_open() -> Result<()> {
    let elapsed = terminal::bounded_reader_cleanup_probe(Duration::from_millis(100))?;
    assert!(elapsed < Duration::from_secs(1));
    Ok(())
}

#[test]
fn terminal_driver_waits_for_complete_frames_before_checking_quiescence() -> Result<()> {
    let mut terminal = fixture("fragmented-frame")?;
    terminal.wait_text(&["focus draft"], &[])?;
    let text_only_baseline = terminal.output.len();
    // The child cannot send the cursor trailer until the parent permits it.
    // A text-only wait incorrectly succeeds here, before the frame is complete.
    let error = terminal
        .wait_composer_frame(&["focus draft"], Duration::from_millis(100))
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");

    terminal.send(b"n")?;
    terminal.wait("partial cursor trailer", READY_TIMEOUT, |terminal| {
        Ok(terminal.output.ends_with(b"\x1b[3;"))
    })?;
    let error = terminal
        .wait_composer_frame(&["focus draft"], Duration::from_millis(100))
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");

    terminal.send(b"n")?;
    terminal.wait_composer_frame(&["focus draft"], READY_TIMEOUT)?;
    assert!(terminal.output.len() > text_only_baseline);
    let settled = terminal.output.len();
    terminal.read_for(Duration::from_millis(100))?;
    terminal.assert_no_output_since(settled, "unfocused terminal is animating")?;

    // A subsequent visual update must still fail the same strict assertion
    // used by the application test; completing a frame grants no tolerance.
    terminal.send(b"n")?;
    terminal.wait_text(&["animated draft"], &[])?;
    let error = terminal
        .assert_no_output_since(settled, "unfocused terminal is animating")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unfocused terminal is animating"),
        "{error}"
    );
    terminal.send(b"q")?;
    terminal.wait_exit(EXIT_TIMEOUT)
}

#[test]
fn terminal_driver_awaits_dream_completion_not_a_stale_dream_entry() -> Result<()> {
    let mut terminal = fixture("dream-settle")?;
    terminal.wait_text(&["dream \u{b7} pool  \u{b7}  3s"], &[])?;
    // While the dream runs the composer shows the busy placeholder: the wait
    // expires at its bound and reports the captured screen.
    let error = terminal
        .wait_dream_settled(Duration::from_millis(200))
        .unwrap_err()
        .to_string();
    assert!(error.contains("the dream settles: timed out"), "{error}");
    assert!(error.contains("dream \u{b7} pool  \u{b7}  3s"), "{error}");
    assert!(error.contains("Keep your next thought here"), "{error}");

    // The settled frame keeps the activity entry and a `dream` request label
    // in the idle status row; neither is the running dream.
    terminal.send(b"n")?;
    terminal.wait_dream_settled(READY_TIMEOUT)?;
    let screen = terminal.screen();
    assert!(
        screen.contains("dream \u{b7} pool \u{b7} parts are consolidating"),
        "{screen}"
    );
    terminal.send(b"q")?;
    terminal.wait_exit(EXIT_TIMEOUT)
}

#[test]
fn real_pty_error_unwind_restores_terminal_and_reports_the_original_error() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let report = directory.path().join("error.txt");
    let mut terminal = error_unwind_fixture(&report)?;
    terminal.wait_composer_frame(&["EVENTSTREAM_READY"], READY_TIMEOUT)?;
    terminal.send(b"x")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    let report = std::fs::read_to_string(report)?;
    assert!(report.contains("event: Ok(())"), "{report}");
    assert!(
        report.contains("injected native terminal input failure"),
        "{report}"
    );
    assert!(report.contains("restore: Ok(())"), "{report}");
    terminal.assert_restored()
}

#[test]
fn real_event_stream_preserves_co_ready_resize_and_paste() -> Result<()> {
    for attempt in 0..32 {
        let mut terminal = fixture("co-ready-events")?;
        terminal.wait_text(&["CO_READY_EVENTSTREAM"], &[])?;
        terminal.wait_stopped(READY_TIMEOUT)?;
        terminal.resize(20, 65)?;
        terminal.send(b"\x1b[200~pasted text\x1b[201~")?;
        terminal.resume()?;
        terminal
            .wait_text(&["Resize(65, 20)", "Paste(\"pasted text\")"], &[])
            .with_context(|| format!("co-ready EventStream attempt {attempt}"))?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
    }
    Ok(())
}

#[derive(Clone)]
struct ComposerProvider {
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn composer_complete(
    State(state): State<ComposerProvider>,
    Json(request): Json<Value>,
) -> Response {
    state.requests.lock().unwrap().push(request);
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{
                    "id":"composer-fixture",
                    "status":"completed",
                    "output":[{"type":"message","content":[{
                        "type":"output_text","text":"COMPOSER_FINAL"
                    }]}],
                    "usage":{"input_tokens":8,"output_tokens":5}
                }
            })
        ),
    )
        .into_response()
}

fn json_contains_exact_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text == expected,
        Value::Array(items) => items
            .iter()
            .any(|item| json_contains_exact_text(item, expected)),
        Value::Object(fields) => fields
            .values()
            .any(|item| json_contains_exact_text(item, expected)),
        _ => false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_public_pages_literal_search_and_deep_item_anchor_at_120_and_80() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use kuru_core::Message;
        use kuru_memory::{PublicTurnSettlement, SessionTurnCheckpoint};
        let sandbox = Sandbox::warmed().await?;
        let created = sandbox
            .command("demo")
            .args(["run", "public navigation origin", "--json"])
            .output()?;
        ensure!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let created: Value = serde_json::from_slice(&created.stdout)?;
        let session = created["session"]
            .as_str()
            .context("navigation session missing")?
            .to_owned();
        let options = memory_options(&sandbox)?;
        let (_, opening) = MemoryStore::open_managed_observed(
            options.clone(),
            sandbox.project.canonicalize()?,
            PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
        );
        let memory = opening.await?;
        let (claimed, driver) = memory.bind_project_driver(&sandbox.project).await?;
        memory.close().await?;
        let memory = claimed;
        let seeded: Result<()> = async {
            driver
                .select(kuru_memory::SessionDriverTarget::Catalog(Box::new(
                    memory
                        .session_catalog_record(&session)
                        .await?
                        .context("navigation claim catalog")?,
                )))
                .await?;
            let generation = memory
                .session_catalog_record(&session)
                .await?
                .context("navigation catalog missing")?
                .lifecycle_generation;
            let namespace = format!(
                "{}/transcript/{session}",
                kuru_runtime::project_scope(&sandbox.project)?
            );
            for index in 0..96 {
                let turn = format!("navigation-{index:03}");
                let marker = format!("SEARCH_OLD_{index:03}");
                memory
                    .checkpoint_session_turn(
                        &namespace,
                        &session,
                        &[Message::text("user", format!("old question {marker}"))],
                        &[],
                        &SessionTurnCheckpoint::Admit {
                            expected_generation: generation,
                            turn_id: turn.clone(),
                            label: None,
                            expected_transcript_rows: None,
                        },
                    )
                    .await?;
                let answer = if index == 95 {
                    format!(
                        "{}END_SINGLE_ITEM\nATTACK\u{1b}]2;FOREIGN_TITLE\u{7}",
                        "LONG_PUBLIC_ROW 👨‍👩‍👧‍👦 👍🏽 e\u{301}\n".repeat(70_000)
                    )
                } else {
                    format!("older answer {marker}")
                };
                memory
                    .checkpoint_session_turn(
                        &namespace,
                        &session,
                        &[Message::text("assistant", answer)],
                        &[],
                        &SessionTurnCheckpoint::Settle {
                            expected_generation: generation,
                            turn_id: turn,
                            settlement: PublicTurnSettlement::Completed,
                            speaker_id: "fixture-speaker".into(),
                        },
                    )
                    .await?;
            }
            ensure!(
                memory
                    .public_transcript_page(&session, None, 32)
                    .await?
                    .total_rows
                    >= 97
            );
            Ok(())
        }
        .await;
        let released = driver.close().await;
        let closed = memory.close().await;
        let quiesced = kuru_memory::test_support::await_managed_quiescence(&options).await;
        seeded?;
        released?;
        closed?;
        quiesced?;
        let mut command = sandbox.command("demo");
        command
            .args(["--resume", &session])
            .env("KURU_REDUCED_MOTION", "1")
            .env("TERM", "xterm-256color");
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal
            .wait_composer_frame(&["END_SINGLE_ITEM", "enter send"], sandbox.startup_timeout)?;
        terminal.send(b"\x1b[5~")?;
        terminal
            .wait_composer_frame(&["LONG_PUBLIC_ROW", "history", "enter send"], READY_TIMEOUT)?;
        terminal.send("literal draft 猫".as_bytes())?;
        terminal.wait_composer_frame(
            &["LONG_PUBLIC_ROW", "literal draft 猫", "enter send"],
            READY_TIMEOUT,
        )?;
        terminal.resize(24, 80)?;
        terminal.wait_composer_frame(
            &["LONG_PUBLIC_ROW", "literal draft 猫", "enter send"],
            READY_TIMEOUT,
        )?;
        terminal.send(b"\x06SEARCH_OLD_003")?;
        terminal.wait_composer_frame(
            &["older answer SEARCH_OLD_003", "Match", "literal draft 猫"],
            READY_TIMEOUT,
        )?;
        ensure!(
            !terminal.screen().contains("old question SEARCH_OLD_003"),
            "search skipped the exact answer anchor: {}",
            terminal.screen()
        );
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(
            &["old question SEARCH_OLD_003", "Match", "literal draft 猫"],
            READY_TIMEOUT,
        )?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(
            &["LONG_PUBLIC_ROW", "literal draft 猫", "enter send"],
            READY_TIMEOUT,
        )?;
        ensure!(!String::from_utf8_lossy(&terminal.output).contains("\u{1b}]2;FOREIGN_TITLE"));
        terminal.send(b"\x03")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
        ensure!(
            terminal
                .output
                .windows(b"\x1b[22;2t".len())
                .any(|bytes| bytes == b"\x1b[22;2t")
        );
        ensure!(
            terminal
                .output
                .windows(b"\x1b[23;2t".len())
                .any(|bytes| bytes == b"\x1b[23;2t")
        );
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_composer_recall_and_paste_chips_keep_literal_prompts_at_120_and_80() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(composer_complete))
            .with_state(ComposerProvider {
                requests: requests.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("composer-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.send(b"history seed\r")?;
        terminal.wait_composer_frame(&["COMPOSER_FINAL", "enter send"], READY_TIMEOUT)?;
        terminal.send("draft 猫".as_bytes())?;
        terminal.wait_composer_frame(&["draft 猫", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\x1bOQ")?;
        terminal.wait("completed model picker frame", READY_TIMEOUT, |terminal| {
            let screen = terminal.screen();
            Ok(screen.contains("Models")
                && screen.contains("fixture")
                && terminal.output.ends_with(b"\x1b[?25l"))
        })?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(&["draft 猫", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\x12history")?;
        terminal.wait_composer_frame(&["reverse search", "history seed"], READY_TIMEOUT)?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(&["draft 猫", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[A")?;
        terminal.wait_composer_frame(&["history 1/1", "history seed"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[B")?;
        terminal.wait_composer_frame(&["draft 猫", "enter send"], READY_TIMEOUT)?;

        let pasted = "猫\n".repeat(300);
        terminal.send(b"pre")?;
        terminal.send(format!("\x1b[200~{pasted}\x1b[201~").as_bytes())?;
        terminal.send(b"post")?;
        terminal.wait_composer_frame(&["pre[paste ·", "post"], READY_TIMEOUT)?;
        terminal.resize(24, 80)?;
        terminal.wait_composer_frame(&["[paste ·", "post"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[D\x1b[D\x1b[D\x1b[D")?;
        terminal.wait_composer_frame(&["Ctrl+G expand/compact"], READY_TIMEOUT)?;
        terminal.send(b"\x07")?;
        terminal.wait_composer_frame(&["Paste expanded", "猫"], READY_TIMEOUT)?;
        terminal.send(b"\x07")?;
        terminal.wait_composer_frame(&["[paste ·"], READY_TIMEOUT)?;
        terminal.send(b"\x18")?;
        terminal.wait_composer_frame(&["draft 猫prepost"], READY_TIMEOUT)?;
        ensure!(
            !terminal.screen().contains("[paste ·"),
            "removed chip remained: {}",
            terminal.screen()
        );
        terminal.send(b"\r")?;
        terminal.wait(
            "provider received exact post-removal prompt",
            READY_TIMEOUT,
            |_| {
                Ok(requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| json_contains_exact_text(request, "draft 猫prepost")))
            },
        )?;
        terminal.wait_composer_frame(&["COMPOSER_FINAL", "enter send"], READY_TIMEOUT)?;

        terminal.send(b"pre")?;
        terminal.send(format!("\x1b[200~{pasted}\x1b[201~").as_bytes())?;
        terminal.send(b"post\r")?;
        let literal = format!("pre{pasted}post");
        terminal.wait(
            "provider received exact compact-paste literal",
            READY_TIMEOUT,
            |_| {
                Ok(requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| json_contains_exact_text(request, &literal)))
            },
        )?;
        terminal.wait_composer_frame(&["COMPOSER_FINAL", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\x03")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[test]
fn real_pty_parallel_activity_tracks_each_same_name_call() -> Result<()> {
    let mut terminal = fixture("parallel-tool-activity")?;
    terminal.wait_composer_frame(&["activity · Calling file_read · 2 active"], READY_TIMEOUT)?;
    terminal.send(b"n")?;
    terminal.wait_composer_frame(&["activity · Calling file_read"], READY_TIMEOUT)?;
    ensure!(
        !terminal.screen().contains("2 active"),
        "settling call-b did not leave only call-a: {}",
        terminal.screen()
    );
    terminal.send(b"n")?;
    terminal.wait_composer_frame(&["activity · Responding"], READY_TIMEOUT)?;
    ensure!(
        !terminal.screen().contains("Calling file_read"),
        "the activity label survived both exact settlements: {}",
        terminal.screen()
    );
    terminal.send(b"n")?;
    terminal.wait_composer_frame(&["FINAL_PARALLEL_RESULT"], READY_TIMEOUT)?;
    let final_frame = terminal.screen();
    ensure!(
        !final_frame.contains("Calling file_read")
            && !final_frame.contains("2 active")
            && !final_frame.contains("activity · Responding"),
        "settled activity survived the final answer: {final_frame}"
    );
    terminal.send(b"q")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

/// The standing dock, which every frame keeps below the transient status slot.
fn dock(screen: &str) -> String {
    screen.lines().rev().take(6).collect::<Vec<_>>().join("\n")
}

#[test]
fn real_pty_cost_inspection_survives_120_80_and_40_columns() -> Result<()> {
    let sandbox = Sandbox::new()?;
    let mut terminal = Terminal::spawn(sandbox.command("demo"), 35, 120)?;
    terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
    terminal.command("hello", None)?;
    terminal.wait_composer_frame(&["[demo]", "enter send"], READY_TIMEOUT)?;
    for (rows, cols) in [(35, 120), (24, 80), (18, 40)] {
        terminal.resize(rows, cols)?;
        terminal.command("/cost", None)?;
        terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
        let screen = terminal.screen();
        ensure!(
            screen.contains("unknown"),
            "unknown price must remain explicit at {cols}x{rows}: {screen}"
        );
        ensure!(
            screen.contains("assumed") || screen.contains("built-in assumption"),
            "the last prepared request must show its assumed bound at {cols}x{rows}: {screen}"
        );
        if cols == 120 {
            ensure!(screen.contains("Session usage"), "{screen}");
            ensure!(screen.contains("not a subscription charge"), "{screen}");
        }
    }
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_commands_complete_and_clear_only_the_visible_conversation() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = Arc::clone(&requests);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move |Json(request): Json<Value>| {
                    let captured = Arc::clone(&captured);
                    async move {
                        captured.lock().unwrap().push(request);
                        priced_complete(Json(json!({}))).await
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("command-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 48, 120)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;

        terminal.send(b"/mem\t")?;
        terminal.wait_composer_frame(&["/memory", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\t")?;
        terminal
            .wait_composer_frame(&["/memory-candidate-abandon", "enter send"], READY_TIMEOUT)?;
        terminal.send(&[127; 25])?;
        terminal.wait_composer_frame(
            &["What shall we explore or build?", "enter send"],
            READY_TIMEOUT,
        )?;
        terminal.command("/help", None)?;
        // Help is a real scrollable transcript at this height. Inspect both
        // completed pages instead of requiring top and bottom commands at once.
        terminal.wait_composer_frame(&["/status", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[5~")?;
        terminal.wait_composer_frame(&["/clear", "/compact [ID]", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[6~")?;
        terminal.wait_composer_frame(&["/status", "enter send"], READY_TIMEOUT)?;

        let before_compact = requests.lock().unwrap().len();
        terminal.send(b"/comp\t")?;
        terminal.wait_composer_frame(&["/compact", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(
            &[
                "No eligible uncompacted history",
                "original records remain stored",
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(
            requests.lock().unwrap().len() == before_compact,
            "an empty manual compact reached the provider"
        );

        let compact_actor = kuru_core::ModeProfile::builtin(sandbox.config()?.mode)
            .roles
            .seeds()
            .into_iter()
            .next()
            .context("mode omitted its first compactable actor")?
            .id;
        terminal.command(&format!("/focus {compact_actor}"), None)?;
        terminal.wait_composer_frame(&["Speaking focus", "enter send"], READY_TIMEOUT)?;
        terminal.command("OLDER_VISIBLE_MARKER", None)?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let after_first = requests.lock().unwrap().len();
        ensure!(
            after_first > before_compact,
            "first turn never reached the provider"
        );
        terminal.command(&format!("/compact {compact_actor}"), None)?;
        terminal.wait_composer_frame(
            &[
                &format!("Compacted {compact_actor}"),
                "source sequences",
                "original records remain stored",
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        let after_named_compact = requests.lock().unwrap().len();
        ensure!(
            after_named_compact > after_first,
            "named manual compact never reached the provider"
        );
        ensure!(
            requests.lock().unwrap()[after_first..after_named_compact]
                .iter()
                .all(|request| !request.to_string().contains("/compact")),
            "slash command text reached a compaction provider request"
        );
        terminal.command(&format!("/compact {compact_actor}"), None)?;
        terminal.wait_composer_frame(
            &[
                &format!("No eligible uncompacted history for {compact_actor}"),
                "original records remain stored",
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(
            requests.lock().unwrap().len() == after_named_compact,
            "manual no-op repeated the provider request"
        );
        let sessions = sandbox.sessions()?;
        ensure!(
            sessions.len() == 1 && sessions[0].turns == 1,
            "{sessions:?}"
        );
        let session = &sessions[0].id;

        terminal.command("/status", None)?;
        terminal.wait_composer_frame(
            &[
                "Session:",
                session,
                "Project:",
                "Model: fixture",
                "Effort:",
                "Mode:",
                &format!("Focus: {compact_actor}"),
                "Turns: 1",
                "Session usage",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(
            requests.lock().unwrap().len() == after_named_compact,
            "/status made a provider request"
        );
        terminal.send(b"/cle\t")?;
        terminal.wait_composer_frame(&["/clear", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(&["stored history unchanged", "enter send"], READY_TIMEOUT)?;
        ensure!(
            !terminal.screen().contains("OLDER_VISIBLE_MARKER"),
            "/clear retained old rows on the screen: {}",
            terminal.screen()
        );
        ensure!(
            requests.lock().unwrap().len() == after_named_compact,
            "/clear made a provider request"
        );
        terminal.command("FOLLOWUP_VISIBLE_MARKER", None)?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let captured = requests.lock().unwrap();
        ensure!(
            captured.len() > after_named_compact,
            "follow-up never reached the provider"
        );
        ensure!(
            captured[after_named_compact..]
                .iter()
                .any(|request| request.to_string().contains("OLDER_VISIBLE_MARKER")),
            "the cleared turn was absent from follow-up provider context"
        );
        drop(captured);

        let before_lifecycle = requests.lock().unwrap().len();
        terminal.command("/sessions", Some("Sessions"))?;
        terminal.send(b"\x06")?;
        terminal.wait_text(&["Settled boundaries", "Enter fork"], &[])?;
        terminal.resize(22, 65)?;
        terminal.wait_text(&["Settled boundaries", "Enter fork"], &[])?;
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(
            &["current project memory remains shared", "enter send"],
            READY_TIMEOUT,
        )?;
        ensure!(
            requests.lock().unwrap().len() == before_lifecycle,
            "session picker fork made a provider request"
        );
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;

        let forked = sandbox.sessions()?;
        let child = forked
            .iter()
            .find(|candidate| candidate.id != *session)
            .map(|candidate| candidate.id.clone())
            .context("session picker did not publish a fork")?;

        let mut resume = sandbox.command("responses");
        resume
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .arg("--resume")
            .arg(&child)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut resumed = Terminal::spawn(resume, 48, 120)?;
        resumed.wait_composer_frame(
            &["OLDER_VISIBLE_MARKER", "enter send"],
            sandbox.startup_timeout,
        )?;
        resumed.send(b"/quit\r")?;
        resumed.wait_exit(EXIT_TIMEOUT)?;
        resumed.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_and_pty_session_actions_share_catalog_identity_and_public_transcript() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let created = sandbox
            .command("demo")
            .args(["run", "CLI-PARITY-ORIGIN", "--json"])
            .output()?;
        ensure!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let created: Value = serde_json::from_slice(&created.stdout)?;
        let source = created["session"]
            .as_str()
            .context("CLI source session missing")?
            .to_owned();
        let renamed = sandbox
            .command("demo")
            .args(["sessions", "rename", &source, "CLI-LABEL"])
            .output()?;
        ensure!(
            renamed.status.success(),
            "{}",
            String::from_utf8_lossy(&renamed.stderr)
        );
        let renamed: Value = serde_json::from_slice(&renamed.stdout)?;
        ensure!(renamed["session_id"] == source);
        let listed = sandbox.command("demo").arg("sessions").output()?;
        ensure!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        let listed: Value = serde_json::from_slice(&listed.stdout)?;
        let source_record = listed
            .as_array()
            .context("CLI sessions is not an array")?
            .iter()
            .find(|row| row["id"] == source)
            .context("renamed CLI source is absent")?;
        let source_node = source_record["head_node_id"]
            .as_str()
            .context("CLI source head missing")?
            .to_owned();
        let cli_action = |args: &[&str]| -> Result<Value> {
            let output = sandbox.command("demo").args(args).output()?;
            ensure!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(serde_json::from_slice(&output.stdout)?)
        };
        let cli_removed = cli_action(&[
            "sessions",
            "fork",
            &source,
            &source_node,
            "--label",
            "CLI-REMOVED",
        ])?;
        let cli_removed = cli_removed["session_id"]
            .as_str()
            .context("CLI removed child ID missing")?
            .to_owned();
        ensure!(cli_action(&["sessions", "remove", &cli_removed])?["lifecycle_state"] == "removed");
        let cli_pending = cli_action(&[
            "sessions",
            "fork",
            &source,
            &source_node,
            "--label",
            "CLI-PENDING",
        ])?;
        let cli_pending = cli_pending["session_id"]
            .as_str()
            .context("CLI pending child ID missing")?
            .to_owned();
        ensure!(cli_action(&["sessions", "remove", &cli_pending])?["lifecycle_state"] == "removed");
        ensure!(cli_action(&["sessions", "restore", &cli_pending])?["lifecycle_state"] == "active");
        let options = memory_options(&sandbox)?;
        let (_, opening) = MemoryStore::open_managed_observed(
            options.clone(),
            sandbox.project.canonicalize()?,
            PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
        );
        let memory = opening.await.context("attach managed CLI catalog")?;
        let (claimed, driver) = memory.bind_project_driver(&sandbox.project).await?;
        memory.close().await?;
        let memory = claimed;
        driver
            .select(kuru_memory::SessionDriverTarget::Catalog(Box::new(
                memory
                    .session_catalog_record(&cli_pending)
                    .await?
                    .context("pending claim catalog")?,
            )))
            .await?;
        let generation = memory
            .session_catalog_record(&cli_pending)
            .await?
            .context("restored CLI child catalog missing")?
            .lifecycle_generation;
        let namespace = format!(
            "{}/transcript/{cli_pending}",
            kuru_runtime::project_scope(&sandbox.project)?
        );
        memory
            .checkpoint_session_turn(
                &namespace,
                &cli_pending,
                &[kuru_core::Message::text("user", "CLI-PENDING-TURN")],
                &[],
                &kuru_memory::SessionTurnCheckpoint::Admit {
                    expected_generation: generation,
                    turn_id: "cli-pending-turn".into(),
                    label: None,
                    expected_transcript_rows: Some(0),
                },
            )
            .await?;
        driver.close().await?;
        memory.close().await?;
        kuru_memory::test_support::await_managed_quiescence(&options).await?;

        let requests = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move |Json(_request): Json<Value>| {
                    let counted = Arc::clone(&counted);
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        priced_complete(Json(json!({}))).await
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("parity-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .arg("--resume")
            .arg(&source)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 48, 120)?;
        terminal.wait_composer_frame(
            &["CLI-PARITY-ORIGIN", "enter send"],
            sandbox.startup_timeout,
        )?;
        terminal.command("/help", None)?;
        terminal.wait_composer_frame(
            &[
                "/file-undo",
                "/memory-candidate-abandon",
                "/sessions",
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        // The shared registry has grown beyond one 48-row viewport. Page the real
        // transcript instead of requiring its top and bottom entries at once.
        terminal.send(b"\x1b[5~")?;
        terminal.wait_composer_frame(&["/new", "/resume", "/export"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[6~")?;
        terminal.wait_composer_frame(&["/tools", "enter send"], READY_TIMEOUT)?;
        for (command, result) in [
            ("/file-checkpoints", "[]"),
            ("/file-inspect missing-id", "file checkpoint does not exist"),
            ("/file-prune missing-id", "file checkpoint does not exist"),
            ("/file-undo missing-id", "file checkpoint does not exist"),
            ("/memory-candidates", "\"candidates\": []"),
            (
                "/memory-candidate-status",
                "usage: /memory-candidate-status BRANCH",
            ),
            (
                "/memory-candidate-abandon",
                "usage: /memory-candidate-abandon BRANCH BASE HEAD",
            ),
        ] {
            terminal.command(command, None)?;
            terminal.wait_composer_frame(&[result, "enter send"], READY_TIMEOUT)?;
            ensure!(
                !terminal.screen().contains("Unknown command"),
                "registered recovery command was not dispatched: {command}"
            );
        }
        ensure!(
            requests.load(Ordering::SeqCst) == 0,
            "registered recovery command reached the lifecycle provider"
        );
        terminal.command("/sessions", Some("Sessions"))?;
        terminal.wait_text(
            &[
                "CLI-LABEL",
                "CLI-REMOVED",
                "CLI-PENDING",
                "removed",
                "pending",
            ],
            &[],
        )?;
        let picker = terminal.screen();
        let picker_lines = picker.lines().collect::<Vec<_>>();
        ensure!(
            picker_lines.iter().any(|line| {
                line.contains(&cli_removed)
                    && line.contains("CLI-REMOVED")
                    && line.contains("removed")
            }),
            "CLI removed child was not distinct in the PTY picker: {picker}"
        );
        ensure!(
            picker_lines.iter().any(|line| {
                line.contains(&cli_pending)
                    && line.contains("CLI-PENDING")
                    && line.contains("pending")
            }),
            "CLI restored pending child was not distinct in the PTY picker: {picker}"
        );
        let active_order = sandbox.sessions()?;
        for pair in active_order.windows(2) {
            let left = picker_lines
                .iter()
                .position(|line| line.contains(&pair[0].id))
                .context("CLI active session absent from picker")?;
            let right = picker_lines
                .iter()
                .position(|line| line.contains(&pair[1].id))
                .context("CLI active session absent from picker")?;
            ensure!(
                left < right,
                "CLI and PTY active catalog order differs: {picker}"
            );
        }
        terminal.resize(48, 80)?;
        // A picker intentionally hides the composer cursor, so inspect its
        // completed visible row after the narrower redraw.
        terminal.wait_text(&["Sessions", "CLI-PENDING", "pending"], &[])?;
        let narrow_picker = terminal.screen();
        ensure!(
            narrow_picker.lines().any(|line| {
                line.contains(&cli_pending)
                    && line.contains("pending")
                    && line.contains("CLI-PENDING")
            }),
            "80-column session picker hid pending identity or state: {narrow_picker}"
        );
        terminal.resize(48, 120)?;
        terminal.wait_text(&["Sessions", "CLI-PENDING"], &[])?;
        terminal.send(b"\x0c")?;
        terminal.wait_composer_frame(&["/session-rename", &source, "enter send"], READY_TIMEOUT)?;
        terminal.send(b"TUI-LABEL\r")?;
        terminal.wait_text(&["TUI-LABEL", "Sessions"], &[])?;
        ensure!(
            sandbox
                .sessions()?
                .iter()
                .any(|row| row.id == source && row.label == "TUI-LABEL"),
            "CLI listing did not observe PTY rename"
        );

        terminal.send(b"\x1b")?;
        terminal.command("/new", None)?;
        terminal.wait_composer_frame(&["Created session", "enter send"], READY_TIMEOUT)?;
        let active = sandbox.sessions()?;
        ensure!(active.len() == 3, "{active:?}");
        let new_session = active
            .iter()
            .find(|row| row.id != source && row.id != cli_pending)
            .context("PTY new session missing from CLI")?;
        ensure!(
            new_session.turns == 0,
            "new session inherited transcript rows"
        );
        terminal.resize(24, 65)?;
        terminal.command("/sessions", Some("Sessions"))?;
        let source_prefix = &source[..8];
        terminal.send(source_prefix.as_bytes())?;
        terminal.wait_text(&["Sessions", &format!("/ {source_prefix}")], &[])?;
        terminal.send(b"\x1b[3~")?;
        terminal.wait_text(&["Sessions", "Type to filter"], &[])?;
        ensure!(
            sandbox.sessions()?.iter().all(|row| row.id != source),
            "CLI still listed PTY-removed source"
        );
        terminal.send(source_prefix.as_bytes())?;
        terminal.wait_text(&["Sessions", &format!("/ {source_prefix}")], &[])?;
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(&["Session is removed", "enter send"], READY_TIMEOUT)?;
        terminal.command("/sessions", Some("Sessions"))?;
        terminal.send(source_prefix.as_bytes())?;
        terminal.wait_text(&["Sessions", &format!("/ {source_prefix}")], &[])?;
        terminal.send(b"\x12")?;
        terminal.wait_text(&["Sessions", "Type to filter"], &[])?;
        ensure!(
            sandbox
                .sessions()?
                .iter()
                .any(|row| row.id == source && row.label == "TUI-LABEL"),
            "CLI did not observe PTY restore"
        );
        terminal.send(source_prefix.as_bytes())?;
        terminal.wait_text(&["Sessions", &format!("/ {source_prefix}")], &[])?;
        terminal.send(b"\x06")?;
        terminal.wait_text(&["Settled boundaries"], &[])?;
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(
            &[
                "current project memory remains shared",
                "CLI-PARITY-ORIGIN",
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(requests.load(Ordering::SeqCst) == 0);
        let listed = sandbox.command("demo").arg("sessions").output()?;
        ensure!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        let listed: Value = serde_json::from_slice(&listed.stdout)?;
        let child = listed
            .as_array()
            .context("CLI session listing is not an array")?
            .iter()
            .find(|row| {
                row["id"] != cli_pending && row["fork_provenance"]["source_session_id"] == source
            })
            .context("PTY fork missing from CLI catalog")?;
        ensure!(child["fork_provenance"]["source_node_id"] == source_node);
        let child_id = child["id"]
            .as_str()
            .context("PTY child ID missing")?
            .to_owned();
        let export = sandbox.project.join("pty-session-export.md");
        terminal.command("/export pty-session-export.md", None)?;
        terminal.wait_composer_frame(&["Exported public session", "enter send"], READY_TIMEOUT)?;
        let exported = std::fs::read_to_string(&export)?;
        ensure!(exported.contains("CLI-PARITY-ORIGIN"));
        terminal.command(&format!("/resume {source}"), None)?;
        terminal.wait_composer_frame(&["CLI-PARITY-ORIGIN", "enter send"], READY_TIMEOUT)?;
        ensure!(requests.load(Ordering::SeqCst) == 0);
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
        ensure!(sandbox.sessions()?.iter().any(|row| row.id == child_id));

        // The first TUI's lifecycle mutations must be visible to a separately
        // opened TUI, not only to CLI reads made while that process was alive.
        let mut reopened_command = sandbox.command("responses");
        reopened_command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .arg("--resume")
            .arg(&child_id)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut reopened = Terminal::spawn(reopened_command, 48, 120)?;
        reopened.wait_composer_frame(
            &["CLI-PARITY-ORIGIN", "enter send"],
            sandbox.startup_timeout,
        )?;
        reopened.command("/sessions", Some("Sessions"))?;
        reopened.wait_text(&["TUI-LABEL", &child_id, "CLI-PENDING"], &[])?;
        let reopened_picker = reopened.screen();
        ensure!(
            reopened_picker.lines().any(|line| {
                line.contains(&source) && line.contains("active") && line.contains("TUI-LABEL")
            }),
            "fresh TUI lost the restored, renamed source: {reopened_picker}"
        );
        ensure!(
            reopened_picker
                .lines()
                .any(|line| { line.contains(&child_id) && line.contains("live fork") }),
            "fresh TUI lost the fork identity or provenance: {reopened_picker}"
        );
        ensure!(
            reopened_picker
                .lines()
                .any(|line| { line.contains(&cli_pending) && line.contains("pending") }),
            "fresh TUI lost the CLI pending boundary: {reopened_picker}"
        );
        reopened.close_picker(b"\x1b")?;
        reopened.send(b"/quit\r")?;
        reopened.wait_exit(EXIT_TIMEOUT)?;
        reopened.assert_restored()?;
        ensure!(
            requests.load(Ordering::SeqCst) == 0,
            "session lifecycle restart dispatched a provider request"
        );
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_lifecycle_hooks_rewrite_and_annotate_without_exposing_hook_output() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = Arc::clone(&requests);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move |Json(request): Json<Value>| {
                    let captured = Arc::clone(&captured);
                    async move {
                        captured.lock().unwrap().push(request);
                        priced_complete(Json(json!({}))).await
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let mut config = sandbox.config()?;
        config.api_base = format!("http://{}/v1", listener.local_addr()?);
        config.api_key_env = "KURU_FIXTURE_KEY".into();
        config.max_rounds = 1;
        let config_path = sandbox.root.path().join("lifecycle-hooks.toml");
        let nested_marker = sandbox.root.path().join("nested-tools-ran");
        config.hooks = LifecycleHooks {
            pre_turn: vec![HookCommand {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "set -e; cat >/dev/null; \"$1\" -C \"$2\" --data-dir \"$3\" --config \"$4\" --provider responses tools >/dev/null; printf x >> \"$5\"; printf '%s' '{\"decision\":\"rewrite\",\"value\":{\"input\":\"REWRITTEN_HOOK_INPUT\"}}'".into(),
                    "hook".into(),
                    env!("CARGO_BIN_EXE_kuru").into(),
                    sandbox.project.to_string_lossy().into_owned(),
                    sandbox.data.to_string_lossy().into_owned(),
                    config_path.to_string_lossy().into_owned(),
                    nested_marker.to_string_lossy().into_owned(),
                ],
                timeout_ms: 5_000,
                max_output_bytes: 64 * 1024,
            }],
            post_turn: vec![HookCommand {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "cat >/dev/null; printf '%s' '{\"decision\":\"annotate\",\"annotation\":\"HOOK_PRIVATE_ANNOTATION_SECRET\"}'"
                        .into(),
                ],
                timeout_ms: 5_000,
                max_output_bytes: 64 * 1024,
            }],
            ..LifecycleHooks::default()
        };
        std::fs::write(&config_path, toml::to_string(&config)?)?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));

        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config_path)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 48, 120)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.command("ORIGINAL_HOOK_INPUT", None)?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let first_count = requests.lock().unwrap().len();
        ensure!(
            first_count > 0,
            "the rewritten turn never reached the provider"
        );
        let first_requests = requests.lock().unwrap();
        ensure!(
            first_requests
                .iter()
                .any(|request| request.to_string().contains("REWRITTEN_HOOK_INPUT")),
            "the pre-turn rewrite was absent from provider input: {first_requests:?}"
        );
        ensure!(
            first_requests
                .iter()
                .all(|request| !request.to_string().contains("ORIGINAL_HOOK_INPUT")),
            "the durable original input leaked into rewritten provider input: {first_requests:?}"
        );
        drop(first_requests);
        ensure!(
            !terminal.screen().contains("HOOK_PRIVATE_ANNOTATION_SECRET"),
            "the private hook annotation was rendered: {}",
            terminal.screen()
        );

        terminal.command("SECOND_HOOK_INPUT", None)?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let later = requests.lock().unwrap();
        ensure!(
            later.len() > first_count
                && later[first_count..].iter().any(|request| request
                    .to_string()
                    .contains("HOOK_PRIVATE_ANNOTATION_SECRET")),
            "the settled post-turn annotation was absent from later actor context: {later:?}"
        );
        drop(later);
        let screen = terminal.screen();
        ensure!(screen.contains("ORIGINAL_HOOK_INPUT"), "{screen}");
        ensure!(screen.contains("SECOND_HOOK_INPUT"), "{screen}");
        ensure!(
            std::fs::read(&nested_marker)? == b"xx",
            "a nested `kuru tools` inspection reentered or skipped the two pre-turn hooks"
        );
        ensure!(
            !screen.contains("HOOK_PRIVATE_ANNOTATION_SECRET"),
            "{screen}"
        );
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_hook_refusals_and_speaker_stop_leave_the_session_usable() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = Arc::clone(&requests);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move |Json(request): Json<Value>| {
                    let captured = Arc::clone(&captured);
                    async move {
                        captured.lock().unwrap().push(request);
                        priced_complete(Json(json!({}))).await
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let mut config = sandbox.config()?;
        config.api_base = format!("http://{}/v1", listener.local_addr()?);
        config.api_key_env = "KURU_FIXTURE_KEY".into();
        config.max_rounds = 1;
        config.hooks = LifecycleHooks {
            pre_turn: vec![HookCommand {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "request=$(cat); case \"$request\" in *PTY_DENY*) printf '%s' '{\"decision\":\"deny\",\"reason\":\"PTY_DENIED\"}';; *PTY_MALFORMED*) printf '{';; *) printf '%s' '{\"decision\":\"allow\"}';; esac".into(),
                ],
                timeout_ms: 5_000,
                max_output_bytes: 64 * 1024,
            }],
            speaker_selected: vec![HookCommand {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "cat >/dev/null; if [ ! -e speaker-stopped ]; then : > speaker-stopped; printf '%s' '{\"decision\":\"stop\",\"reason\":\"PTY_STOPPED\"}'; else printf '%s' '{\"decision\":\"observe\"}'; fi".into(),
                ],
                timeout_ms: 5_000,
                max_output_bytes: 64 * 1024,
            }],
            ..LifecycleHooks::default()
        };
        let config_path = sandbox.root.path().join("hook-refusals.toml");
        std::fs::write(&config_path, toml::to_string(&config)?)?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));

        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config_path)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 48, 120)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;

        terminal.command("PTY_DENY", None)?;
        terminal.wait_composer_frame(&["PTY_DENIED", "enter send"], READY_TIMEOUT)?;
        ensure!(
            requests.lock().unwrap().is_empty(),
            "denied turn dispatched provider work"
        );

        terminal.command("PTY_MALFORMED", None)?;
        terminal.wait_composer_frame(&["hook", "enter send"], READY_TIMEOUT)?;
        ensure!(
            requests.lock().unwrap().is_empty(),
            "malformed hook dispatched provider work"
        );

        terminal.command("PTY_STOP", None)?;
        terminal.wait_composer_frame(&["PTY_STOPPED", "enter send"], READY_TIMEOUT)?;
        let stopped = requests.lock().unwrap().clone();
        ensure!(
            !stopped.is_empty(),
            "speaker stop did not reach deliberation"
        );
        ensure!(
            stopped
                .iter()
                .all(|request| !request.to_string().contains("Phase: speak")),
            "speaker stop dispatched speaking provider work: {stopped:?}"
        );
        ensure!(sandbox.project.join("speaker-stopped").exists());

        terminal.command("PTY_ALLOWED", None)?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let later = requests.lock().unwrap();
        ensure!(
            later.len() > stopped.len()
                && later[stopped.len()..]
                    .iter()
                    .any(|request| request.to_string().contains("Phase: speak")),
            "allowed continuation did not reach speaking dispatch: {later:?}"
        );
        drop(later);
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_started_kuru_run_reaches_provider_without_reentering_hooks() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let nested_project = sandbox.root.path().join("nested-project");
        std::fs::create_dir(&nested_project)?;
        let config_path = sandbox.root.path().join("nested-hooks.toml");
        let hook_marker = sandbox.root.path().join("hook-invocations");
        let nested_output = sandbox.root.path().join("nested-output.json");
        let nested_stderr = sandbox.root.path().join("nested-stderr.txt");
        let profile_destination = std::env::var_os("LLVM_PROFILE_FILE")
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("coverage destination is not UTF-8"))
            })
            .transpose()?
            .unwrap_or_default();
        if !profile_destination.is_empty() {
            ensure!(
                PathBuf::from(&profile_destination).is_absolute(),
                "coverage destination must be absolute"
            );
        }
        let mut config = sandbox.config()?;
        config.max_rounds = 1;
        config.dream_every = 0;
        config.dream_on_exit = false;
        config.hooks = LifecycleHooks {
            pre_turn: vec![HookCommand {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "set -e; cat >/dev/null; printf x >> \"$5\"; if [ \"$(wc -c < \"$5\")\" -eq 1 ]; then if [ -n \"$8\" ]; then export LLVM_PROFILE_FILE=\"$8\"; fi; \"$1\" -C \"$2\" --data-dir \"$3\" --config \"$4\" --provider demo --no-dream run 'nested hook boundary' --json > \"$6\" 2> \"$7\"; fi; printf '%s' '{\"decision\":\"allow\"}'".into(),
                    "hook".into(),
                    env!("CARGO_BIN_EXE_kuru").into(),
                    nested_project.to_string_lossy().into_owned(),
                    sandbox.data.to_string_lossy().into_owned(),
                    config_path.to_string_lossy().into_owned(),
                    hook_marker.to_string_lossy().into_owned(),
                    nested_output.to_string_lossy().into_owned(),
                    nested_stderr.to_string_lossy().into_owned(),
                    profile_destination,
                ],
                timeout_ms: 15_000,
                max_output_bytes: 64 * 1024,
            }],
            ..LifecycleHooks::default()
        };
        std::fs::write(&config_path, toml::to_string(&config)?)?;

        let started = Instant::now();
        let output = sandbox
            .command("demo")
            .arg("--config")
            .arg(&config_path)
            .args(["run", "outer hook boundary", "--json"])
            .output()?;
        if !output.status.success() {
            let nested_error = std::fs::File::open(&nested_stderr)
                .and_then(|file| {
                    let mut bytes = Vec::new();
                    file.take(4096).read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
                .unwrap_or_default();
            anyhow::bail!(
                "outer run failed after {:?}: {}; nested stderr (4 KiB max): {}",
                started.elapsed(),
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&nested_error)
            );
        }
        let outer: Value = serde_json::from_slice(&output.stdout)?;
        let nested: Value = serde_json::from_slice(&std::fs::read(&nested_output)?)?;
        ensure!(
            outer["text"].as_str().is_some_and(|text| !text.is_empty())
                && nested["text"].as_str().is_some_and(|text| !text.is_empty()),
            "a run ended before the outer or nested provider lifecycle"
        );
        ensure!(
            std::fs::read(&hook_marker)? == b"x",
            "nested Kuru reentered the originating hook chain"
        );
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_custom_command_uses_reviewed_catalog_and_literal_arguments() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let custom = sandbox.project.join(".kuru/commands/review.md");
        std::fs::create_dir_all(custom.parent().context("command parent")?)?;
        std::fs::write(
            &custom,
            "---\nname: review\ndescription: Review this change\n---\nCUSTOM-PROMPT-SENTINEL\n",
        )?;
        let unapproved = sandbox
            .command("demo")
            .args(["run", "before review"])
            .output()?;
        ensure!(
            !unapproved.status.success()
                && String::from_utf8_lossy(&unapproved.stderr)
                    .contains("workspace authority is not approved"),
            "unapproved project prompt command did not stop at trust preflight"
        );
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = Arc::clone(&requests);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move |Json(request): Json<Value>| {
                    let captured = Arc::clone(&captured);
                    async move {
                        captured.lock().unwrap().push(request);
                        priced_complete(Json(json!({}))).await
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("custom-command-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--trust-workspace-once", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 48, 120)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.command("/help", None)?;
        terminal.wait_composer_frame(
            &["/review [arguments]", "Review this change"],
            READY_TIMEOUT,
        )?;
        let before = requests.lock().unwrap().len();
        std::fs::write(
            &custom,
            "---\nname: review\ndescription: Changed after startup\n---\nNEW-PROMPT-SENTINEL\n",
        )?;
        terminal.command("/not-registered", None)?;
        terminal.wait_composer_frame(&["Unknown command", "enter send"], READY_TIMEOUT)?;
        ensure!(
            requests.lock().unwrap().len() == before,
            "unknown command reached provider"
        );
        terminal.send(b"/rev\t")?;
        terminal.wait_composer_frame(&["/review", "enter send"], READY_TIMEOUT)?;
        terminal.send(b" literal $HOME $(echo no-execution)\r")?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let seen = requests.lock().unwrap();
        ensure!(
            seen[before..].iter().any(|request| {
                let wire = request.to_string();
                wire.contains("CUSTOM-PROMPT-SENTINEL")
                    && wire.contains("literal $HOME $(echo no-execution)")
                    && !wire.contains("NEW-PROMPT-SENTINEL")
            }),
            "custom prompt and literal arguments were absent from the provider request"
        );
        drop(seen);
        let sessions = sandbox.sessions()?;
        ensure!(
            sessions.len() == 1 && sessions[0].turns == 1,
            "{sessions:?}"
        );
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[derive(Clone)]
struct SkillSelectionProvider {
    seen: Arc<Mutex<Vec<String>>>,
}

async fn skill_selection_complete(
    State(state): State<SkillSelectionProvider>,
    Json(request): Json<Value>,
) -> Response {
    let instructions = request["instructions"].as_str().unwrap_or_default();
    let speaking = instructions.contains("Phase: speak and act");
    let continued = request["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    if speaking {
        state.seen.lock().unwrap().push(instructions.to_owned());
    }
    let output = if speaking && !continued {
        json!([{"type":"function_call","call_id":"select-review", "name":"skill_load",
            "arguments":json!({"name":"review"}).to_string()}])
    } else {
        let answer = if speaking && instructions.contains("SELECTED-SKILL-BODY") {
            "SKILL_SELECTION_FINAL"
        } else {
            "SKILL_SELECTION_WAITING"
        };
        json!([{"type":"message","content":[{"type":"output_text","text":answer}]}])
    };
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{"id":"skill-selection","status":"completed","output":output,
                    "usage":{"input_tokens":8,"output_tokens":5}}
            })
        ),
    )
        .into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_skill_selection_reviews_body_before_provider_continuation() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let skill = sandbox.project.join(".agents/skills/review/SKILL.md");
        std::fs::create_dir_all(skill.parent().context("skill parent")?)?;
        std::fs::write(
            &skill,
            "---\nname: review\ndescription: Review code carefully\nallowed-tools: shell\n---\nSELECTED-SKILL-BODY\n",
        )?;
        let approval = sandbox
            .command("demo")
            .args(["trust", "approve", "--yes"])
            .output()?;
        ensure!(
            approval.status.success(),
            "base workspace approval failed: {}",
            String::from_utf8_lossy(&approval.stderr)
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(skill_selection_complete))
            .with_state(SkillSelectionProvider { seen: seen.clone() });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("skill-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--mode", "freudian", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"Select the review skill\r")?;
        terminal.wait_composer_frame(
            &[
                "Workspace instruction review",
                "selected project skill material",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(
            seen.lock().unwrap().len() == 1,
            "provider continued before skill review"
        );
        terminal.send(b"1")?;
        terminal.wait_text(
            &["SKILL_SELECTION_FINAL"],
            &["Workspace instruction review"],
        )?;
        let requests = seen.lock().unwrap();
        ensure!(
            requests.len() >= 2,
            "skill selection did not reach a new provider request"
        );
        ensure!(requests[0].contains("Review code carefully"));
        ensure!(!requests[0].contains("SELECTED-SKILL-BODY"));
        ensure!(requests[1].contains("SELECTED-SKILL-BODY"));
        ensure!(!requests[1].contains("allowed-tools"));
        drop(requests);
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

/// T3's coherent surface: one completed frame carries model, effort, mode,
/// cost, context use and permission state at once, and the cost and permission
/// figures are the same ones `/cost` and `/permissions` print.
#[test]
fn real_pty_status_bar_holds_all_six_session_facts_at_80_and_120_columns() -> Result<()> {
    let sandbox = Sandbox::new()?;
    let mut terminal = Terminal::spawn(sandbox.command("demo"), 35, 120)?;
    terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
    terminal.command("hello", None)?;
    terminal.wait_composer_frame(&["[demo]", "enter send"], READY_TIMEOUT)?;
    for (rows, cols) in [(35, 120), (24, 80)] {
        terminal.resize(rows, cols)?;
        terminal.command("/cost", None)?;
        terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
        let screen = terminal.screen();
        let standing = dock(&screen);
        for (element, token) in [
            ("model", "demo"),
            ("effort", "default"),
            ("mode", "IFS"),
            ("context use", "ctx ≈"),
            ("cost", "cost unknown"),
            ("permission state", "Permissions · 0 session · 0 always"),
        ] {
            ensure!(
                standing.contains(token),
                "{element} is missing from the standing dock at {cols}x{rows}: {screen}"
            );
        }
        // An unknown price is never a zero charge and never a quota figure.
        ensure!(!standing.contains('$'), "{screen}");
        // The bar's context bound repeats the prepared request's own numbers.
        ensure!(standing.contains("assumed"), "{screen}");
        // The bar's cost token says exactly what /cost says in this same frame.
        ensure!(
            screen.contains("API-standard estimate: unknown")
                && screen.contains("charge): unknown"),
            "status bar cost must agree with /cost at {cols}x{rows}: {screen}"
        );

        // /permissions reports the same grants the dock counts.
        terminal.command("/permissions", None)?;
        terminal.wait_composer_frame(&["Permissions · ↑↓ select"], READY_TIMEOUT)?;
        let screen = terminal.screen();
        ensure!(screen.contains("No session or always grants."), "{screen}");
        ensure!(
            dock(&screen).contains("Permissions · 0 session · 0 always"),
            "the dock must keep agreeing with /permissions at {cols}x{rows}: {screen}"
        );
        terminal.send(b"\x1b")?;
        terminal.wait("permission inspector closes", READY_TIMEOUT, |terminal| {
            Ok(!terminal.screen().contains("Permissions · ↑↓ select"))
        })?;
    }
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

async fn priced_complete(Json(_): Json<Value>) -> Response {
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{
                    "id":"priced-fixture",
                    "status":"completed",
                    "output":[{"type":"message","content":[{
                        "type":"output_text", "text":"PRICED_RESPONSE_MARKER"
                    }]}],
                    "usage":{
                        "input_tokens": PRICED_INPUT_TOKENS_PER_INVOCATION,
                        "output_tokens": PRICED_OUTPUT_TOKENS_PER_INVOCATION,
                        "input_tokens_details":{"cached_tokens": 0},
                        "output_tokens_details":{"reasoning_tokens": 0}
                    }
                }
            })
        ),
    )
        .into_response()
}

// The harness dispatches more than one provider invocation per user turn
// (speaker selection, per-part contribution, and so on); the exact count is
// an internal runtime detail this test does not pin down. Every invocation
// this mock answers reports the same per-invocation counts below, so the
// expected dollar figure is derived from the *cumulative* totals `/cost`
// itself reports, not from an assumed invocation count.
const PRICED_INPUT_TOKENS_PER_INVOCATION: u64 = 8;
const PRICED_OUTPUT_TOKENS_PER_INVOCATION: u64 = 5;
const PRICED_INPUT_RATE_PER_MILLION_USD: f64 = 1.00;
const PRICED_OUTPUT_RATE_PER_MILLION_USD: f64 = 2.00;

/// Reads the token count out of a `/cost` component line such as
/// `"Input: 64 tokens"`, so the test derives its expected dollar figure from
/// the session's actual reported totals instead of guessing how many
/// provider invocations one turn dispatches.
fn parse_component_tokens(screen: &str, label: &str) -> Result<u64> {
    let marker = format!("{label}: ");
    let start = screen
        .find(&marker)
        .with_context(|| format!("{label:?} component missing from screen: {screen}"))?
        + marker.len();
    let rest = &screen[start..];
    let end = rest
        .find(" tokens")
        .with_context(|| format!("{label:?} token count unparsable: {screen}"))?;
    rest[..end]
        .trim()
        .parse::<u64>()
        .with_context(|| format!("{label:?} token count unparsable: {screen}"))
}

/// T3's residual known-cost branch: `apps/kuru-tui/tests/fixtures/priced-model-catalog.json`
/// (loaded through the `test-support`-gated `KURU_TEST_MODEL_CATALOG_PATH`
/// seam in `kuru-core`'s model catalog) gives the mock `responses` provider's
/// `fixture` model a real price, so a turn against it exercises
/// `dock_meters`'s known-cost formatting arms end to end instead of only the
/// "cost unknown" branch every other real-PTY fixture is stuck on. The
/// expected dollar figure is derived from the same rate/usage inputs the
/// ledger fold consumes (`EstimateFold::add` in
/// `kuru-memory/src/store/usage_ledger.rs`), never asserted independently of
/// it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_status_bar_renders_known_cost_from_priced_invocation() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(priced_complete));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("priced-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let catalog_override =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/priced-model-catalog.json");
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1")
            .env("KURU_TEST_MODEL_CATALOG_PATH", &catalog_override);
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"Priced fixture turn\r")?;
        terminal.wait_composer_frame(&["PRICED_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;

        terminal.command("/cost", None)?;
        terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
        let cost_screen = terminal.screen();
        let total_input = parse_component_tokens(&cost_screen, "Input")?;
        let total_output = parse_component_tokens(&cost_screen, "Output")?;
        // Every invocation reported a real (zero) cached-input and
        // reasoning-output count, so the fold left no term unapplied: the
        // estimate must be a complete "≈" figure, never a "≥" subtotal.
        ensure!(
            cost_screen.contains("Cached input (subset): 0 tokens")
                && !cost_screen.contains("Reasoning output (subset): unknown"),
            "fixture usage must report every component so the estimate is complete: {cost_screen}"
        );
        let expected_usd = total_input as f64 * PRICED_INPUT_RATE_PER_MILLION_USD / 1_000_000.0
            + total_output as f64 * PRICED_OUTPUT_RATE_PER_MILLION_USD / 1_000_000.0;
        let expected_known_usd = format!("{expected_usd:.6}");
        let expected_dock_token = format!("≈${expected_known_usd} est");
        let expected_cost_line = format!("API-standard estimate: ${expected_known_usd} estimated");
        ensure!(
            cost_screen.contains(&expected_cost_line),
            "/cost must state the derived known figure {expected_cost_line:?}: {cost_screen}"
        );

        for (rows, cols) in [(35, 120), (24, 80)] {
            terminal.resize(rows, cols)?;
            terminal.command("/cost", None)?;
            terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
            let screen = terminal.screen();
            let standing = dock(&screen);
            ensure!(
                standing.contains(&expected_dock_token),
                "standing dock must carry the known-cost token {expected_dock_token:?} at {cols}x{rows}: {screen}"
            );
            ensure!(
                screen.contains(&expected_cost_line),
                "/cost must state the same known figure the dock renders at {cols}x{rows}: {screen}"
            );
        }
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumed_pty_reports_automatic_compaction_cost_once_and_reuses_its_summary() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = Arc::clone(&requests);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move |Json(request): Json<Value>| {
                    let captured = Arc::clone(&captured);
                    async move {
                        let compact = request["instructions"]
                            .as_str()
                            .is_some_and(|instructions| {
                                instructions.starts_with("Replace the prior rolling context summary")
                            });
                        captured.lock().unwrap().push(request);
                        (
                            [(CONTENT_TYPE, "text/event-stream")],
                            format!(
                                "data: {}\n\n",
                                json!({
                                    "type":"response.completed",
                                    "response":{
                                        "id":"resumed-compaction-fixture",
                                        "status":"completed",
                                        "output":[{"type":"message","content":[{
                                            "type":"output_text",
                                            "text": if compact {
                                                "AUTO_COMPACT_SUMMARY_SENTINEL"
                                            } else {
                                                "RESUMED_ORDINARY_RESPONSE"
                                            }
                                        }]}],
                                        "usage":{
                                            "input_tokens": PRICED_INPUT_TOKENS_PER_INVOCATION,
                                            "output_tokens": PRICED_OUTPUT_TOKENS_PER_INVOCATION,
                                            "input_tokens_details":{"cached_tokens": 0},
                                            "output_tokens_details":{"reasoning_tokens": 0}
                                        }
                                    }
                                })
                            ),
                        )
                            .into_response()
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("resumed-compaction-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\nassumed_context_window_tokens=8192\ncontext_output_reserve_tokens=64\ncontext_compaction_threshold_percent=50\ncontext_compaction_output_reserve_tokens=64\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let catalog_override =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/priced-model-catalog.json");
        let command = || {
            let mut command = sandbox.command("responses");
            command
                .args(["--model", "fixture", "--config"])
                .arg(&config)
                .env("KURU_FIXTURE_KEY", "fixture")
                .env("KURU_REDUCED_MOTION", "1")
                .env("KURU_TEST_MODEL_CATALOG_PATH", &catalog_override);
            command
        };
        let mut terminal = Terminal::spawn(command(), 48, 120)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.command("automatic compaction source", None)?;
        terminal.wait_composer_frame(&["RESUMED_ORDINARY_RESPONSE", "enter send"], READY_TIMEOUT)?;
        terminal.command("automatic compaction trigger", None)?;
        terminal.wait_composer_frame(
            &["RESUMED_ORDINARY_RESPONSE", "Compacted", "source sequences", "enter send"],
            READY_TIMEOUT,
        )?;
        ensure!(
            terminal.screen().contains("original")
                && terminal.screen().contains("records remain stored"),
            "accepted compaction metadata was not visible: {}",
            terminal.screen()
        );
        terminal.resize(48, 80)?;
        terminal.wait_composer_frame(
            &["Compacted", "source sequences", "original", "remain stored", "enter send"],
            READY_TIMEOUT,
        )?;
        ensure!(
            !String::from_utf8_lossy(&terminal.output).contains("AUTO_COMPACT_SUMMARY_SENTINEL"),
            "automatic notice disclosed the private summary"
        );
        terminal.resize(48, 120)?;
        terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
        let sessions = sandbox.sessions()?;
        ensure!(
            sessions.len() == 1 && sessions[0].turns == 2,
            "{sessions:?}"
        );
        let session = sessions[0].id.clone();
        let before_resume = requests.lock().unwrap().clone();
        let compact_requests = before_resume
            .iter()
            .filter(|request| {
                request["instructions"]
                    .as_str()
                    .is_some_and(|instructions| {
                        instructions.starts_with("Replace the prior rolling context summary")
                    })
            })
            .count();
        ensure!(compact_requests > 0, "automatic compaction did not run");
        terminal.command("/cost", None)?;
        terminal.wait_composer_frame(&["Session usage", "enter send"], READY_TIMEOUT)?;
        let first_cost = terminal.screen();
        ensure!(
            parse_component_tokens(&first_cost, "Input")?
                == before_resume.len() as u64 * PRICED_INPUT_TOKENS_PER_INVOCATION,
            "automatic compaction input usage was not counted exactly once: {first_cost}"
        );
        ensure!(
            parse_component_tokens(&first_cost, "Output")?
                == before_resume.len() as u64 * PRICED_OUTPUT_TOKENS_PER_INVOCATION,
            "automatic compaction output usage was not counted exactly once: {first_cost}"
        );
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;

        let mut resume = command();
        resume.args(["--resume", &session]);
        let mut resumed = Terminal::spawn(resume, 48, 120)?;
        resumed.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        resumed.command("/status", None)?;
        resumed.wait_composer_frame(
            &[
                &format!("Session: {session}"),
                "Turns: 2",
                &format!(
                    "Session usage · {} provider invocations",
                    before_resume.len()
                ),
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        resumed.command("/cost", None)?;
        resumed.wait_composer_frame(&["Session usage", "enter send"], READY_TIMEOUT)?;
        let resumed_cost = resumed.screen();
        ensure!(
            parse_component_tokens(&resumed_cost, "Input")?
                == before_resume.len() as u64 * PRICED_INPUT_TOKENS_PER_INVOCATION
                && parse_component_tokens(&resumed_cost, "Output")?
                    == before_resume.len() as u64 * PRICED_OUTPUT_TOKENS_PER_INVOCATION,
            "resumed usage duplicated or lost automatic compaction cost: {resumed_cost}"
        );
        ensure!(
            requests.lock().unwrap().len() == before_resume.len(),
            "resume/status/cost made a provider request"
        );

        resumed.command("resumed context uses the summary", None)?;
        resumed.wait_composer_frame(&["RESUMED_ORDINARY_RESPONSE", "enter send"], READY_TIMEOUT)?;
        let after_resume = requests.lock().unwrap();
        ensure!(
            after_resume[before_resume.len()..].iter().any(|request| {
                !request["instructions"]
                    .as_str()
                    .is_some_and(|instructions| {
                        instructions.starts_with("Replace the prior rolling context summary")
                    })
                    && request
                        .to_string()
                        .contains("AUTO_COMPACT_SUMMARY_SENTINEL")
            }),
            "resumed ordinary context omitted the persisted rolling summary"
        );
        drop(after_resume);
        resumed.send(b"/quit\r")?;
        resumed.wait_exit(EXIT_TIMEOUT)?;
        resumed.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_compaction_notices_keep_streams_clean_after_success_and_sigint() -> Result<()> {
    kuru_memory::test_support::closing(async {
        use kuru_platform::unix::{OwnedProcessGroup, Reap, RootState, StdioPlan, StdioSlot};
        use nix::{sys::signal::{Signal, kill}, unistd::Pid};

        let sandbox = Sandbox::warmed().await?;
        let compact_seen = Arc::new(AtomicBool::new(false));
        let hold_answer = Arc::new(AtomicBool::new(false));
        let ordinary_started = Arc::new(tokio::sync::Notify::new());
        let release_answer = Arc::new(tokio::sync::Notify::new());
        let compact_count = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/v1/models", get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }))
            .route("/v1/responses", post({
                let compact_seen = compact_seen.clone();
                let hold_answer = hold_answer.clone();
                let ordinary_started = ordinary_started.clone();
                let release_answer = release_answer.clone();
                let compact_count = compact_count.clone();
                move |Json(request): Json<Value>| {
                    let compact_seen = compact_seen.clone();
                    let hold_answer = hold_answer.clone();
                    let ordinary_started = ordinary_started.clone();
                    let release_answer = release_answer.clone();
                    let compact_count = compact_count.clone();
                    async move {
                        let compact = request["instructions"].as_str().is_some_and(|value| {
                            value.starts_with("Replace the prior rolling context summary")
                        });
                        if compact {
                            compact_count.fetch_add(1, Ordering::SeqCst);
                            compact_seen.store(true, Ordering::SeqCst);
                        } else if hold_answer.load(Ordering::SeqCst) && compact_seen.load(Ordering::SeqCst) {
                            // This ordinary request follows the actor's accepted
                            // compaction checkpoint, never merely its summary reply.
                            ordinary_started.notify_one();
                            release_answer.notified().await;
                        }
                        ([(CONTENT_TYPE, "text/event-stream")], format!("data: {}\n\n", json!({
                            "type":"response.completed",
                            "response": {"id":"headless-compaction", "status":"completed",
                                "output":[{"type":"message","content":[{"type":"output_text",
                                    "text": if compact { "HEADLESS_PRIVATE_SUMMARY_SENTINEL" } else { "HEADLESS_ANSWER" }
                                }]}],
                                "usage":{"input_tokens":64,"output_tokens":40}
                            }
                        }))).into_response()
                    }
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("headless-compaction.toml");
        std::fs::write(&config, format!(
            "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\nassumed_context_window_tokens=8192\ncontext_output_reserve_tokens=64\ncontext_compaction_threshold_percent=50\ncontext_compaction_output_reserve_tokens=64\n",
            listener.local_addr()?
        ))?;
        let _server = Server(tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }));
        for (label, json_output, interrupt) in [("answer", false, false), ("json", true, false), ("interrupt", true, true)] {
            compact_seen.store(false, Ordering::SeqCst);
            hold_answer.store(interrupt, Ordering::SeqCst);
            let prior_compactions = compact_count.load(Ordering::SeqCst);
            let mut command = sandbox.command("responses");
            command.args(["--model", "fixture", "--config"]).arg(&config)
                .env("KURU_FIXTURE_KEY", "fixture").args(["run", label]);
            if json_output { command.arg("--json"); }
            let pid_file = sandbox.root.path().join(format!("{label}.pid"));
            let mut launch = Command::new("/bin/sh");
            // Only this fixed builtin writes the private file. exec retains
            // the exact owned root; no legacy concurrent child is spawned.
            launch.args(["-c", "printf '%s' \"$$\" > \"$1\"; shift; exec \"$@\"", "kuru-headless-fixture"])
                .arg(&pid_file).arg(command.get_program()).args(command.get_args());
            if let Some(directory) = command.get_current_dir() { launch.current_dir(directory); }
            for (key, value) in command.get_envs() {
                if let Some(value) = value { launch.env(key, value); } else { launch.env_remove(key); }
            }
            let mut child = OwnedProcessGroup::spawn(launch, StdioPlan::new(StdioSlot::Null, StdioSlot::Pipe, StdioSlot::Pipe))?;
            let capture = |mut pipe: Box<dyn Read + Send>| {
                let (sender, receiver) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let mut bytes = vec![];
                    let result = pipe.by_ref().take(1024 * 1024 + 1).read_to_end(&mut bytes).map(|_| bytes);
                    let _ = sender.send(result);
                });
                receiver
            };
            let stdout = capture(Box::new(child.take_stdout()?));
            let stderr = capture(Box::new(child.take_stderr()?));
            let outcome = async {
                if interrupt {
                    tokio::time::timeout(sandbox.startup_timeout + READY_TIMEOUT, ordinary_started.notified()).await
                        .context("ordinary request did not follow an accepted checkpoint")?;
                    let mut options = memory_options(&sandbox)?;
                    options.read_only = true;
                    let inspector = MemoryStore::open_managed_observed(options, sandbox.project.canonicalize()?, PathBuf::from(env!("CARGO_BIN_EXE_kuru"))).1.await?;
                    let session = kuru_runtime::Harness::continuation_session(&inspector, &sandbox.project).await?;
                    let scope = kuru_runtime::project_scope(&sandbox.project)?;
                    let membership = inspector.get(&format!("{scope}/ifs/membership")).await?.context("membership absent")?;
                    let parts: Vec<kuru_core::Part> = serde_json::from_value(membership["parts"].clone())?;
                    let profile = ModeProfile::builtin(Mode::Ifs);
                    let mut confirmed = false;
                    for part in parts {
                        let namespace = profile.memory.identity_namespace(&scope, Mode::Ifs, &part.id);
                        if let Some(cursor) = inspector.context_summary_cursor(&namespace, &session, &namespace).await? {
                            ensure!(inspector.context_summary_confirmation(&cursor.summary_id).await?.is_some());
                            confirmed = true;
                        }
                    }
                    inspector.close().await?;
                    ensure!(confirmed, "signal preceded any accepted summary checkpoint");
                    let pid: i32 = std::fs::read_to_string(&pid_file)?.parse()?;
                    ensure!(pid > 1);
                    // The sole owner remains borrowed and unreaped across this
                    // non-reaping observation and root-only signal; no async
                    // waiter or numeric identity outlives this retained child.
                    ensure!(matches!(child.root_state(), RootState::Running), "headless root was not running before SIGINT");
                    kill(Pid::from_raw(pid), Signal::SIGINT)?;
                }
                let deadline = Instant::now() + sandbox.startup_timeout + EXIT_TIMEOUT;
                loop {
                    match child.root_state() {
                        RootState::Exited => break,
                        RootState::Running | RootState::Interrupted => {},
                        state => anyhow::bail!("headless root lost its owned state: {state:?}"),
                    }
                    ensure!(Instant::now() < deadline, "headless root did not settle");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok::<(), anyhow::Error>(())
            }.await;
            // Cleanup always runs, including a failed causal/stream assertion.
            child.terminate_before_reap();
            child.wait_pre_reap(Duration::from_millis(10), Instant::now() + EXIT_TIMEOUT).await;
            let status = child.reap_if_exited();
            release_answer.notify_waiters();
            let stdout = stdout.recv_timeout(IO_TIMEOUT).context("headless stdout did not drain")??;
            let stderr = stderr.recv_timeout(IO_TIMEOUT).context("headless stderr did not drain")??;
            outcome?;
            let Reap::Reaped(status) = status else { anyhow::bail!("headless child was not reaped: {status:?}"); };
            ensure!(status.success() != interrupt, "{label}: {status:?}: {}", String::from_utf8_lossy(&stderr));
            ensure!(stdout.len() <= 1024 * 1024 && stderr.len() <= 1024 * 1024);
            let stderr_text = String::from_utf8_lossy(&stderr);
            ensure!(compact_count.load(Ordering::SeqCst) > prior_compactions, "{label}: automatic compaction did not run");
            ensure!(stderr_text.contains("Compacted") && stderr_text.contains("original records remain stored"), "{label}: {stderr_text}");
            ensure!(!stderr_text.contains("HEADLESS_PRIVATE_SUMMARY_SENTINEL"));
            ensure!(!String::from_utf8_lossy(&stdout).contains("HEADLESS_PRIVATE_SUMMARY_SENTINEL"));
            if interrupt {
                ensure!(stdout.is_empty(), "interrupted answer contaminated stdout: {}", String::from_utf8_lossy(&stdout));
            } else if json_output {
                let output: Value = serde_json::from_slice(&stdout)?;
                ensure!(output["text"] == "HEADLESS_ANSWER");
            } else {
                ensure!(stdout == b"HEADLESS_ANSWER\n");
            }
        }
        // The same acknowledged checkpoint must survive interactive
        // cancellation, not only the headless stderr settlement above.
        compact_seen.store(false, Ordering::SeqCst);
        hold_answer.store(true, Ordering::SeqCst);
        let mut command = sandbox.command("responses");
        command.args(["--model", "fixture", "--config"]).arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture").env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 48, 80)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.submit("accepted interactive compaction cancellation")?;
        tokio::time::timeout(sandbox.startup_timeout + READY_TIMEOUT, ordinary_started.notified()).await
            .context("interactive ordinary request did not follow accepted compaction")?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(
            &["Compacted", "source sequences", "remain stored", "Cancelled", "enter send"],
            READY_TIMEOUT,
        )?;
        ensure!(
            !String::from_utf8_lossy(&terminal.output).contains("HEADLESS_PRIVATE_SUMMARY_SENTINEL"),
            "interactive cancellation disclosed the private summary"
        );
        release_answer.notify_waiters();
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
        Ok(())
    }).await
}

#[test]
fn real_pty_reports_actual_optional_context_omission_without_erasing_history() -> Result<()> {
    let sandbox = Sandbox::new()?;
    let sentinel = "older-context-remains-stored";
    let long_prompt = format!("{sentinel} {}", "x".repeat(12_000));
    let first = sandbox
        .command("demo")
        .args(["run", &long_prompt, "--json"])
        .output()?;
    ensure!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: Value = serde_json::from_slice(&first.stdout)?;
    let session = first["session"]
        .as_str()
        .context("first run has no session")?;
    let config_path = sandbox.root.path().join("config/kuru/config.toml");
    let config = format!(
        "assumed_context_window_tokens = 4000\ncontext_output_reserve_tokens = 256\n{}",
        std::fs::read_to_string(&config_path)?
    );
    std::fs::write(config_path, config)?;

    let mut command = sandbox.command("demo");
    command.args(["--resume", session]);
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
    terminal.command("short follow-up", None)?;
    terminal.wait_composer_frame(&["omitted", "enter send"], READY_TIMEOUT)?;
    let screen = terminal.screen();
    ensure!(screen.contains("older public"), "{screen}");
    ensure!(screen.contains("stored history is unchanged"), "{screen}");
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

/// The offset of the last `needle` in `haystack`.
fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    rfind_bytes(haystack, needle).is_some()
}

/// The terminal bytes written before the interface takes the screen.
fn primary_screen_bytes(output: &[u8]) -> Result<&[u8]> {
    let alternate = output
        .windows(b"\x1b[?1049h".len())
        .position(|bytes| bytes == b"\x1b[?1049h")
        .context("terminal did not enter its alternate screen")?;
    Ok(&output[..alternate])
}

/// Whether `directory` lists an entry whose name satisfies `matches`. Only
/// the directory itself is listed and none of its entries is entered: an open
/// renames or removes the stages and build stores this looks for.
fn lists(directory: &std::path::Path, matches: impl Fn(&str) -> bool) -> Result<bool> {
    let entries = match std::fs::read_dir(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        entries => entries?,
    };
    for entry in entries {
        if matches(&entry?.file_name().to_string_lossy()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The store template root of an engine cache.
fn template_root(cache: &std::path::Path) -> PathBuf {
    cache
        .join(kuru_memory::provision::DOLT_VERSION)
        .join("templates")
}

/// Fails unless the creating sentence was written and no other sentence was
/// written after it: the creating sentence stays up for the rest of the open.
fn assert_creating_kept(output: &[u8]) -> Result<()> {
    use kuru::memory_activity::{CREATING, SENTENCES};
    let creating = output
        .windows(CREATING.len())
        .position(|bytes| bytes == CREATING.as_bytes())
        .context("the creating sentence was never written")?;
    for sentence in SENTENCES.iter().filter(|sentence| **sentence != CREATING) {
        ensure!(
            !contains_bytes(&output[creating..], sentence.as_bytes()),
            "{sentence:?} replaced the creating sentence"
        );
    }
    Ok(())
}

/// After the last sentence the line is padded, then erased with `\r`, spaces
/// and `\r`; no sentence bytes follow that erase. Other bytes, such as the
/// first-run notice, may follow it.
fn assert_sentence_erased(startup: &[u8]) -> Result<()> {
    let end = kuru::memory_activity::SENTENCES
        .iter()
        .filter_map(|sentence| {
            rfind_bytes(startup, sentence.as_bytes()).map(|at| at + sentence.len())
        })
        .max()
        .context("memory startup wrote no sentence")?;
    let after = &startup[end..];
    let after = &after[after.iter().take_while(|byte| **byte == b' ').count()..];
    let blanks = after
        .strip_prefix(b"\r")
        .map(|rest| rest.iter().take_while(|byte| **byte == b' ').count());
    ensure!(
        blanks.is_some_and(|blanks| blanks > 0 && after[1 + blanks..].starts_with(b"\r")),
        "the last sentence was not erased: {:?}",
        String::from_utf8_lossy(&after[..after.len().min(80)])
    );
    Ok(())
}

/// Each run of the interface: the first creates the project and holds its
/// owner at creation until the creating sentence is on the terminal; a later
/// one reopens it after the previous owner exited.
fn smoke(sandbox: &Sandbox, reduced: bool, full: bool, expect_notice: bool) -> Result<()> {
    use kuru::memory_activity::{CREATING, OPENING, SENTENCES};
    let holds = sandbox.root.path().join("holds");
    let hold = holds.join("CreatingDatabase.hold");
    let mut command = sandbox.command("demo");
    command.args(["--mode", "freudian"]);
    if reduced {
        command.env("KURU_REDUCED_MOTION", "1");
    }
    if expect_notice {
        std::fs::create_dir(&holds)?;
        // The marker carries the budget of the wait that ends in its removal,
        // the creating-sentence wait below (kuru-memory `OPEN_HOLD_DIR_ENV`).
        std::fs::write(&hold, sandbox.startup_timeout.as_millis().to_string())?;
        command.env(kuru_memory::test_support::OPEN_HOLD_DIR_ENV, &holds);
    } else {
        // A reopen that landed while the previous owner still closed would
        // show the waiting sentence instead of only the opening one.
        memory::await_owner_exit(&memory_options(sandbox)?)?;
    }
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    if expect_notice {
        terminal.wait(
            "the creating sentence on the terminal",
            sandbox.startup_timeout,
            |terminal| Ok(contains_bytes(&terminal.output, CREATING.as_bytes())),
        )?;
        std::fs::remove_file(&hold)?;
        // The shared cache holds a warm store template, so the new project
        // is copied from it into a stage that its first engine adopts: the
        // creating sentence stays up through the copy and the adoption.
        let memory = sandbox.data.join("memory");
        terminal.wait(
            "the new project's stage holding its template copy",
            sandbox.startup_timeout,
            |_| lists(&memory, |name| name.contains(".staging-")),
        )?;
        ensure!(
            terminal.screen().contains(CREATING),
            "the creating sentence is not on the screen during the copy"
        );
        assert_creating_kept(&terminal.output)?;
    }
    let expected = if expect_notice {
        vec!["KURU", "enter send", "Memory is ready at"]
    } else {
        vec!["KURU", "enter send"]
    };
    terminal.wait_text_with_timeout(
        &expected,
        if expect_notice {
            &[]
        } else {
            &["Memory is ready at"]
        },
        sandbox.startup_timeout,
    )?;
    terminal.wait_composer_frame(&expected, READY_TIMEOUT)?;
    let startup = primary_screen_bytes(&terminal.output)?;
    ensure!(
        contains_bytes(startup, OPENING.as_bytes()),
        "memory startup did not show its opening sentence before the first completed TUI frame"
    );
    ensure!(
        !contains_bytes(startup, b"Memory:"),
        "memory startup still wrote a labelled line"
    );
    ensure!(
        contains_bytes(startup, CREATING.as_bytes()) == expect_notice,
        "the creating sentence belongs to the first run only"
    );
    if expect_notice {
        assert_creating_kept(startup)?;
    } else {
        // A reopen after the previous owner exited shows nothing else.
        for sentence in SENTENCES.iter().filter(|sentence| **sentence != OPENING) {
            ensure!(
                !contains_bytes(startup, sentence.as_bytes()),
                "a reopen showed {sentence:?}"
            );
        }
    }
    assert_sentence_erased(startup)?;
    assert!(
        terminal
            .output
            .windows(7)
            .any(|value| value == b"\x1b[38;2;")
    );
    terminal.read_for(Duration::from_millis(4100))?;
    let settled = terminal.output.len();
    if reduced {
        // An absence window, kept: no product event falls due inside it that
        // a probe frame could be ordered after, so no barrier can replace it.
        terminal.read_for(Duration::from_millis(700))?;
    } else {
        // The next ambient frame is due within `FRAME_ALLOWANCE`, unless the
        // loop is still recording the first-run notice (one Remote reply).
        terminal.wait("an ambient animation repaint", READY_TIMEOUT, |terminal| {
            Ok(terminal.output.len() > settled)
        })?;
    }
    assert_eq!(terminal.output.len() == settled, reduced);

    // Observe the complete draft frame after focus loss, including its cursor
    // trailer, before checking animation. Text can precede the final flush.
    terminal.send(b"\x1b[Ofocus draft")?;
    terminal.wait_composer_frame(&["focus draft", "enter send"], READY_TIMEOUT)?;
    let settled = terminal.output.len();
    terminal.read_for(Duration::from_millis(400))?;
    terminal.assert_no_output_since(settled, "unfocused terminal is animating")?;
    terminal.send(b"\x1b[I")?;
    terminal.send(&[127; 11])?;
    terminal.wait_text(&["What shall we explore or build?", "enter send"], &[])?;

    if full {
        terminal.command("/help", None)?;
        terminal.command("/tools", None)?;
        terminal.wait_composer_frame(&["\"mcp\": []", "enter send"], READY_TIMEOUT)?;
        let resume = terminal.pause_for(Duration::from_secs(1))?;
        terminal.command("hello from a terminal", None)?;
        drop(resume);
        terminal.command("/parts", None)?;
        terminal.send(b"\x1bOQ")?;
        terminal.wait_text(&["Models", "Esc back"], &[])?;
        terminal.close_picker(b"\r")?;
        terminal.command("/effort default", None)?;
        terminal.command("/mode jungian", None)?;
        terminal.command("/model", Some("Models"))?;
        terminal.close_picker(b"\x1b")?;
        terminal.command("/effort", Some("Efforts"))?;
        terminal.close_picker(b"\r")?;
        terminal.command("/mode", Some("Modes"))?;
        terminal.close_picker(b"\x1b[B\r")?;
        // The welcome placeholder returns only when the dream completes; await
        // that event under the sandbox budget rather than READY_TIMEOUT.
        terminal.submit("/dream")?;
        terminal.wait_dream_settled(sandbox.startup_timeout)?;
        terminal.command("/unknown", None)?;
        terminal
            .wait_composer_frame(&["Unknown command; use /help", "enter send"], READY_TIMEOUT)?;
        terminal.resize(20, 65)?;
        terminal.send(b"\x1b[200~pasted text\x1b[201~")?;
        terminal.wait_text(&["pasted text", "enter send"], &[])?;
        terminal.send(b"\x01\r")?;
        terminal.wait_text(&["What shall we explore or build?", "enter send"], &[])?;
    }
    terminal.wait_idle()?;
    let resume = terminal.pause_for(Duration::from_secs(1))?;
    terminal.resize(60, 180)?;
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    drop(resume);
    assert!(terminal.output.windows(4).any(|bytes| bytes == b"demo"));
    terminal.assert_restored()
}

/// T18: an interactive session whose standard error is redirected shows its
/// sentence on the terminal, before the interface, and writes none to the file.
#[test]
fn real_pty_redirected_standard_error_still_shows_the_sentence_on_the_terminal() -> Result<()> {
    use kuru::memory_activity::{OPENING, SENTENCES};
    let sandbox = Sandbox::new()?;
    let stderr = sandbox.root.path().join("redirected-stderr");
    let inner = sandbox.command("demo");
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "stderr=$1; shift; exec \"$@\" 2>\"$stderr\"", "sh"])
        .arg(&stderr)
        .arg(inner.get_program())
        .args(inner.get_args());
    for (name, value) in inner.get_envs() {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    terminal.wait_composer_frame(&["KURU", "enter send"], sandbox.startup_timeout)?;
    let startup = primary_screen_bytes(&terminal.output)?;
    ensure!(
        contains_bytes(startup, OPENING.as_bytes()),
        "the sentence did not reach the terminal"
    );
    assert_sentence_erased(startup)?;
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()?;
    let redirected = std::fs::read(&stderr)?;
    for sentence in SENTENCES {
        ensure!(
            !contains_bytes(&redirected, sentence.as_bytes()),
            "{sentence:?} was written to the redirected file: {:?}",
            String::from_utf8_lossy(&redirected)
        );
    }
    Ok(())
}

/// With markers on, each is a whole line of its own on the same terminal as
/// the sentence, never inside it: the waiting marker arrives while the opening
/// sentence is on screen, and the screen holds no sentence once memory is
/// ready.
#[test]
fn real_pty_marker_lines_never_share_a_row_with_the_sentence() -> Result<()> {
    use kuru::memory_activity::{MARKER_PREFIX, MARKERS_ENV, OPENING, SENTENCES, WAITING};
    let sandbox = Sandbox::new()?;
    // An existing project, so every stage after the wait is the opening one.
    let seed = sandbox
        .command("demo")
        .args(["run", "create the project first", "--json"])
        .output()?;
    ensure!(
        seed.status.success(),
        "{}",
        String::from_utf8_lossy(&seed.stderr)
    );
    let options = memory_options(&sandbox)?;
    memory::await_owner_exit(&options)?;
    let held = kuru_memory::test_support::hold_owner_lock(&options)?;

    let mut command = sandbox.command("demo");
    command.env(MARKERS_ENV, "1");
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    terminal.wait(
        "the waiting sentence on the terminal",
        sandbox.startup_timeout,
        |terminal| Ok(contains_bytes(&terminal.output, WAITING.as_bytes())),
    )?;
    held.release()?;
    terminal.wait_composer_frame(&["KURU", "enter send"], sandbox.startup_timeout)?;
    let startup = primary_screen_bytes(&terminal.output)?;
    // A marker comes before the sentence it introduces.
    for (event, sentence) in [("open-start", OPENING), ("waiting-ownership", WAITING)] {
        let marker = format!("{MARKER_PREFIX}{event} ");
        let marker_at = rfind_bytes(startup, marker.as_bytes())
            .with_context(|| format!("no {event} marker was written"))?;
        let sentence_at = startup
            .windows(sentence.len())
            .position(|bytes| bytes == sentence.as_bytes())
            .with_context(|| format!("{sentence:?} was never written"))?;
        ensure!(
            marker_at < sentence_at,
            "{event} did not precede {sentence:?}"
        );
    }
    let mut screen = vt100::Parser::new(35, 120, 0);
    screen.process(startup);
    let rows: Vec<String> = screen
        .screen()
        .rows(0, 120)
        .map(|row| row.trim_end().to_owned())
        .collect();
    let mut previous = 0_u128;
    let mut events = Vec::new();
    for row in rows.iter().filter(|row| row.contains("kuru-open-marker")) {
        let rest = row
            .strip_prefix(MARKER_PREFIX)
            .with_context(|| format!("a marker row holds more than the marker: {row:?}"))?;
        let (event, nanoseconds) = rest
            .split_once(' ')
            .with_context(|| format!("malformed marker row {row:?}"))?;
        let nanoseconds: u128 = nanoseconds
            .parse()
            .with_context(|| format!("marker clock is not decimal in {row:?}"))?;
        ensure!(
            nanoseconds >= previous,
            "marker clock went backwards: {rows:?}"
        );
        previous = nanoseconds;
        events.push(event);
    }
    ensure!(
        events == ["open-start", "waiting-ownership", "ready"],
        "{rows:?}"
    );
    for row in &rows {
        ensure!(
            !SENTENCES.iter().any(|sentence| row.contains(sentence)),
            "a sentence is still on the screen after ready: {rows:?}"
        );
    }
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

#[test]
fn real_pty_accepts_chat_navigation_commands_and_restores_terminal() -> Result<()> {
    let sandbox = Sandbox::new()?;
    smoke(&sandbox, false, true, true)?;
    smoke(&sandbox, true, false, false)?;
    assert!(
        sandbox
            .sessions()?
            .iter()
            .any(|session| { session.label == "hello from a terminal" && session.turns >= 1 })
    );
    Ok(())
}

/// The first launch on a machine with an empty cache builds the store
/// template inside its open, then copies the new project from it. The owner
/// is held where creation begins until the creating sentence is on the
/// terminal; once released, the sentence stays up while the template's build
/// store exists, and no other sentence replaces it before memory is ready and
/// the line is erased.
#[test]
fn real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds() -> Result<()> {
    use kuru::memory_activity::CREATING;
    let (sandbox, cache) = Sandbox::fresh_cache()?;
    let templates = template_root(&cache);
    let holds = sandbox.root.path().join("holds");
    let hold = holds.join("CreatingDatabase.hold");
    let mut terminal = None;
    let outcome = (|| -> Result<()> {
        std::fs::create_dir(&holds)?;
        // The marker carries the budget of the wait that ends in its removal, the
        // creating-sentence wait below (kuru-memory `OPEN_HOLD_DIR_ENV`).
        std::fs::write(&hold, sandbox.startup_timeout.as_millis().to_string())?;
        let mut command = sandbox.command("demo");
        command.env(kuru_memory::test_support::OPEN_HOLD_DIR_ENV, &holds);
        terminal = Some(Terminal::spawn(command, 35, 120)?);
        let terminal = terminal.as_mut().expect("terminal was spawned");
        terminal.wait(
            "the creating sentence on the terminal",
            sandbox.startup_timeout,
            |terminal| Ok(contains_bytes(&terminal.output, CREATING.as_bytes())),
        )?;
        ensure!(
            !lists(&templates, |_| true)?,
            "the store template cache was used before creation began"
        );
        std::fs::remove_file(&hold)?;
        terminal.wait(
            "the store template's build store",
            sandbox.startup_timeout,
            |_| lists(&templates, |name| name.starts_with(".build-")),
        )?;
        ensure!(
            terminal.screen().contains(CREATING),
            "the creating sentence is not on the screen while the template builds"
        );
        assert_creating_kept(&terminal.output)?;
        terminal.wait_composer_frame(&["KURU", "enter send"], sandbox.startup_timeout)?;
        let startup = primary_screen_bytes(&terminal.output)?;
        assert_creating_kept(startup)?;
        assert_sentence_erased(startup)?;
        ensure!(
            !lists(&templates, |name| name.starts_with('.'))?,
            "the template build left a transient entry"
        );
        let published = std::fs::read_dir(&templates)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|path| path.join("manifest.json").is_file())
            .count();
        ensure!(
            published == 1,
            "the first launch did not publish one store template"
        );
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })();
    // An early assertion must not leave the owner deliberately held while
    // fixture cleanup asks that same owner to retire.
    let released = match std::fs::remove_file(&hold) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("release the fixture's CreatingDatabase hold"),
    };
    let outcome = match (outcome, released) {
        (outcome, Ok(())) => outcome,
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!(
            "releasing the creation hold also failed: {cleanup:#}"
        ))),
    };
    let closed = match terminal.as_mut() {
        Some(terminal) => terminal
            .close(EXIT_TIMEOUT)
            .context("close the first-launch PTY"),
        None => Ok(()),
    };
    drop(terminal);
    let outcome = match (outcome, closed) {
        (outcome, Ok(())) => outcome,
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!(
            "closing the first-launch PTY also failed: {cleanup:#}"
        ))),
    };
    sandbox.root.release(outcome)
}

#[derive(Clone)]
struct ProviderState {
    started: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    release: watch::Receiver<bool>,
}

async fn complete(State(mut state): State<ProviderState>, Json(_): Json<Value>) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let delayed = !*state.release.borrow();
    state.started.store(true, Ordering::SeqCst);
    if delayed {
        tokio::time::timeout(
            Duration::from_secs(15),
            state.release.wait_for(|released| *released),
        )
        .await
        .expect("fixture release timed out")
        .expect("fixture release channel closed");
    }
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{
                    "id":"terminal-fixture",
                    "status":"completed",
                    "output":[{"type":"message", "content":[{
                        "type":"output_text", "text": if delayed { "LATE_RESPONSE_MUST_STAY_ABSENT" }
                        else { "FRESH_RESPONSE_MARKER" }
                    }]}],
                    "usage":{
                        "input_tokens":8,
                        "output_tokens":5,
                        "input_tokens_details":{"cached_tokens":0},
                        "output_tokens_details":{"reasoning_tokens":0}
                    }
                }
            })
        ),
    ).into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_cancels_manual_compaction_before_later_identities() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let (release, receiver) = watch::channel(true);
        let started = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(complete))
            .with_state(ProviderState {
                started: started.clone(),
                requests: requests.clone(),
                release: receiver,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("compact-cancel-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let actor = kuru_core::ModeProfile::builtin(Mode::Ifs)
            .roles
            .seeds()
            .into_iter()
            .next()
            .context("IFS mode omitted its first compactable actor")?
            .id;
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.command(&format!("/focus {actor}"), None)?;
        terminal.wait_composer_frame(&["Speaking focus", "enter send"], READY_TIMEOUT)?;
        terminal.command("manual compact cancellation source", None)?;
        terminal.wait_composer_frame(&["FRESH_RESPONSE_MARKER", "enter send"], READY_TIMEOUT)?;
        let before = requests.load(Ordering::SeqCst);

        started.store(false, Ordering::SeqCst);
        release.send(false)?;
        terminal.send(b"/compact")?;
        terminal.wait_composer_frame(&["/compact", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"\r")?;
        terminal.wait(
            "manual compact provider request started",
            READY_TIMEOUT,
            |_| Ok(started.load(Ordering::SeqCst)),
        )?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(
            &["Cancelled", "turn interrupted", "enter send"],
            READY_TIMEOUT,
        )?;
        release.send(true)?;
        terminal.read_for(Duration::from_millis(250))?;
        ensure!(
            requests.load(Ordering::SeqCst) == before + 1,
            "manual all-active cancellation started a later identity"
        );

        terminal.command(&format!("/compact {actor}"), None)?;
        terminal.wait_composer_frame(
            &[
                &format!("Compacted {actor}"),
                "source sequences",
                "; original",
                "records remain stored.",
                "enter send",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(
            requests.load(Ordering::SeqCst) == before + 2,
            "cancelled compaction advanced its cursor or replayed provider work"
        );
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct CatalogStdioPeer {
    directory: tempfile::TempDir,
    command: PathBuf,
}

impl CatalogStdioPeer {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("stdio_peer.rs");
        let command = directory.path().join("stdio_peer");
        std::fs::write(
            &source,
            include_str!("../../../packages/kuru-connectors/tests/fixtures/stdio_peer.rs"),
        )?;
        let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
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
            .output()?;
        ensure!(
            output.status.success(),
            "stdio MCP fixture failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o500))?;
        Ok(Self { directory, command })
    }

    fn plan(&self, text: &str) -> Result<()> {
        std::fs::write(self.command.with_extension("plan"), text)?;
        Ok(())
    }

    fn started(&self) -> Result<usize> {
        Ok(std::fs::read_dir(self.directory.path())?
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "started")
            })
            .count())
    }

    fn requests(&self) -> Result<String> {
        std::fs::read_dir(self.directory.path())?
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "requests")
            })
            .map(|entry| std::fs::read_to_string(entry.path()))
            .collect::<std::io::Result<String>>()
            .map_err(Into::into)
    }
}

#[derive(Clone)]
struct MixedCatalogState {
    calls: Arc<Mutex<Vec<String>>>,
    provider_receipts: Arc<Mutex<Vec<Value>>>,
    issued: Arc<AtomicBool>,
}

fn mcp_response(alias: &str, request: &Value, state: &MixedCatalogState) -> Value {
    let result = match request["method"].as_str() {
        Some("initialize") => json!({
            "protocolVersion":"2025-11-25",
            "capabilities":{"tools":{}}
        }),
        Some("tools/list") => {
            let tool = if alias == "live" { "effect" } else { "blocked" };
            json!({"tools":[{
                "name":tool,
                "description":format!("{alias} integrated acceptance fixture"),
                "inputSchema":{"type":"object","additionalProperties":false}
            }]})
        }
        Some("tools/call") => {
            state.calls.lock().unwrap().push(alias.to_owned());
            json!({
                "content":[{"type":"text","text":format!("{alias}-effect")}],
                "isError":false
            })
        }
        _ => json!({}),
    };
    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
}

async fn live_mcp(
    State(state): State<MixedCatalogState>,
    Json(request): Json<Value>,
) -> Json<Value> {
    Json(mcp_response("live", &request, &state))
}

async fn denied_mcp(
    State(state): State<MixedCatalogState>,
    Json(request): Json<Value>,
) -> Json<Value> {
    Json(mcp_response("denied", &request, &state))
}

fn projected_mcp_name(alias: &str, tool: &str) -> String {
    format!(
        "mcp_{}",
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("{alias}\0{tool}").as_bytes(),
        )
        .simple()
    )
}

async fn mixed_catalog_complete(
    State(state): State<MixedCatalogState>,
    Json(request): Json<Value>,
) -> Response {
    let speaking = request["instructions"]
        .as_str()
        .is_some_and(|text| text.contains("Phase: speak and act"));
    let receipts = request["input"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "function_call_output")
        .cloned()
        .collect::<Vec<_>>();
    let output = if !speaking {
        json!([{"type":"message","content":[{
            "type":"output_text","text":"MCP_CATALOG_READY"
        }]}])
    } else if !receipts.is_empty() {
        state
            .provider_receipts
            .lock()
            .unwrap()
            .clone_from(&receipts);
        json!([{"type":"message","content":[{
            "type":"output_text","text":"MCP_MIXED_FINAL"
        }]}])
    } else if !state.issued.swap(true, Ordering::SeqCst) {
        let live = request["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|tool| {
                tool["description"]
                    .as_str()
                    .is_some_and(|text| text.contains("MCP live/effect"))
            })
            .and_then(|tool| tool["name"].as_str())
            .expect("live MCP route must be advertised");
        let stale = request["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|tool| {
                tool["description"]
                    .as_str()
                    .is_some_and(|text| text.contains("MCP stale/cached"))
            })
            .and_then(|tool| tool["name"].as_str())
            .expect("stale MCP metadata must be advertised");
        json!([
            {"type":"function_call","call_id":"mixed-live","name":live,
                "arguments":"{}"},
            {"type":"function_call","call_id":"mixed-stale","name":stale,
                "arguments":"{}"},
            {"type":"function_call","call_id":"mixed-denied",
                "name":projected_mcp_name("denied", "blocked"),"arguments":"{}"}
        ])
    } else {
        json!([{"type":"message","content":[{
            "type":"output_text","text":"MCP_UNEXPECTED_EXTRA_ROUND"
        }]}])
    };
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{
                "id":"mixed-catalog-response","status":"completed","output":output,
                "usage":{"input_tokens":8,"output_tokens":5}
            }})
        ),
    )
        .into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_mcp_catalog_parity_and_mixed_call_batch_are_fail_closed() -> Result<()> {
    kuru_memory::test_support::closing(async {
        const FIXTURE_SECRET: &str = "mcp-fixture-secret-must-not-render";
        let sandbox = Sandbox::warmed().await?;
        kuru_platform::fs::Directory::ensure_private(&sandbox.data)?;
        let stale = CatalogStdioPeer::new()?;
        stale.plan(concat!(
            "read\n",
            "write {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{\"tools\":{}}}}\n",
            "read\n",
            "read\n",
            "write {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"cached\",\"description\":\"stale fixture metadata\",\"inputSchema\":{\"type\":\"object\",\"additionalProperties\":false}}]}}\n",
            "eof\n",
        ))?;
        let degraded = CatalogStdioPeer::new()?;
        degraded.plan(concat!(
            "read\n",
            "write {\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32000,\"message\":\"degraded fixture\"}}\n",
            "eof\n",
        ))?;
        let disabled = CatalogStdioPeer::new()?;
        disabled.plan("eof\n")?;

        let state = MixedCatalogState {
            calls: Arc::new(Mutex::new(vec![])),
            provider_receipts: Arc::new(Mutex::new(vec![])),
            issued: Arc::new(AtomicBool::new(false)),
        };
        let app = Router::new()
            .route("/mcp/live", post(live_mcp))
            .route("/mcp/denied", post(denied_mcp))
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(mixed_catalog_complete))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));

        let mut config = sandbox.config()?;
        config.provider = "responses".into();
        config.model = "fixture".into();
        config.api_base = format!("http://{address}/v1");
        config.api_key_env = "KURU_FIXTURE_KEY".into();
        config.max_rounds = 2;
        config.mcp = BTreeMap::from([
            (
                "degraded".into(),
                McpConfig {
                    command: Some(degraded.command.to_string_lossy().into_owned()),
                    ..McpConfig::default()
                },
            ),
            (
                "denied".into(),
                McpConfig {
                    url: Some(format!("http://{address}/mcp/denied")),
                    ..McpConfig::default()
                },
            ),
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    command: Some(disabled.command.to_string_lossy().into_owned()),
                    ..McpConfig::default()
                },
            ),
            (
                "live".into(),
                McpConfig {
                    url: Some(format!("http://{address}/mcp/live")),
                    ..McpConfig::default()
                },
            ),
            (
                "stale".into(),
                McpConfig {
                    command: Some(stale.command.to_string_lossy().into_owned()),
                    ..McpConfig::default()
                },
            ),
        ]);
        config.permissions = vec![
            PermissionRule {
                action: PermissionAction::Allow,
                selector: PermissionSelector::mcp("live", "effect")?,
                path: None,
            },
            PermissionRule {
                action: PermissionAction::Allow,
                selector: PermissionSelector::mcp("stale", "cached")?,
                path: None,
            },
            PermissionRule {
                action: PermissionAction::Deny,
                selector: PermissionSelector::mcp("denied", "blocked")?,
                path: None,
            },
        ];
        let config_path = sandbox.root.path().join("mixed-mcp.toml");
        std::fs::write(&config_path, toml::to_string(&config)?)?;

        let run_tools = || -> Result<std::process::Output> {
            Ok(sandbox
                .command("responses")
                .args(["--model", "fixture", "--config"])
                .arg(&config_path)
                .args(["--trust-workspace-once", "tools"])
                .env("KURU_FIXTURE_KEY", FIXTURE_SECRET)
                .output()?)
        };
        let seed = run_tools()?;
        ensure!(
            seed.status.success(),
            "initial MCP cache seed failed: {}",
            String::from_utf8_lossy(&seed.stderr)
        );
        stale.plan(concat!(
            "read\n",
            "write {\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32000,\"message\":\"stale fixture offline\"}}\n",
            "eof\n",
        ))?;

        let inspected = run_tools()?;
        ensure!(
            inspected.status.success(),
            "mixed MCP CLI inspection failed: {}",
            String::from_utf8_lossy(&inspected.stderr)
        );
        let catalog: Value = serde_json::from_slice(&inspected.stdout)?;
        assert!(!String::from_utf8_lossy(&inspected.stdout).contains(FIXTURE_SECRET));
        assert!(!String::from_utf8_lossy(&inspected.stderr).contains(FIXTURE_SECRET));
        let statuses = catalog["mcp"]
            .as_array()
            .context("CLI tool catalog omitted MCP statuses")?
            .iter()
            .map(|status| {
                (
                    status["alias"].as_str().unwrap().to_owned(),
                    status["availability"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            statuses,
            BTreeMap::from([
                ("degraded".into(), "degraded".into()),
                ("denied".into(), "live".into()),
                ("disabled".into(), "disabled".into()),
                ("live".into(), "live".into()),
                ("stale".into(), "stale".into()),
            ])
        );
        let projected = catalog["tools"]
            .as_array()
            .context("CLI tool catalog omitted tools")?;
        assert!(projected.iter().any(|tool| {
            tool["description"]
                .as_str()
                .is_some_and(|text| text.contains("MCP live/effect"))
        }));
        assert!(projected.iter().any(|tool| {
            tool["description"]
                .as_str()
                .is_some_and(|text| text.contains("MCP stale/cached (stale"))
        }));
        assert!(projected.iter().all(|tool| {
            !tool["description"]
                .as_str()
                .is_some_and(|text| text.contains("MCP denied/blocked"))
        }));

        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config_path)
            .arg("--trust-workspace-once")
            .env("KURU_FIXTURE_KEY", FIXTURE_SECRET)
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 50, 160)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.command("/tools", None)?;
        terminal.wait_composer_frame(
            &[
                "\"degraded\"",
                "\"denied\"",
                "\"disabled\"",
                "\"live\"",
                "\"stale\"",
            ],
            READY_TIMEOUT,
        )?;
        let tools_frame = terminal.screen();
        assert!(!tools_frame.contains(FIXTURE_SECRET));
        for (alias, availability) in &statuses {
            let alias_marker = format!("\"alias\": \"{alias}\"");
            let alias_start = tools_frame
                .find(&alias_marker)
                .with_context(|| format!("/tools omitted MCP alias {alias}: {tools_frame}"))?;
            let after_alias = &tools_frame[alias_start + alias_marker.len()..];
            let status_block =
                &after_alias[..after_alias.find("\"alias\":").unwrap_or(after_alias.len())];
            ensure!(
                status_block.contains(&format!("\"availability\": \"{availability}\"")),
                "/tools disagreed with CLI state for {alias}: {tools_frame}"
            );
        }
        terminal.command("Exercise the mixed MCP batch", None)?;
        terminal.wait_composer_frame(&["MCP_MIXED_FINAL", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;

        assert_eq!(state.calls.lock().unwrap().as_slice(), ["live"]);
        let receipts = state.provider_receipts.lock().unwrap();
        assert_eq!(
            receipts
                .iter()
                .map(|item| item["call_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["mixed-live", "mixed-stale", "mixed-denied"]
        );
        let outputs = receipts
            .iter()
            .map(|item| item["output"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(outputs[0].contains("live-effect"), "{}", outputs[0]);
        assert!(outputs[1].contains("MCP route failed"), "{}", outputs[1]);
        assert!(outputs[2].contains("permission denied"), "{}", outputs[2]);
        ensure!(
            !stale.requests()?.contains("\"method\":\"tools/call\""),
            "stale MCP received an effect request"
        );
        ensure!(
            !degraded.requests()?.contains("\"method\":\"tools/call\""),
            "degraded MCP received an effect request"
        );
        assert_eq!(disabled.started()?, 0, "disabled MCP process started");
        Ok(())
    })
    .await
}

#[test]
fn real_pty_mcp_oauth_status_uses_the_shared_command_family_without_network() -> Result<()> {
    let sandbox = Sandbox::new()?;
    kuru_platform::fs::Directory::ensure_private(&sandbox.data)?;
    let mut config = sandbox.config()?;
    config.mcp = BTreeMap::from([(
        "auth".into(),
        McpConfig {
            enabled: false,
            url: Some("https://disabled.example.test/mcp".into()),
            oauth: Some(McpOAuthConfig {
                enabled: true,
                client_id: Some("native-client".into()),
                client_secret_env: Some("KURU_TEST_MCP_CLIENT_SECRET".into()),
                scopes: vec!["mcp.read".into()],
                ..McpOAuthConfig::default()
            }),
            ..McpConfig::default()
        },
    )]);
    let config_path = sandbox.root.path().join("mcp-oauth-pty.toml");
    std::fs::write(&config_path, toml::to_string(&config)?)?;

    let mut command = sandbox.command("demo");
    command
        .args(["--config"])
        .arg(&config_path)
        .arg("--trust-workspace-once")
        .env(
            "KURU_TEST_MCP_CLIENT_SECRET",
            "recognizable-pty-client-secret",
        )
        .env("KURU_REDUCED_MOTION", "1");
    let mut terminal = Terminal::spawn(command, 45, 150)?;
    terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
    terminal.command("/mcp status auth", None)?;
    terminal.wait_composer_frame(
        &[
            "\"alias\": \"auth\"",
            "\"state\": \"disabled\"",
            "\"availability\": \"disabled\"",
            "enter send",
        ],
        READY_TIMEOUT,
    )?;
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    ensure!(
        !String::from_utf8_lossy(&terminal.output).contains("recognizable-pty-client-secret"),
        "MCP status terminal output disclosed the configured client secret"
    );
    terminal.assert_restored()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_mcp_device_login_status_logout_matches_cli() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        kuru_platform::fs::Directory::ensure_private(&sandbox.data)?;
        let oauth = HttpsMcpFixture::start(sandbox.root.path()).await;
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let response_calls = Arc::clone(&provider_calls);
        let provider = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move || {
                    let calls = Arc::clone(&response_calls);
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let _provider = Server(tokio::spawn(async move {
            axum::serve(listener, provider).await.unwrap();
        }));

        let mut config = sandbox.config()?;
        config.provider = "responses".into();
        config.model = "fixture".into();
        config.api_base = format!("http://{address}/v1");
        config.api_key_env = "KURU_FIXTURE_KEY".into();
        config.mcp.insert(
            "secure".into(),
            McpConfig {
                url: Some(format!("{}/mcp", oauth.base)),
                oauth: Some(McpOAuthConfig {
                    enabled: true,
                    client_id: Some("synthetic-client".into()),
                    scopes: vec!["mcp.read".into()],
                    ..McpOAuthConfig::default()
                }),
                ..McpConfig::default()
            },
        );
        let config_path = sandbox.root.path().join("mcp-device-pty.toml");
        std::fs::write(&config_path, toml::to_string(&config)?)?;
        let cli_status = || -> Result<Value> {
            let output = sandbox
                .command("responses")
                .args(["--model", "fixture", "--config"])
                .arg(&config_path)
                .args(["--trust-workspace-once", "mcp", "status", "secure"])
                .env("KURU_FIXTURE_KEY", "fixture-key")
                .env("KURU_TEST_MCP_CA_PEM", &oauth.ca_path)
                .output()?;
            ensure!(
                output.status.success(),
                "selected CLI MCP status failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(serde_json::from_slice(&output.stdout)?)
        };

        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config_path)
            .arg("--trust-workspace-once")
            .env("KURU_FIXTURE_KEY", "fixture-key")
            .env("KURU_TEST_MCP_CA_PEM", &oauth.ca_path)
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 50, 160)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.command("/mcp login --device secure", None)?;
        terminal.wait_composer_frame(&["Signed in to MCP secure.", "enter send"], READY_TIMEOUT)?;
        ensure!(
            String::from_utf8_lossy(&terminal.output).contains("TEST-CODE"),
            "device login did not show its public user code"
        );
        assert_eq!(oauth.device_requests.load(Ordering::SeqCst), 1);
        assert_eq!(oauth.token_requests.load(Ordering::SeqCst), 1);
        let forms = oauth.token_forms();
        assert_eq!(forms.len(), 1);
        assert_eq!(
            forms[0].get("grant_type").map(String::as_str),
            Some("urn:ietf:params:oauth:grant-type:device_code")
        );
        assert_eq!(
            forms[0].get("device_code").map(String::as_str),
            Some("synthetic-device")
        );

        terminal.command("/mcp status secure", None)?;
        terminal
            .wait_composer_frame(&["\"state\": \"authorized\"", "enter send"], READY_TIMEOUT)?;
        assert_eq!(cli_status()?["state"], "authorized");
        terminal.command("/mcp logout secure", None)?;
        terminal.wait_composer_frame(&["\"local_deleted\": true", "enter send"], READY_TIMEOUT)?;
        assert_eq!(oauth.revocations.load(Ordering::SeqCst), 1);
        terminal.command("/mcp status secure", None)?;
        terminal.wait_composer_frame(
            &["\"state\": \"login_required\"", "enter send"],
            READY_TIMEOUT,
        )?;
        assert_eq!(cli_status()?["state"], "login_required");
        terminal.command("/help", None)?;
        terminal.wait_composer_frame(&["/mcp", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
        ensure!(
            provider_calls.load(Ordering::SeqCst) == 0,
            "MCP account commands sent a provider request"
        );
        let output = String::from_utf8_lossy(&terminal.output);
        for secret in [
            "synthetic-mcp-access",
            "synthetic-mcp-refresh",
            "synthetic-device",
        ] {
            ensure!(
                !output.contains(secret),
                "MCP account output included a private credential"
            );
        }
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_mcp_refusals_keep_composer_and_other_aliases_idle() -> Result<()> {
    kuru_memory::test_support::closing(async {
        const SECRET: &str = "recognizable-pty-refusal-client-secret";
        let sandbox = Sandbox::warmed().await?;
        kuru_platform::fs::Directory::ensure_private(&sandbox.data)?;
        let unrelated_stdio = CatalogStdioPeer::new()?;
        unrelated_stdio.plan("eof\n")?;

        let unrelated_http_calls = Arc::new(AtomicUsize::new(0));
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let mcp_calls = Arc::clone(&unrelated_http_calls);
        let response_calls = Arc::clone(&provider_calls);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move || {
                    let calls = Arc::clone(&response_calls);
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    }
                }),
            )
            .route(
                "/mcp/unrelated",
                axum::routing::any(move || {
                    let calls = Arc::clone(&mcp_calls);
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));

        let mut config = sandbox.config()?;
        config.provider = "responses".into();
        config.model = "fixture".into();
        config.api_base = format!("http://{address}/v1");
        config.api_key_env = "KURU_FIXTURE_KEY".into();
        config.mcp = BTreeMap::from([
            (
                "disabled".into(),
                McpConfig {
                    enabled: false,
                    url: Some("https://127.0.0.1:9/mcp".into()),
                    oauth: Some(McpOAuthConfig {
                        enabled: true,
                        client_id: Some("synthetic-native-client".into()),
                        client_secret_env: Some("KURU_TEST_MCP_CLIENT_SECRET".into()),
                        ..McpOAuthConfig::default()
                    }),
                    ..McpConfig::default()
                },
            ),
            (
                "stdio".into(),
                McpConfig {
                    command: Some(unrelated_stdio.command.to_string_lossy().into_owned()),
                    ..McpConfig::default()
                },
            ),
            (
                "unrelated_http".into(),
                McpConfig {
                    url: Some(format!("http://{address}/mcp/unrelated")),
                    ..McpConfig::default()
                },
            ),
        ]);
        let config_path = sandbox.root.path().join("mcp-refusal-pty.toml");
        std::fs::write(&config_path, toml::to_string(&config)?)?;

        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config_path)
            .arg("--trust-workspace-once")
            .env("KURU_FIXTURE_KEY", "fixture-key")
            .env("KURU_TEST_MCP_CLIENT_SECRET", SECRET)
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 50, 160)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        let cases = [
            (
                "/mcp status missing",
                "configured MCP alias missing is unavailable",
            ),
            (
                "/mcp login --no-browser missing",
                "configured MCP alias missing is unavailable",
            ),
            (
                "/mcp logout missing",
                "configured MCP alias missing is unavailable",
            ),
            (
                "/mcp status stdio",
                "selected MCP alias does not enable OAuth",
            ),
            (
                "/mcp login --no-browser stdio",
                "selected MCP alias does not enable OAuth",
            ),
            (
                "/mcp logout stdio",
                "selected MCP alias does not enable OAuth",
            ),
            ("/mcp status disabled", "\"state\": \"disabled\""),
            (
                "/mcp login --no-browser disabled",
                "configured MCP alias disabled is unavailable",
            ),
            (
                "/mcp logout disabled",
                "configured MCP alias disabled is unavailable",
            ),
        ];
        for (request, expected) in cases {
            terminal.command(request, None)?;
            terminal.wait_composer_frame(&[expected, "enter send"], READY_TIMEOUT)?;
            ensure!(
                unrelated_stdio.started()? == 0,
                "{request} started an unrelated STDIO MCP"
            );
            ensure!(
                unrelated_http_calls.load(Ordering::SeqCst) == 0,
                "{request} contacted an unrelated HTTP MCP"
            );
            ensure!(
                provider_calls.load(Ordering::SeqCst) == 0,
                "{request} sent a provider request"
            );
        }
        terminal.command("/help", None)?;
        terminal.wait_composer_frame(&["/mcp", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        ensure!(
            !String::from_utf8_lossy(&terminal.output).contains(SECRET),
            "MCP refusal terminal output disclosed the configured client secret"
        );
        terminal.assert_restored()?;

        // An unapproved automatic project claim stops before the TUI exists. It
        // cannot share the composer assertion above because no session is opened.
        let local = sandbox.project.join(".kuru");
        std::fs::create_dir(&local)?;
        std::fs::write(
            local.join("config.toml"),
            "[mcp.automatic]\nurl = 'https://127.0.0.1:9/mcp'\n[mcp.automatic.oauth]\nenabled = true\nclient_id = 'synthetic-native-client'\n",
        )?;
        let mut unapproved = sandbox.command("responses");
        unapproved
            .args(["--model", "fixture", "--config"])
            .arg(&config_path)
            .env("KURU_FIXTURE_KEY", "fixture-key")
            .env("KURU_TEST_MCP_CLIENT_SECRET", SECRET);
        let mut refused = Terminal::spawn(unapproved, 50, 160)?;
        refused.wait_text(&["Choice [3]:"], &[])?;
        refused.send(b"3\r")?;
        let exit = refused.wait_exit(EXIT_TIMEOUT).unwrap_err();
        ensure!(exit.to_string().contains("child failed"), "{exit:#}");
        let output = String::from_utf8_lossy(&refused.output);
        ensure!(
            output.contains("workspace trust was not granted"),
            "automatic MCP claim did not stop at trust preflight: {output}"
        );
        ensure!(
            !output.contains("\x1b[?1049h") && !output.contains(SECRET),
            "unapproved automatic MCP claim reached the TUI or disclosed a secret"
        );
        ensure!(
            unrelated_stdio.started()? == 0,
            "preflight started STDIO MCP"
        );
        ensure!(
            unrelated_http_calls.load(Ordering::SeqCst) == 0,
            "preflight sent HTTP MCP request"
        );
        ensure!(
            provider_calls.load(Ordering::SeqCst) == 0,
            "preflight sent provider request"
        );
        refused.assert_restored()
    })
    .await
}

#[derive(Clone)]
struct PermissionProvider {
    tool: String,
    arguments: String,
}

async fn permission_complete(
    State(state): State<PermissionProvider>,
    Json(request): Json<Value>,
) -> Response {
    let speaking = request["instructions"]
        .as_str()
        .is_some_and(|text| text.contains("Phase: speak and act"));
    let continued = request["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    let output = if !speaking || continued {
        json!([{"type":"message","content":[{"type":"output_text","text":"PERMISSION_FINAL"}]}])
    } else {
        json!([{"type":"function_call","call_id":"permission-fixture", "name":state.tool,
            "arguments":state.arguments}])
    };
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{"id":"permission-response","status":"completed","output":output,
                    "usage":{"input_tokens":8,"output_tokens":5}}
            })
        ),
    )
        .into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_tool_cards_expand_checked_recorded_diffs_at_120_and_80() -> Result<()> {
    kuru_memory::test_support::closing(async {
        for (rows, columns) in [(35, 120), (24, 80)] {
            let sandbox = Sandbox::warmed().await?;
            let target = sandbox.project.join("card-note.txt");
            std::fs::write(&target, "before\nsk-proj-abcdefgh01234567\n")?;
            let app = Router::new()
                .route(
                    "/v1/models",
                    get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
                )
                .route("/v1/responses", post(permission_complete))
                .with_state(PermissionProvider {
                    tool: "file_write".into(),
                    arguments:
                        json!({"path":"card-note.txt","content":"after\nsk-proj-ijklmnop01234567\n"})
                            .to_string(),
                });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let config = sandbox.root.path().join("tool-card-provider.toml");
            std::fs::write(
                &config,
                format!(
                    "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                    listener.local_addr()?
                ),
            )?;
            let _server = Server(tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap()
            }));
            let mut command = sandbox.command("responses");
            command
                .args([
                    "--model",
                    "fixture",
                    "--mode",
                    "freudian",
                    "--allow-write",
                    "--config",
                ])
                .arg(&config)
                .env("KURU_FIXTURE_KEY", "fixture")
                .env("KURU_REDUCED_MOTION", "1");
            let mut terminal = Terminal::spawn(command, rows, columns)?;
            terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
            terminal.submit("edit this note")?;
            terminal.wait_composer_frame(
                &["file_write", "Complete", "PERMISSION_FINAL", "F6 select"],
                READY_TIMEOUT,
            )?;
            ensure!(
                terminal.screen().matches("PERMISSION_FINAL").count() == 1,
                "final answer rendered twice: {}",
                terminal.screen()
            );
            // Expansion reads retained checked snapshots, not the now changed
            // workspace. This mutation cannot become the card's recorded diff.
            std::fs::write(&target, "UNRELATED_WORKSPACE_CONTENT")?;
            terminal.send(b"\x1b[17~\x1b[18~")?;
            terminal.wait_composer_frame(&["-before", "+after"], READY_TIMEOUT)?;
            // The expanded card can exceed the viewport. Observe its heading
            // through completed scrolling frames, separately from the hunk.
            let navigation_deadline = Instant::now() + READY_TIMEOUT;
            while !terminal.screen().contains("checked recorded file diff") {
                let previous_screen = terminal.screen();
                let previous_output = terminal.output.len();
                terminal.send(b"\x1b[5~")?;
                terminal.wait(
                    "recorded diff heading after PageUp",
                    navigation_deadline.saturating_duration_since(Instant::now()),
                    |terminal| {
                        Ok(terminal.screen() != previous_screen
                            && terminal.completed_frame_after(previous_output))
                    },
                )?;
            }
            ensure!(!terminal.screen().contains("UNRELATED_WORKSPACE_CONTENT"));
            ensure!(!String::from_utf8_lossy(&terminal.output).contains("sk-proj-abcdefgh01234567"));
            ensure!(!String::from_utf8_lossy(&terminal.output).contains("sk-proj-ijklmnop01234567"));
            let previous_output = terminal.output.len();
            terminal.send(b"\x1b[18~")?;
            terminal.wait(
                "collapsed recorded diff frame",
                navigation_deadline.saturating_duration_since(Instant::now()),
                |terminal| Ok(terminal.completed_frame_after(previous_output)),
            )?;
            // Collapsing preserves a valid history position. Return through
            // the ordinary scroll control rather than assuming it jumps down.
            while !(terminal.screen().contains("file_write")
                && terminal.screen().contains("PERMISSION_FINAL"))
            {
                let previous_screen = terminal.screen();
                let previous_output = terminal.output.len();
                terminal.send(b"\x1b[6~")?;
                terminal.wait(
                    "collapsed card after PageDown",
                    navigation_deadline.saturating_duration_since(Instant::now()),
                    |terminal| {
                        Ok(terminal.screen() != previous_screen
                            && terminal.completed_frame_after(previous_output))
                    },
                )?;
            }
            ensure!(!terminal.screen().contains("checked recorded file diff"));
            terminal.resize(rows, if columns == 120 { 80 } else { 120 })?;
            terminal.wait_composer_frame(
                &["file_write", "PERMISSION_FINAL", "enter send"],
                READY_TIMEOUT,
            )?;
            terminal.send(b"/quit\r")?;
            terminal.wait_exit(EXIT_TIMEOUT)?;
            terminal.assert_restored()?;
        }
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_tool_cards_keep_safe_partial_preview_after_owned_shell_cancellation() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(permission_complete))
            .with_state(PermissionProvider {
                tool: "shell".into(),
                arguments:
                    json!({"command":"printf 'CARD_SAFE sk-proj-abcdefgh01234567\\n'; exec /bin/sleep 120"})
                        .to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("partial-card-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let mut command = sandbox.command("responses");
        command
            .args([
                "--model",
                "fixture",
                "--mode",
                "freudian",
                "--allow-shell",
                "--config",
            ])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 24, 80)?;
        terminal.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        terminal.submit("run the held shell")?;
        terminal.wait_composer_frame(&["shell", "Pending", "F6 select"], READY_TIMEOUT)?;
        terminal.send(b"\x1b[17~\x1b[18~")?;
        // Pending was observed before expansion; its header can now be above
        // the viewport. The cancel hint proves this safe preview is still live.
        terminal.wait_composer_frame(
            &["stdout (partial)", "CARD_SAFE", "esc cancel"],
            READY_TIMEOUT,
        )?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(&["Cancelled", "CARD_SAFE", "enter send"], READY_TIMEOUT)?;
        ensure!(!String::from_utf8_lossy(&terminal.output).contains("sk-proj-abcdefgh01234567"));
        ensure!(!terminal.screen().contains("PERMISSION_FINAL"));
        terminal.resize(35, 120)?;
        terminal.wait_composer_frame(&["Cancelled", "CARD_SAFE", "enter send"], READY_TIMEOUT)?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_permission_choices_show_exact_file_scope_and_revoke_grants() -> Result<()> {
    kuru_memory::test_support::closing(async {
        for (answer, dimensions, granted, remembered) in [
            (b"\x1b1".as_slice(), (24, 80), true, false),
            (b"\x1b2".as_slice(), (35, 120), true, true),
            (b"\x1b3".as_slice(), (24, 80), true, true),
            (b"\x1b4".as_slice(), (35, 120), false, false),
        ] {
            let sandbox = Sandbox::warmed().await?;
            let marker = sandbox.project.join("literal[1].txt");
            let app = Router::new()
                .route(
                    "/v1/models",
                    get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
                )
                .route("/v1/responses", post(permission_complete))
                .with_state(PermissionProvider {
                    tool: "file_write".into(),
                    arguments: json!({"path":"literal[1].txt","content":"approved"}).to_string(),
                });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let config = sandbox.root.path().join("permission-provider.toml");
            std::fs::write(
                &config,
                format!(
                    "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                    listener.local_addr()?
                ),
            )?;
            let _server = Server(tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap()
            }));
            let mut command = sandbox.command("responses");
            command
                .args(["--model", "fixture", "--mode", "freudian", "--config"])
                .arg(&config)
                .env("KURU_FIXTURE_KEY", "fixture")
                .env("KURU_REDUCED_MOTION", "1");
            let mut terminal = Terminal::spawn(command, dimensions.0, dimensions.1)?;
            terminal.wait_text_with_timeout(
                &["KURU", "enter send"],
                &[],
                sandbox.startup_timeout,
            )?;
            terminal.send(b"Permission fixture turn\r")?;
            terminal.wait_composer_frame(
                &[
                    "Permission request",
                    "literal[1].txt",
                    "1 once",
                    "also Alt+digit",
                    "esc cancel",
                ],
                READY_TIMEOUT,
            )?;
            ensure!(!marker.exists(), "file operation ran before approval");
            terminal.send(answer)?;
            terminal.wait("permission choice settled", READY_TIMEOUT, |terminal| {
                let screen = terminal.screen();
                Ok(!screen.contains("Permission request") && screen.contains("enter send"))
            })?;
            ensure_eq_marker(&marker, granted)?;
            terminal.send(b"/permissions\r")?;
            if remembered {
                terminal.wait_text(&["Selected exact scope", "literal[1].txt"], &[])?;
                terminal.send(b"\x1b[3~")?;
                terminal.wait_text(&["No session or always grants"], &[])?;
            } else {
                terminal.wait_text(&["No session or always grants"], &[])?;
            }
            terminal.send(b"\x1b")?;
            terminal.wait("permission inspector closed", READY_TIMEOUT, |terminal| {
                Ok(!terminal.screen().contains("Permissions · ↑↓ select"))
            })?;
            terminal.send(b"/quit\r")?;
            terminal.wait_exit(EXIT_TIMEOUT)?;
            terminal.assert_restored()?;
        }
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_resume_continue_and_fork_processes_reset_session_only_file_authority() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let marker = sandbox.project.join("literal[1].txt");
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(permission_complete))
            .with_state(PermissionProvider {
                tool: "file_write".into(),
                arguments: json!({"path":"literal[1].txt","content":"approved"}).to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("session-permission-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let spawn = |selector: Option<(&str, &str)>| -> Result<Terminal> {
            let mut command = sandbox.command("responses");
            command
                .args(["--model", "fixture", "--mode", "freudian", "--config"])
                .arg(&config)
                .env("KURU_FIXTURE_KEY", "fixture")
                .env("KURU_REDUCED_MOTION", "1");
            if let Some((flag, id)) = selector {
                command.arg(flag);
                if !id.is_empty() {
                    command.arg(id);
                }
            }
            Terminal::spawn(command, 48, 120)
        };

        let mut first = spawn(None)?;
        first.wait_composer_frame(&["enter send"], sandbox.startup_timeout)?;
        first.send(b"PERMISSION-ORIGINAL-TURN\r")?;
        first.wait_composer_frame(
            &["Permission request", "literal[1].txt", "esc cancel"],
            READY_TIMEOUT,
        )?;
        ensure!(!marker.exists(), "initial file write preceded approval");
        first.send(b"2")?;
        first.wait_composer_frame(&["PERMISSION_FINAL", "enter send"], READY_TIMEOUT)?;
        ensure_eq_marker(&marker, true)?;
        first.send(b"/permissions\r")?;
        first.wait_text(&["Selected exact scope", "literal[1].txt"], &[])?;
        first.send(b"\x1b")?;
        first.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
        first.send(b"/quit\r")?;
        first.wait_exit(EXIT_TIMEOUT)?;
        first.assert_restored()?;

        let listed = sandbox.command("demo").arg("sessions").output()?;
        ensure!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        let listed: Value = serde_json::from_slice(&listed.stdout)?;
        let source = listed[0]["id"]
            .as_str()
            .context("source session missing")?
            .to_owned();
        let source_node = listed[0]["head_node_id"]
            .as_str()
            .context("settled source node missing")?
            .to_owned();
        std::fs::remove_file(&marker)?;

        for (selector, label) in [
            (Some(("--resume", source.as_str())), "resume"),
            (Some(("--continue", "")), "continue"),
        ] {
            let mut terminal = spawn(selector)?;
            terminal.wait_composer_frame(
                &["PERMISSION-ORIGINAL-TURN", "enter send"],
                sandbox.startup_timeout,
            )?;
            terminal.send(b"/permissions\r")?;
            terminal.wait_text(&["No session or always grants"], &[])?;
            terminal.send(b"\x1b")?;
            terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
            terminal.send(format!("PERMISSION-{label}-TURN\r").as_bytes())?;
            terminal.wait_composer_frame(
                &["Permission request", "literal[1].txt", "esc cancel"],
                READY_TIMEOUT,
            )?;
            ensure!(
                !marker.exists(),
                "{label} reused the previous process grant"
            );
            terminal.send(b"4")?;
            terminal.wait_composer_frame(&["PERMISSION_FINAL", "enter send"], READY_TIMEOUT)?;
            ensure!(!marker.exists(), "{label} denied file write still ran");
            terminal.send(b"/quit\r")?;
            terminal.wait_exit(EXIT_TIMEOUT)?;
            terminal.assert_restored()?;
        }

        let forked = sandbox
            .command("demo")
            .args([
                "sessions",
                "fork",
                &source,
                &source_node,
                "--label",
                "permission child",
            ])
            .output()?;
        ensure!(
            forked.status.success(),
            "{}",
            String::from_utf8_lossy(&forked.stderr)
        );
        let forked: Value = serde_json::from_slice(&forked.stdout)?;
        let child = forked["session_id"]
            .as_str()
            .context("forked child missing")?;
        let mut terminal = spawn(Some(("--resume", child)))?;
        terminal.wait_composer_frame(
            &["PERMISSION-ORIGINAL-TURN", "enter send"],
            sandbox.startup_timeout,
        )?;
        terminal.send(b"/permissions\r")?;
        terminal.wait_text(&["No session or always grants"], &[])?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(&["enter send"], READY_TIMEOUT)?;
        terminal.send(b"PERMISSION-FORK-TURN\r")?;
        terminal.wait_composer_frame(
            &["Permission request", "literal[1].txt", "esc cancel"],
            READY_TIMEOUT,
        )?;
        ensure!(
            !marker.exists(),
            "fork inherited the previous process grant"
        );
        terminal.send(b"4")?;
        terminal.wait_composer_frame(&["PERMISSION_FINAL", "enter send"], READY_TIMEOUT)?;
        ensure!(!marker.exists(), "fork denied file write still ran");
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[derive(Clone)]
struct NestedInstructionProvider {
    seen_instructions: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn nested_instruction_complete(
    State(state): State<NestedInstructionProvider>,
    Json(request): Json<Value>,
) -> Response {
    let instructions = request["instructions"].as_str().unwrap_or_default();
    let speaking = instructions.contains("Phase: speak and act");
    let continued = request["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    if speaking {
        state
            .seen_instructions
            .lock()
            .unwrap()
            .push(instructions.to_owned());
    }
    let output = if speaking && !continued {
        json!([{"type":"function_call","call_id":"nested-write", "name":"file_write",
            "arguments":json!({"path":"src/reviewed.txt","content":"must not run before replanning"}).to_string()}])
    } else {
        json!([{"type":"message","content":[{"type":"output_text","text":"NESTED_FINAL"}]}])
    };
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{"id":"nested-response","status":"completed","output":output,
                    "usage":{"input_tokens":8,"output_tokens":5}}
            })
        ),
    )
        .into_response()
}

async fn nested_read_search_complete(
    State(state): State<NestedInstructionProvider>,
    Json(request): Json<Value>,
) -> Response {
    let instructions = request["instructions"].as_str().unwrap_or_default();
    let speaking = instructions.contains("Phase: speak and act");
    let call = if speaking {
        let mut seen = state.seen_instructions.lock().unwrap();
        seen.push(instructions.to_owned());
        seen.len()
    } else {
        0
    };
    let output = match call {
        1 => json!([{"type":"function_call","call_id":"nested-read-src", "name":"file_read",
            "arguments":json!({"path":"src/one.txt"}).to_string()}]),
        2 => json!([{"type":"function_call","call_id":"nested-read-sibling", "name":"file_read",
            "arguments":json!({"path":"sibling/two.txt"}).to_string()}]),
        3 => json!([{"type":"function_call","call_id":"nested-search", "name":"grep",
            "arguments":json!({"pattern":"needle"}).to_string()}]),
        _ => {
            let search = request["input"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|item| {
                    item["type"] == "function_call_output" && item["call_id"] == "nested-search"
                })
                .and_then(|item| item["output"].as_str())
                .unwrap_or_default();
            let result = if search.contains("one.txt") && search.contains("two.txt") {
                "NESTED_READ_FINAL"
            } else {
                "SEARCH_RESULT_MISSING"
            };
            json!([{"type":"message","content":[{"type":"output_text","text":result}]}])
        }
    };
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\n",
            json!({
                "type":"response.completed",
                "response":{"id":"nested-read-response","status":"completed","output":output,
                    "usage":{"input_tokens":8,"output_tokens":5}}
            })
        ),
    )
        .into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_nested_read_and_search_activate_only_used_subtrees() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        for (directory, instruction, file) in [
            ("src", "src instruction", "one.txt"),
            ("sibling", "sibling instruction", "two.txt"),
        ] {
            std::fs::create_dir(sandbox.project.join(directory))?;
            std::fs::write(
                sandbox.project.join(directory).join("AGENTS.md"),
                instruction,
            )?;
            std::fs::write(sandbox.project.join(directory).join(file), "needle\n")?;
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(nested_read_search_complete))
            .with_state(NestedInstructionProvider {
                seen_instructions: seen.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("nested-read-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=5\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--mode", "freudian", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"Nested path reads\r")?;
        terminal.wait_composer_frame(
            &["Workspace instruction review", "src/AGENTS.md"],
            READY_TIMEOUT,
        )?;
        ensure!(
            seen.lock().unwrap().len() == 1,
            "provider continued before src review"
        );
        terminal.send(b"1")?;
        terminal.wait_composer_frame(
            &["Workspace instruction review", "sibling/AGENTS.md"],
            READY_TIMEOUT,
        )?;
        ensure!(
            seen.lock().unwrap().len() == 2,
            "provider continued before sibling review"
        );
        terminal.send(b"1")?;
        terminal.wait_text(&["NESTED_READ_FINAL"], &["Workspace instruction review"])?;
        let requests = seen.lock().unwrap();
        ensure!(
            requests.len() >= 4,
            "read/search continuation did not reach provider"
        );
        ensure!(!requests[0].contains("src instruction"));
        ensure!(!requests[0].contains("sibling instruction"));
        ensure!(requests[1].contains("src instruction"));
        ensure!(!requests[1].contains("sibling instruction"));
        ensure!(requests[2].contains("src instruction"));
        ensure!(requests[2].contains("sibling instruction"));
        ensure!(requests[3].contains("src instruction"));
        ensure!(requests[3].contains("sibling instruction"));
        drop(requests);
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_nested_instruction_review_is_separate_and_precedes_write() -> Result<()> {
    kuru_memory::test_support::closing(async {
        for (answer, granted) in [(b"1".as_slice(), true), (b"3".as_slice(), false)] {
            let sandbox = Sandbox::warmed().await?;
            std::fs::create_dir(sandbox.project.join("src"))?;
            std::fs::write(
                sandbox.project.join("src/AGENTS.md"),
                "nested AGENTS first\n@rules.md\n",
            )?;
            std::fs::write(sandbox.project.join("src/rules.md"), "nested import second")?;
            std::fs::write(sandbox.project.join("src/CLAUDE.md"), "nested CLAUDE third")?;
            let marker = sandbox.project.join("src/reviewed.txt");
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let app = Router::new()
                .route(
                    "/v1/models",
                    get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
                )
                .route("/v1/responses", post(nested_instruction_complete))
                .with_state(NestedInstructionProvider {
                    seen_instructions: seen.clone(),
                });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let config = sandbox.root.path().join("nested-provider.toml");
            std::fs::write(
                &config,
                format!(
                    "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                    listener.local_addr()?
                ),
            )?;
            let _server = Server(tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap()
            }));
            let mut command = sandbox.command("responses");
            command
                .args([
                    "--model",
                    "fixture",
                    "--mode",
                    "freudian",
                    "--allow-write",
                    "--config",
                ])
                .arg(&config)
                .env("KURU_FIXTURE_KEY", "fixture")
                .env("KURU_REDUCED_MOTION", "1");
            let mut terminal = Terminal::spawn(command, 35, 120)?;
            terminal.wait_text_with_timeout(
                &["KURU", "enter send"],
                &[],
                sandbox.startup_timeout,
            )?;
            terminal.send(b"Nested instruction turn\r")?;
            terminal.wait_composer_frame(
                &[
                    "Workspace instruction review",
                    "src/AGENTS.md",
                    "1 continue once",
                    "3 deny",
                ],
                READY_TIMEOUT,
            )?;
            ensure!(!marker.exists(), "nested write ran before workspace review");
            terminal.send(answer)?;
            terminal.wait_text(&["NESTED_FINAL"], &["Workspace instruction review"])?;
            ensure!(
                !marker.exists(),
                "pre-discovery write must replan or be denied"
            );
            let requests = seen.lock().unwrap();
            ensure!(
                requests.len() >= 2,
                "provider did not receive the tool continuation"
            );
            ensure!(!requests[0].contains("nested AGENTS first"));
            ensure!(
                requests[1].contains("nested AGENTS first") == granted,
                "provider instruction authority did not match the review answer"
            );
            if granted {
                let agents = requests[1]
                    .find("nested AGENTS first")
                    .context("AGENTS missing")?;
                let imported = requests[1]
                    .find("nested import second")
                    .context("import missing")?;
                let claude = requests[1]
                    .find("nested CLAUDE third")
                    .context("CLAUDE missing")?;
                ensure!(
                    agents < imported && imported < claude,
                    "nested instruction order changed"
                );
            }
            drop(requests);
            terminal.send(b"/quit\r")?;
            terminal.wait_exit(EXIT_TIMEOUT)?;
            terminal.assert_restored()?;
        }
        Ok(())
    })
    .await
}

fn ensure_eq_marker(path: &std::path::Path, expected: bool) -> Result<()> {
    ensure!(
        path.exists() == expected,
        "unexpected permission effect at {}",
        path.display()
    );
    if expected {
        ensure!(
            std::fs::read_to_string(path)? == "approved",
            "file content changed unexpectedly"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_shell_permission_cancel_closes_reply_and_preserves_draft() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let marker = sandbox.project.join("shell-permission-marker");
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(permission_complete))
            .with_state(PermissionProvider {
                tool: "shell".into(),
                arguments: json!({"command":"printf approved > shell-permission-marker"})
                    .to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("permission-shell.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--mode", "freudian", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 24, 80)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"First shell turn\r")?;
        terminal.wait_composer_frame(
            &[
                "Permission request",
                "native shell",
                "whole tool",
                "4 deny",
                "also Alt+digit",
            ],
            READY_TIMEOUT,
        )?;
        terminal.send(b"Next draft")?;
        terminal.wait_composer_frame(&["Next draft", "Permission request"], READY_TIMEOUT)?;
        ensure!(
            !marker.exists(),
            "shell ran while permission reply was pending"
        );
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(&["Cancelled", "Next draft", "enter send"], READY_TIMEOUT)?;
        ensure!(
            terminal.screen().contains("shell") && terminal.screen().contains("F6 select"),
            "cancelled card missing: {}",
            terminal.screen()
        );
        terminal.send(b"\x1b1")?;
        ensure!(
            !marker.exists(),
            "late answer dispatched the cancelled shell invocation"
        );
        terminal.send(b"\r")?;
        terminal.wait_composer_frame(
            &["Permission request", "native shell", "whole tool"],
            READY_TIMEOUT,
        )?;
        ensure!(
            !marker.exists(),
            "new shell invocation ran before its own decision"
        );
        terminal.send(b"\x1b4")?;
        terminal.wait("denied shell settled", READY_TIMEOUT, |terminal| {
            Ok(!terminal.screen().contains("Permission request")
                && terminal.screen().contains("enter send"))
        })?;
        ensure!(!marker.exists(), "denied shell created a marker");
        terminal.wait_composer_frame(
            &["shell", "Denied", "F6 select", "enter send"],
            READY_TIMEOUT,
        )?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_long_literal_permission_scope_survives_resize_and_inspection() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let parent = "a".repeat(220);
        let leaf = "b".repeat(220);
        std::fs::create_dir(sandbox.project.join(&parent))?;
        let target = format!("{parent}/{leaf}");
        let marker = sandbox.project.join(&target);
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(permission_complete))
            .with_state(PermissionProvider {
                tool: "file_write".into(),
                arguments: json!({"path":target,"content":"approved"}).to_string(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("permission-long-file.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=3\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--mode", "freudian", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 24, 48)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"Long literal path turn\r")?;
        terminal.wait_composer_frame(
            &[
                "Permission request",
                "Exact grant scope",
                "1 once",
                "Alt+digit",
            ],
            READY_TIMEOUT,
        )?;
        ensure!(!marker.exists(), "long-path operation ran before approval");
        terminal.send(b"\x1b[B\x1b[B\x1b[B")?;
        terminal.resize(35, 120)?;
        terminal.wait_composer_frame(
            &[
                "Permission request",
                &parent[..16],
                &leaf[..16],
                "2 session",
                "also Alt+digit",
            ],
            READY_TIMEOUT,
        )?;
        terminal.send(b"\x1b2")?;
        terminal.wait("long-path grant settled", READY_TIMEOUT, |terminal| {
            Ok(!terminal.screen().contains("Permission request")
                && terminal.screen().contains("enter send"))
        })?;
        ensure_eq_marker(&marker, true)?;
        terminal.send(b"/permissions\r")?;
        terminal.wait_text(&["Selected exact scope", &parent[..16], &leaf[..16]], &[])?;
        terminal.send(b"\x1b[3~")?;
        terminal.wait_text(&["No session or always grants"], &[])?;
        terminal.send(b"\x1b")?;
        terminal.wait("permission inspector closed", READY_TIMEOUT, |terminal| {
            Ok(!terminal.screen().contains("Permissions · ↑↓ select"))
        })?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[derive(Clone)]
struct StreamingState {
    selected_started: Arc<AtomicBool>,
    selected_requests: Arc<AtomicUsize>,
    release: watch::Receiver<bool>,
    burst: bool,
}

async fn streaming_complete(
    State(state): State<StreamingState>,
    Json(request): Json<Value>,
) -> Response {
    let speaking = request["instructions"]
        .as_str()
        .is_some_and(|instructions| instructions.contains("Phase: speak and act"));
    if !speaking {
        return (
            [(CONTENT_TYPE, "text/event-stream")],
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":{
                    "id":"private-deliberation","status":"completed",
                    "output":[{"type":"message","content":[{"type":"output_text","text":"PRIVATE_PEER_SENTINEL"}]}],
                    "usage":{"input_tokens":8,"output_tokens":5}
                }})
            ),
        )
            .into_response();
    }
    state.selected_started.store(true, Ordering::SeqCst);
    if state.selected_requests.fetch_add(1, Ordering::SeqCst) > 0 {
        return (
            [(CONTENT_TYPE, "text/event-stream")],
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":{
                    "id":"after-cancel","status":"completed",
                    "output":[{"type":"message","content":[{"type":"output_text","text":"FRESH_AFTER_CANCEL"}]}],
                    "usage":{"input_tokens":8,"output_tokens":5}
                }})
            ),
        ).into_response();
    }
    let mut first = format!(
        "data: {}\n\n",
        json!({"type":"response.reasoning_summary_text.delta","item_id":"reasoning-1","output_index":0,"summary_index":0,"delta":"VISIBLE_SUMMARY"}),
    );
    let leading = if state.burst {
        "x".repeat(16 * 1024)
    } else {
        String::new()
    };
    for chunk in leading.as_bytes().chunks(128) {
        first.push_str(&format!(
            "data: {}\n\n",
            json!({"type":"response.output_text.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":std::str::from_utf8(chunk).expect("ASCII burst")}),
        ));
    }
    first.push_str(&format!(
        "data: {}\n\n",
        json!({"type":"response.output_text.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":"VISIBLE_LIVE_TAIL "}),
    ));
    let final_text = format!("{leading}VISIBLE_LIVE_TAIL FINAL_PUBLIC");
    let last = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.output_text.delta","item_id":"message-1","output_index":1,"content_index":0,"delta":"FINAL_PUBLIC"}),
        json!({"type":"response.completed","response":{
            "id":"selected-final","status":"completed",
            "output":[
                {"id":"reasoning-1","type":"reasoning","summary":[{"type":"summary_text","text":"VISIBLE_SUMMARY"}],"encrypted_content":"PRIVATE_NATIVE_SENTINEL"},
                {"id":"message-1","type":"message","content":[{"type":"output_text","text":final_text}]}
            ],
            "usage":{"input_tokens":8,"output_tokens":5}
        }}),
    );
    let body = stream::unfold((0u8, state.release), move |(phase, mut release)| {
        let first = first.clone();
        let last = last.clone();
        async move {
            match phase {
                0 => Some((Ok::<_, io::Error>(first), (1, release))),
                1 => {
                    if !*release.borrow() {
                        release.wait_for(|ready| *ready).await.ok()?;
                    }
                    Some((Ok(last), (2, release)))
                }
                _ => None,
            }
        }
    });
    (
        [(CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body),
    )
        .into_response()
}

#[derive(Clone)]
struct ToolActivityState {
    started: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    release: watch::Receiver<bool>,
}

/// One facing round whose only streamed output is a tool call: the arguments
/// stay open until released, so the rendered activity line can be inspected
/// while the call is genuinely in flight.
async fn tool_activity_complete(
    State(state): State<ToolActivityState>,
    Json(request): Json<Value>,
) -> Response {
    let speaking = request["instructions"]
        .as_str()
        .is_some_and(|instructions| instructions.contains("Phase: speak and act"));
    let continued = request["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    if !speaking || continued || state.requests.fetch_add(1, Ordering::SeqCst) > 0 {
        return (
            [(CONTENT_TYPE, "text/event-stream")],
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":{
                    "id":"tool-activity-final","status":"completed",
                    "output":[{"type":"message","content":[{"type":"output_text","text":"FINAL_PUBLIC"}]}],
                    "usage":{"input_tokens":8,"output_tokens":5}
                }})
            ),
        )
            .into_response();
    }
    state.started.store(true, Ordering::SeqCst);
    let arguments = json!({"activation":0.4,"note":"PRIVATE_ARGUMENT_SENTINEL"}).to_string();
    let first = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.reasoning_summary_text.delta","item_id":"reasoning-1","output_index":0,"summary_index":0,"delta":"VISIBLE_SUMMARY"}),
        json!({"type":"response.function_call_arguments.delta","item_id":"tool-1","output_index":1,"delta":arguments}),
    );
    let last = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.function_call_arguments.done","item_id":"tool-1","output_index":1,"name":"state_report","arguments":arguments}),
        json!({"type":"response.completed","response":{
            "id":"tool-activity-call","status":"completed",
            "output":[
                {"id":"reasoning-1","type":"reasoning","summary":[{"type":"summary_text","text":"VISIBLE_SUMMARY"}]},
                {"id":"tool-1","type":"function_call","call_id":"tool-activity-1","name":"state_report","arguments":arguments}
            ],
            "usage":{"input_tokens":8,"output_tokens":5}
        }}),
    );
    let body = stream::unfold((0u8, state.release), move |(phase, mut release)| {
        let first = first.clone();
        let last = last.clone();
        async move {
            match phase {
                0 => Some((Ok::<_, io::Error>(first), (1, release))),
                1 => {
                    if !*release.borrow() {
                        release.wait_for(|ready| *ready).await.ok()?;
                    }
                    Some((Ok(last), (2, release)))
                }
                _ => None,
            }
        }
    });
    (
        [(CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body),
    )
        .into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_activity_line_tracks_a_streaming_tool_call() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let (release, receiver) = watch::channel(false);
        let started = Arc::new(AtomicBool::new(false));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(tool_activity_complete))
            .with_state(ToolActivityState {
                started: started.clone(),
                requests: Arc::new(AtomicUsize::new(0)),
                release: receiver,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("tool-activity-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=2\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 24, 80)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"Call a tool\r")?;
        terminal.wait("selected provider request started", READY_TIMEOUT, |_| {
            Ok(started.load(Ordering::SeqCst))
        })?;
        terminal.wait_composer_frame(
            &["activity · Calling tool", "VISIBLE_SUMMARY"],
            READY_TIMEOUT,
        )?;
        let streaming = terminal.screen();
        ensure!(
            !streaming.contains("Responding"),
            "activity still claimed plain responding during a tool call: {streaming}"
        );
        ensure!(
            !streaming.contains("PRIVATE_ARGUMENT_SENTINEL") && !streaming.contains("activation"),
            "tool arguments were rendered: {streaming}"
        );
        release.send(true)?;
        terminal.wait_composer_frame(&["FINAL_PUBLIC", "enter send"], READY_TIMEOUT)?;
        let settled = terminal.screen();
        ensure!(
            !settled.contains("Calling tool") && !settled.contains("Calling state_report"),
            "a settled turn kept a stale calling label: {settled}"
        );
        let output = String::from_utf8_lossy(&terminal.output);
        ensure!(
            !output.contains("PRIVATE_ARGUMENT_SENTINEL"),
            "tool arguments appeared in terminal output"
        );
        terminal.send(b"\x03")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_previews_delayed_native_selected_stream_at_three_sizes() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let (release, receiver) = watch::channel(false);
        let selected_started = Arc::new(AtomicBool::new(false));
        let selected_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(streaming_complete))
            .with_state(StreamingState {
                selected_started: selected_started.clone(),
                selected_requests,
                release: receiver,
                burst: true,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("streaming-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 24, 80)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        let prompt = format!(
            "Stream to the user\n{}",
            (0..40)
                .map(|row| format!("PUBLIC_READING_{row:03}\n"))
                .collect::<String>()
        );
        terminal.send(format!("\x1b[200~{prompt}\x1b[201~\r").as_bytes())?;
        terminal.wait("selected provider request started", READY_TIMEOUT, |_| {
            Ok(selected_started.load(Ordering::SeqCst))
        })?;
        terminal.wait_composer_frame(&["VISIBLE_LIVE_TAIL", "thinking"], READY_TIMEOUT)?;
        let before_scroll = terminal.output.len();
        terminal.send(b"\x1b[5~")?;
        terminal.wait(
            "completed held-stream reading frame",
            READY_TIMEOUT,
            |terminal| {
                Ok(terminal.completed_frame_after(before_scroll)
                    && terminal.screen().contains("PUBLIC_READING_"))
            },
        )?;
        let reading = terminal
            .screen()
            .split_whitespace()
            .find(|token| token.starts_with("PUBLIC_READING_"))
            .context("held-stream public anchor missing")?
            .to_owned();
        for (rows, cols) in [(24, 80), (40, 120), (18, 40)] {
            if (rows, cols) != (24, 80) {
                terminal.resize(rows, cols)?;
            }
            terminal.wait_composer_frame(
                &[
                    "VISIBLE_LIVE_TAIL",
                    "thinking",
                    "VISIBLE_SUMMARY",
                    "earlier text omitted",
                ],
                READY_TIMEOUT,
            )?;
            let screen = terminal.screen();
            // The inherited 40-column preview case has no transcript rows
            // left after its panel and chrome. At practical widths, resizing
            // must keep the saved public reading item visible.
            if cols >= 80 {
                ensure!(
                    screen.contains(&reading),
                    "reading item moved at {cols}x{rows}: {screen}"
                );
            }
            ensure!(
                !screen.contains("PRIVATE_PEER_SENTINEL")
                    && !screen.contains("PRIVATE_NATIVE_SENTINEL"),
                "private material appeared at {cols}x{rows}: {screen}"
            );
            ensure!(
                !screen.contains("FINAL_PUBLIC"),
                "terminal answer appeared before completion"
            );
        }
        let before_resize = terminal.output.len();
        terminal.resize(24, 80)?;
        terminal.wait(
            "completed restored reading viewport",
            READY_TIMEOUT,
            |terminal| {
                Ok(terminal.completed_frame_after(before_resize)
                    && terminal.screen().contains(&reading)
                    && terminal.screen().contains("VISIBLE_LIVE_TAIL"))
            },
        )?;
        terminal.send(b"\x1b[200~NEXT_DRAFT\x1b[201~")?;
        terminal.wait_composer_frame(&["NEXT_DRAFT", "VISIBLE_LIVE_TAIL"], READY_TIMEOUT)?;
        let navigation_deadline = Instant::now() + READY_TIMEOUT;
        loop {
            ensure!(
                Instant::now() < navigation_deadline,
                "forward navigation did not reach the newest public item: {}",
                terminal.screen()
            );
            let before_scroll = terminal.output.len();
            terminal.send(b"\x1b[6~")?;
            terminal.wait(
                "completed forward-navigation frame",
                navigation_deadline.saturating_duration_since(Instant::now()),
                |terminal| Ok(terminal.completed_frame_after(before_scroll)),
            )?;
            if !terminal.screen().contains("↑ history") {
                break;
            }
        }
        ensure!(
            !terminal.output.contains(&b'\x07'),
            "completion signal preceded settlement"
        );
        let before_focus = terminal.output.len();
        terminal.send(b"\x1b[O")?;
        terminal.wait(
            "completed unfocused held-provider frame",
            READY_TIMEOUT,
            |terminal| Ok(terminal.completed_frame_after(before_focus)),
        )?;
        release.send(true)?;
        terminal
            .wait_composer_frame(&["FINAL_PUBLIC", "NEXT_DRAFT", "enter send"], READY_TIMEOUT)?;
        ensure!(
            terminal
                .output
                .iter()
                .filter(|byte| **byte == b'\x07')
                .count()
                == 1,
            "successful unfocused completion did not emit exactly one content-free bell"
        );
        let screen = terminal.screen();
        ensure!(
            !screen.contains("draft · provisional") && !screen.contains("VISIBLE_SUMMARY"),
            "settled answer retained a provisional panel: {screen}"
        );
        let terminal_output = String::from_utf8_lossy(&terminal.output);
        ensure!(
            !terminal_output.contains("PRIVATE_PEER_SENTINEL")
                && !terminal_output.contains("PRIVATE_NATIVE_SENTINEL"),
            "private material appeared in terminal output"
        );
        terminal.send(b"\x03")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_cancels_provisional_stream_then_retries_only_completed_answer() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let (_release, receiver) = watch::channel(false);
        let selected_started = Arc::new(AtomicBool::new(false));
        let selected_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(streaming_complete))
            .with_state(StreamingState {
                selected_started: selected_started.clone(),
                selected_requests: selected_requests.clone(),
                release: receiver,
                burst: false,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("cancel-streaming-provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args(["--model", "fixture", "--config"])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 24, 80)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"First turn\r")?;
        terminal.wait("selected provider request started", READY_TIMEOUT, |_| {
            Ok(selected_started.load(Ordering::SeqCst))
        })?;
        terminal.wait_composer_frame(&["VISIBLE_LIVE_TAIL", "VISIBLE_SUMMARY"], READY_TIMEOUT)?;
        terminal.send(b"\x1b")?;
        terminal.wait_composer_frame(&["Turn interrupted", "enter send"], READY_TIMEOUT)?;
        let cancelled = terminal.screen();
        ensure!(
            !cancelled.contains("VISIBLE_LIVE_TAIL") && !cancelled.contains("VISIBLE_SUMMARY"),
            "cancelled preview remained visible: {cancelled}"
        );
        terminal.send(b"Second turn\r")?;
        terminal.wait_composer_frame(&["FRESH_AFTER_CANCEL", "enter send"], READY_TIMEOUT)?;
        ensure_eq_selected_requests(&selected_requests, 2)?;
        terminal.send(b"/retry\r")?;
        terminal.wait_composer_frame(
            &["Stored result reused", "FRESH_AFTER_CANCEL"],
            READY_TIMEOUT,
        )?;
        ensure_eq_selected_requests(&selected_requests, 2)?;
        ensure!(!String::from_utf8_lossy(&terminal.output).contains("PRIVATE_NATIVE_SENTINEL"));
        terminal.send(b"\x03")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()
    })
    .await
}

fn ensure_eq_selected_requests(selected_requests: &AtomicUsize, expected: usize) -> Result<()> {
    ensure!(
        selected_requests.load(Ordering::SeqCst) == expected,
        "expected {expected} selected provider requests, got {}",
        selected_requests.load(Ordering::SeqCst)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_cancels_provider_work_preserves_draft_and_accepts_the_next_turn() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let (release, receiver) = watch::channel(false);
        let started = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route("/v1/responses", post(complete))
            .with_state(ProviderState {
                started: started.clone(),
                requests: requests.clone(),
                release: receiver,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.path().join("provider.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_FIXTURE_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let mut command = sandbox.command("responses");
        command
            .args([
                "--debug", "--model", "fixture", "--mode", "freudian", "--config",
            ])
            .arg(&config)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut terminal = Terminal::spawn(command, 35, 120)?;
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
        terminal.send(b"Slow request\r")?;
        terminal.wait("provider started", READY_TIMEOUT, |_| {
            Ok(started.load(Ordering::SeqCst))
        })?;
        terminal.send(b"\x1b[200~Next thought\x1b[201~")?;
        terminal.send(b"\x1bOQ")?;
        terminal.wait_text(&["current turn"], &[])?;
        terminal.send(b"\x1b")?;
        terminal.wait_text(
            &[
                "Cancelled",
                "Turn interrupted",
                "no completed answer was committed",
                "Next thought",
                "enter send",
            ],
            &[],
        )?;
        release.send(true)?;
        terminal.send(b"\r")?;
        terminal.wait_text(
            &[
                "Turn interrupted",
                "FRESH_RESPONSE_MARKER",
                "32 input tokens",
                "20 output tokens",
            ],
            &[],
        )?;
        let completed_requests = requests.load(Ordering::SeqCst);
        terminal.send(b"/retry\r")?;
        terminal.wait_text(
            &[
                "Stored result reused",
                "Turn interrupted",
                "FRESH_RESPONSE_MARKER",
                "enter send",
            ],
            &[],
        )?;
        assert_eq!(
            requests.load(Ordering::SeqCst),
            completed_requests,
            "completed /retry reached the provider"
        );
        terminal.resize(35, 65)?;
        terminal.wait_text(
            &[
                "FRESH_RESPONSE_MARKER",
                "32 input tokens",
                "20 output tokens",
                "enter send",
            ],
            &[],
        )?;
        assert!(
            !String::from_utf8_lossy(&terminal.output).contains("LATE_RESPONSE_MUST_STAY_ABSENT")
        );
        terminal.wait_idle()?;
        terminal.send(b"/quit\r")?;
        terminal.wait_exit(EXIT_TIMEOUT)?;
        terminal.assert_restored()?;
        let transcript = String::from_utf8_lossy(&terminal.output);
        let alternate_start = transcript
            .find("\u{1b}[?1049h")
            .context("TUI did not enter the alternate screen")?;
        let alternate_end = transcript
            .rfind("\u{1b}[?1049l")
            .context("TUI did not leave the alternate screen")?;
        ensure!(
            transcript[..alternate_start].contains("\"debug_ring\"")
                && transcript[..alternate_start].contains("/diagnostics/"),
            "debug ring location was not reported before the TUI: {transcript}"
        );
        ensure!(
            !transcript[alternate_start..alternate_end].contains("debug_ring")
                && !transcript[alternate_start..alternate_end].contains("span_open")
                && !transcript[alternate_start..alternate_end].contains("/diagnostics/"),
            "debug diagnostics leaked to the PTY transcript: {transcript}"
        );
        let logs = diagnostics(&sandbox.data)?;
        ensure!(logs.contains("\"target\":\"kuru.actor\""));
        ensure!(logs.contains("\"status\":\"cancelled\""));
        ensure!(
            logs.contains("span_close"),
            "cancelled spans were not closed"
        );

        let sessions = sandbox.sessions()?;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].turns, 1);
        let mut resume = sandbox.command("responses");
        resume
            .args(["--model", "fixture", "--mode", "freudian", "--config"])
            .arg(&config)
            .arg("--resume")
            .arg(&sessions[0].id)
            .env("KURU_FIXTURE_KEY", "fixture")
            .env("KURU_REDUCED_MOTION", "1");
        let mut resumed = Terminal::spawn(resume, 35, 120)?;
        resumed.wait_text_with_timeout(
            &[
                "Turn interrupted",
                "no completed answer was committed",
                "FRESH_RESPONSE_MARKER",
                "enter send",
            ],
            &[],
            sandbox.startup_timeout,
        )?;
        resumed.send(b"/quit\r")?;
        resumed.wait_exit(EXIT_TIMEOUT)?;
        resumed.assert_restored()?;

        let mut observed_options = memory_options(&sandbox)?;
        observed_options.read_only = true;
        let project = sandbox.project.canonicalize()?;
        let (_, opening) = MemoryStore::open_managed_observed(
            observed_options,
            project.clone(),
            PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
        );
        let memory = opening
            .await
            .context("attach managed read-only PTY history inspector")?;
        let scope = kuru_runtime::project_scope(&project)?;
        let transcript = ModeProfile::builtin(Mode::Freudian)
            .memory
            .transcript_namespace(&scope, &sessions[0].id);
        let history = memory
            .history(&transcript, 500)
            .await
            .context("read committed PTY history")?;
        memory
            .close()
            .await
            .context("close managed read-only PTY history inspector")?;
        assert!(
            history
                .iter()
                .any(|message| message.role == "user"
                    && message.plain_text() == Some("Slow request"))
        );
        assert!(
            history
                .iter()
                .any(|message| message.role == "user"
                    && message.plain_text() == Some("Next thought"))
        );
        assert_eq!(
            history
                .iter()
                .filter(|message| {
                    message.role == kuru_runtime::INTERRUPTION_ROLE
                        && message.plain_text() == Some(kuru_runtime::INTERRUPTION_TEXT)
                })
                .count(),
            1
        );
        assert!(!history.iter().any(|message| {
            message
                .text_projection()
                .contains("LATE_RESPONSE_MUST_STAY_ABSENT")
        }));
        Ok(())
    })
    .await
}

struct Selection<'a> {
    keys: &'a [u8],
    mode: Mode,
    model: &'a str,
    effort: Option<&'a str>,
}

/// Options for the store the application created: reopens are never
/// refused for want of a warm-up.
fn memory_options(sandbox: &Sandbox) -> Result<kuru_memory::OpenOptions> {
    kuru_memory::test_support::open_options(
        sandbox.data.clone(),
        kuru_runtime::project_scope(&sandbox.project)?,
    )
}

// Fault injection owns no application handle: the real UI remains the sole
// writer owner. Only this test's generated endpoint and credentials are read.
struct InvalidSessionCatalogMode {
    connection: MySqlConnection,
    session_id: String,
    previous: String,
    revision: String,
}

impl InvalidSessionCatalogMode {
    async fn inject(sandbox: &Sandbox, selected_session_id: &str) -> Result<Self> {
        let session_id = selected_session_id.to_owned();
        let mut options = memory_options(sandbox)?;
        options.read_only = true;
        let inspector = MemoryStore::open(options).await?;
        let status = inspector.status().await;
        inspector.close().await?;
        let status = status?;
        let directory = status.directory.canonicalize()?;
        ensure!(
            directory.starts_with(sandbox.data.canonicalize()?),
            "fault injection must remain inside the isolated fixture"
        );
        #[derive(Deserialize)]
        struct Identity {
            instance: String,
            project_scope: String,
            password: String,
        }
        #[derive(Deserialize)]
        struct Endpoint {
            instance: String,
            port: u16,
        }
        let identity: Identity =
            serde_json::from_slice(&std::fs::read(directory.join("identity.json"))?)?;
        let endpoint: Endpoint =
            serde_json::from_slice(&std::fs::read(directory.join("endpoint.json"))?)?;
        ensure!(
            identity.instance == endpoint.instance && identity.project_scope == status.project,
            "fixture memory endpoint identity mismatch"
        );
        let options = MySqlConnectOptions::new()
            .host("127.0.0.1")
            .port(endpoint.port)
            .username("root")
            .password(&identity.password)
            .database("kuru/main")
            .ssl_mode(MySqlSslMode::Disabled);
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut connection = MySqlConnection::connect_with(&options).await?;
            let previous =
                sqlx::query_scalar("SELECT mode FROM session_catalog WHERE session_id = ?")
                    .bind(session_id.as_bytes())
                    .fetch_one(&mut connection)
                    .await?;
            let revision = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
                .fetch_one(&mut connection)
                .await?;
            let updated = sqlx::query(
                "UPDATE session_catalog SET mode = ? WHERE session_id = ? AND mode = ?",
            )
            .bind("not-a-kuru-mode")
            .bind(session_id.as_bytes())
            .bind(&previous)
            .execute(&mut connection)
            .await?;
            ensure!(
                updated.rows_affected() == 1,
                "fixture selected session catalog row changed"
            );
            Ok(Self {
                connection,
                session_id,
                previous,
                revision,
            })
        })
        .await
        .context("fixture SQL fault injection timed out")?
    }

    async fn restore(mut self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let updated = sqlx::query(
                "UPDATE session_catalog SET mode = ? WHERE session_id = ? AND mode = ?",
            )
            .bind(&self.previous)
            .bind(self.session_id.as_bytes())
            .bind("not-a-kuru-mode")
            .execute(&mut self.connection)
            .await?;
            ensure!(
                updated.rows_affected() == 1,
                "fixture selected session catalog mode changed"
            );
            let revision: String = sqlx::query_scalar("SELECT DOLT_HASHOF('HEAD')")
                .fetch_one(&mut self.connection)
                .await?;
            ensure!(
                revision == self.revision,
                "rejected choice created a revision"
            );
            let dirty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dolt_status")
                .fetch_one(&mut self.connection)
                .await?;
            ensure!(
                dirty == 0,
                "restored fixture left uncommitted database changes"
            );
            self.connection.close().await?;
            Ok(())
        })
        .await
        .context("fixture SQL fault restoration timed out")?
    }
}

async fn preferences_session(
    sandbox: &Sandbox,
    expected: &[&str],
    selections: &[Selection<'_>],
    reject: bool,
) -> Result<()> {
    let prior_sessions = if reject {
        Some(
            sandbox
                .sessions()?
                .into_iter()
                .map(|session| session.id)
                .collect::<BTreeSet<_>>(),
        )
    } else {
        None
    };
    let mut command = sandbox.command("demo");
    command.env("KURU_REDUCED_MOTION", "1");
    let mut terminal = Terminal::spawn(command, 38, 130)?;
    terminal.wait("initial selections", sandbox.startup_timeout, |terminal| {
        let screen = terminal.screen().to_lowercase();
        Ok(screen.contains("enter send") && expected.iter().all(|value| screen.contains(value)))
    })?;
    let rejected_index = if let Some(prior_sessions) = prior_sessions {
        let created = sandbox
            .sessions()?
            .into_iter()
            .map(|session| session.id)
            .filter(|id| !prior_sessions.contains(id))
            .collect::<Vec<_>>();
        ensure!(
            created.len() == 1,
            "fixture TUI launch did not create exactly one selected session"
        );
        Some(InvalidSessionCatalogMode::inject(sandbox, &created[0]).await?)
    } else {
        None
    };
    for selection in selections {
        terminal.wait_idle()?;
        if selection.keys.starts_with(b"/") {
            terminal.command(
                std::str::from_utf8(selection.keys)?.trim_end_matches('\r'),
                None,
            )?;
        } else {
            terminal.send(&selection.keys[..3])?;
            terminal.wait_text(&["Esc back"], &[])?;
            terminal.close_picker(&selection.keys[3..])?;
        }
        terminal.wait("selection rendered", READY_TIMEOUT, |terminal| {
            let screen = terminal.screen();
            Ok(screen.contains("enter send")
                && (!reject
                    || (screen.contains("memory service write outcome is uncertain")
                        && !screen.contains("not-a-kuru-mode"))))
        })?;
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let actual = sandbox.effective_config().await?;
            if actual.mode == selection.mode
                && actual.model == selection.model
                && actual.effort.as_deref() == selection.effort
            {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "selection was rendered but its saved preference did not match"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    if let Some(fault) = rejected_index {
        fault.restore().await?;
    }
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

#[tokio::test]
async fn terminal_selections_survive_restarts_picker_changes_and_failed_database_writes()
-> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = Sandbox::warmed().await?;
        let initial = sandbox.config()?;
        preferences_session(
            &sandbox,
            &["demo", "ifs"],
            &[
                Selection {
                    keys: b"/mode jungian\r",
                    mode: Mode::Jungian,
                    model: &initial.model,
                    effort: initial.effort.as_deref(),
                },
                Selection {
                    keys: b"/model persistent-demo\r",
                    mode: Mode::Jungian,
                    model: "persistent-demo",
                    effort: None,
                },
                Selection {
                    keys: b"/effort high\r",
                    mode: Mode::Jungian,
                    model: "persistent-demo",
                    effort: Some("high"),
                },
            ],
            false,
        )
        .await?;
        assert_eq!(sandbox.effective_config().await?.mode, Mode::Jungian);
        preferences_session(
            &sandbox,
            &["persistent-demo", "jungian", "high"],
            &[
                Selection {
                    keys: b"\x1bOS\x1b[A\r",
                    mode: Mode::Freudian,
                    model: "persistent-demo",
                    effort: Some("high"),
                },
                Selection {
                    keys: b"\x1bOQ\r",
                    mode: Mode::Freudian,
                    model: "demo",
                    effort: None,
                },
                Selection {
                    keys: b"/effort high\r",
                    mode: Mode::Freudian,
                    model: "demo",
                    effort: Some("high"),
                },
                Selection {
                    keys: b"\x1bOR\r",
                    mode: Mode::Freudian,
                    model: "demo",
                    effort: None,
                },
            ],
            false,
        )
        .await?;
        preferences_session(&sandbox, &["demo", "freudian", "default"], &[], false).await?;
        let current = sandbox.effective_config().await?;
        assert_eq!(
            (
                current.mode,
                current.model.as_str(),
                current.effort.as_deref()
            ),
            (Mode::Freudian, "demo", None)
        );
        preferences_session(
            &sandbox,
            &["demo", "freudian", "default"],
            &[Selection {
                keys: b"/mode jungian\r",
                mode: Mode::Freudian,
                model: "demo",
                effort: None,
            }],
            true,
        )
        .await?;
        assert_eq!(sandbox.effective_config().await?.mode, Mode::Freudian);
        let sessions = sandbox.sessions()?;
        assert_eq!(sessions.len(), 4);
        assert!(sessions.iter().all(|session| session.turns == 0));
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.mode)
                .collect::<Vec<_>>(),
            [
                Mode::Freudian,
                Mode::Freudian,
                Mode::Freudian,
                Mode::Jungian
            ]
        );
        Ok(())
    })
    .await
}
