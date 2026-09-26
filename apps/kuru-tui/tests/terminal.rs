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
use mcp_oauth_https::HttpsMcpFixture;
use terminal::{READY_TIMEOUT, Terminal, startup_timeout};

const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

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

struct Sandbox {
    root: memory::ServiceCleanup,
    project: PathBuf,
    data: PathBuf,
    startup_timeout: Duration,
}

impl Sandbox {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project)?;
        let configuration = memory::configuration(root.path())?;
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

    async fn preferences(&self) -> Result<kuru_core::ProjectPreferences> {
        let mut options = memory_options(self)?;
        options.read_only = true;
        let memory = MemoryStore::open(options).await?;
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
        "nested-dimensions" => {
            let mut terminal = fixture_with_size("dimensions", 35, 120)?;
            terminal.wait_text(&["SIZE:120x35"], &[])?;
            terminal.wait_exit(EXIT_TIMEOUT)?;
            println!("INNER_SIZE_OK");
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
                    transcript: Vec::new(),
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
    let sandbox = Sandbox::new()?;
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
    terminal.wait_composer_frame(&["/memory-candidate-abandon", "enter send"], READY_TIMEOUT)?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_and_pty_session_actions_share_catalog_identity_and_public_transcript() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
    let (_, opening) = MemoryStore::open_managed_observed(
        memory_options(&sandbox)?,
        sandbox.project.canonicalize()?,
        PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
    );
    let memory = opening.await.context("attach managed CLI catalog")?;
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
    memory.close().await?;

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
            line.contains(&cli_removed) && line.contains("CLI-REMOVED") && line.contains("removed")
        }),
        "CLI removed child was not distinct in the PTY picker: {picker}"
    );
    ensure!(
        picker_lines.iter().any(|line| {
            line.contains(&cli_pending) && line.contains("CLI-PENDING") && line.contains("pending")
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
            line.contains(&cli_pending) && line.contains("pending") && line.contains("CLI-PENDING")
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
            .any(|line| { line.contains(&child_id) && line.contains("active fork") }),
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_lifecycle_hooks_rewrite_and_annotate_without_exposing_hook_output() -> Result<()>
{
    let sandbox = Sandbox::new()?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_hook_refusals_and_speaker_stop_leave_the_session_usable() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_started_kuru_run_reaches_provider_without_reentering_hooks() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_custom_command_uses_reviewed_catalog_and_literal_arguments() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
    let sandbox = Sandbox::new()?;
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
    let sandbox = Sandbox::new()?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumed_pty_reports_automatic_compaction_cost_once_and_reuses_its_summary() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
    terminal.wait_composer_frame(&["RESUMED_ORDINARY_RESPONSE", "enter send"], READY_TIMEOUT)?;
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

fn smoke(sandbox: &Sandbox, reduced: bool, full: bool, expect_notice: bool) -> Result<()> {
    let mut command = sandbox.command("demo");
    command.args(["--mode", "freudian"]);
    if reduced {
        command.env("KURU_REDUCED_MOTION", "1");
    }
    let mut terminal = Terminal::spawn(command, 35, 120)?;
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
    let alternate = terminal
        .output
        .windows(b"\x1b[?1049h".len())
        .position(|bytes| bytes == b"\x1b[?1049h")
        .context("terminal did not enter its alternate screen")?;
    let startup = &terminal.output[..alternate];
    let waiting = b"Memory: waiting for project ownership";
    let ready = b"Memory: ready.";
    let waiting_at = startup
        .windows(waiting.len())
        .position(|bytes| bytes == waiting)
        .context("memory startup did not report project-ownership wait before the first completed TUI frame")?;
    let ready_at = startup
        .windows(ready.len())
        .position(|bytes| bytes == ready)
        .context("memory startup did not report ready before the first completed TUI frame")?;
    assert!(
        waiting_at < ready_at,
        "memory startup reported ready before ownership wait"
    );
    let mut previous = waiting_at;
    for stage in [
        b"Memory: waiting for verified runtime cache".as_slice(),
        b"Memory: extracting embedded runtime",
        b"Memory: verifying cached runtime".as_slice(),
        b"Memory: checking runtime version",
        b"Memory: preparing database",
        b"Memory: opening database",
    ] {
        if let Some(position) = startup
            .windows(stage.len())
            .position(|bytes| bytes == stage)
        {
            assert!(
                position > previous && position < ready_at,
                "memory startup reordered {stage:?} before the first completed TUI frame"
            );
            previous = position;
        }
    }
    assert!(
        terminal
            .output
            .windows(7)
            .any(|value| value == b"\x1b[38;2;")
    );
    terminal.read_for(Duration::from_millis(4100))?;
    let settled = terminal.output.len();
    terminal.read_for(Duration::from_millis(700))?;
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
        terminal.command("/dream", None)?;
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
    let sandbox = Sandbox::new()?;
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
    const FIXTURE_SECRET: &str = "mcp-fixture-secret-must-not-render";
    let sandbox = Sandbox::new()?;
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
    let sandbox = Sandbox::new()?;
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
    terminal.wait_composer_frame(&["\"state\": \"authorized\"", "enter send"], READY_TIMEOUT)?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_mcp_refusals_keep_composer_and_other_aliases_idle() -> Result<()> {
    const SECRET: &str = "recognizable-pty-refusal-client-secret";
    let sandbox = Sandbox::new()?;
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
async fn real_pty_permission_choices_show_exact_file_scope_and_revoke_grants() -> Result<()> {
    for (answer, dimensions, granted, remembered) in [
        (b"\x1b1".as_slice(), (24, 80), true, false),
        (b"\x1b2".as_slice(), (35, 120), true, true),
        (b"\x1b3".as_slice(), (24, 80), true, true),
        (b"\x1b4".as_slice(), (35, 120), false, false),
    ] {
        let sandbox = Sandbox::new()?;
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
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_resume_continue_and_fork_processes_reset_session_only_file_authority() -> Result<()>
{
    let sandbox = Sandbox::new()?;
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
    let sandbox = Sandbox::new()?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_nested_instruction_review_is_separate_and_precedes_write() -> Result<()> {
    for (answer, granted) in [(b"1".as_slice(), true), (b"3".as_slice(), false)] {
        let sandbox = Sandbox::new()?;
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
        terminal.wait_text_with_timeout(&["KURU", "enter send"], &[], sandbox.startup_timeout)?;
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
    let sandbox = Sandbox::new()?;
    let marker = sandbox.project.join("shell-permission-marker");
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
        )
        .route("/v1/responses", post(permission_complete))
        .with_state(PermissionProvider {
            tool: "shell".into(),
            arguments: json!({"command":"printf approved > shell-permission-marker"}).to_string(),
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
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_long_literal_permission_scope_survives_resize_and_inspection() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
    let sandbox = Sandbox::new()?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_previews_delayed_native_selected_stream_at_three_sizes() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
    terminal.send(b"Stream to the user\r")?;
    terminal.wait("selected provider request started", READY_TIMEOUT, |_| {
        Ok(selected_started.load(Ordering::SeqCst))
    })?;
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
    terminal.send(b"\x1b[200~NEXT_DRAFT\x1b[201~")?;
    terminal.wait_composer_frame(&["NEXT_DRAFT", "VISIBLE_LIVE_TAIL"], READY_TIMEOUT)?;
    release.send(true)?;
    terminal.wait_composer_frame(&["FINAL_PUBLIC", "NEXT_DRAFT", "enter send"], READY_TIMEOUT)?;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_cancels_provisional_stream_then_retries_only_completed_answer() -> Result<()> {
    let sandbox = Sandbox::new()?;
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
    let sandbox = Sandbox::new()?;
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
    assert!(!String::from_utf8_lossy(&terminal.output).contains("LATE_RESPONSE_MUST_STAY_ABSENT"));
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
            .any(|message| message.role == "user" && message.plain_text() == Some("Slow request"))
    );
    assert!(
        history
            .iter()
            .any(|message| message.role == "user" && message.plain_text() == Some("Next thought"))
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
}

struct Selection<'a> {
    keys: &'a [u8],
    mode: Mode,
    model: &'a str,
    effort: Option<&'a str>,
}

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
    let sandbox = Sandbox::new()?;
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
}
