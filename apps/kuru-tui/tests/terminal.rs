#![cfg(unix)]

use std::{
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use kuru_core::{Config, MemoryStore, Mode};
use serde_json::{Value, json};
use tokio::sync::watch;

#[path = "support/terminal.rs"]
mod terminal;
use terminal::{READY_TIMEOUT, Terminal};

const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

struct Sandbox {
    root: tempfile::TempDir,
    project: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project)?;
        Ok(Self {
            root,
            project,
            data,
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
        other => anyhow::bail!("unknown fixture mode {other}"),
    }
    Ok(())
}

fn fixture(mode: &str) -> Result<Terminal> {
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

fn smoke(sandbox: &Sandbox, reduced: bool, full: bool) -> Result<()> {
    let mut command = sandbox.command("demo");
    command.args(["--mode", "freudian"]);
    if reduced {
        command.env("KURU_REDUCED_MOTION", "1");
    }
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    terminal.wait_text(&["KURU", "enter send"], &[])?;
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
        terminal.wait_text(&["unknown command"], &[])?;
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
    smoke(&sandbox, false, true)?;
    smoke(&sandbox, true, false)?;
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
    release: watch::Receiver<bool>,
}

async fn complete(State(mut state): State<ProviderState>, Json(_): Json<Value>) -> Json<Value> {
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
    Json(json!({
        "status":"completed", "output":[{"type":"message", "content":[{
            "type":"output_text", "text": if delayed { "LATE_RESPONSE_MUST_STAY_ABSENT" }
            else { "FRESH_RESPONSE_MARKER" }
        }]}]
    }))
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_cancels_provider_work_preserves_draft_and_accepts_the_next_turn() -> Result<()> {
    let sandbox = Sandbox::new()?;
    let (release, receiver) = watch::channel(false);
    let started = Arc::new(AtomicBool::new(false));
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
        )
        .route("/v1/responses", post(complete))
        .with_state(ProviderState {
            started: started.clone(),
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
        .args(["--model", "fixture", "--mode", "freudian", "--config"])
        .arg(config)
        .env("KURU_FIXTURE_KEY", "fixture")
        .env("KURU_REDUCED_MOTION", "1");
    let mut terminal = Terminal::spawn(command, 35, 120)?;
    terminal.wait_text(&["KURU", "enter send"], &[])?;
    terminal.send(b"Slow request\r")?;
    terminal.wait("provider started", READY_TIMEOUT, |_| {
        Ok(started.load(Ordering::SeqCst))
    })?;
    terminal.send(b"\x1b[200~Next thought\x1b[201~")?;
    terminal.send(b"\x1bOQ")?;
    terminal.wait_text(&["current turn"], &[])?;
    terminal.send(b"\x1b")?;
    terminal.wait_text(&["Cancelled", "Next thought", "enter send"], &[])?;
    release.send(true)?;
    terminal.send(b"\r")?;
    terminal.wait_text(&["FRESH_RESPONSE_MARKER"], &[])?;
    assert!(!String::from_utf8_lossy(&terminal.output).contains("LATE_RESPONSE_MUST_STAY_ABSENT"));
    terminal.wait_idle()?;
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()?;

    let sessions = sandbox.sessions()?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].turns, 1);
    let harness = kuru_runtime::Harness::new(
        Config {
            provider: "demo".into(),
            ..Config::default()
        },
        &sandbox.project,
        MemoryStore::open(&sandbox.data.join("memory.sqlite3"))?,
        Arc::new(kuru_connectors::DemoProvider),
        Some(&sessions[0].id),
    )?;
    let history = harness.history()?;
    assert!(
        history
            .iter()
            .any(|message| message.role == "user" && message.content == "Next thought")
    );
    assert!(
        !history
            .iter()
            .any(|message| message.content.contains("LATE_RESPONSE_MUST_STAY_ABSENT"))
    );
    Ok(())
}

struct Selection<'a> {
    keys: &'a [u8],
    mode: Mode,
    model: &'a str,
    effort: Option<&'a str>,
}

struct RejectPreferences(rusqlite::Connection);
impl Drop for RejectPreferences {
    fn drop(&mut self) {
        let _ = self
            .0
            .execute_batch("DROP TRIGGER IF EXISTS reject_mode_update");
    }
}

fn preferences_session(
    sandbox: &Sandbox,
    expected: &[&str],
    selections: &[Selection<'_>],
    reject: bool,
) -> Result<()> {
    let mut command = sandbox.command("demo");
    command.env("KURU_REDUCED_MOTION", "1");
    let mut terminal = Terminal::spawn(command, 38, 130)?;
    terminal.wait("initial selections", READY_TIMEOUT, |terminal| {
        let screen = terminal.screen().to_lowercase();
        Ok(screen.contains("enter send") && expected.iter().all(|value| screen.contains(value)))
    })?;
    let _reject = if reject {
        let connection = rusqlite::Connection::open(sandbox.data.join("memory.sqlite3"))?;
        connection.execute_batch(
            "CREATE TRIGGER reject_mode_update BEFORE UPDATE ON state
             WHEN NEW.key LIKE '%/sessions'
             BEGIN SELECT RAISE(ABORT, 'preference write rejected'); END",
        )?;
        Some(RejectPreferences(connection))
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
        terminal.wait(
            "selection persisted and rendered",
            READY_TIMEOUT,
            |terminal| {
                let actual = sandbox.config()?;
                let screen = terminal.screen();
                Ok(actual.mode == selection.mode
                    && actual.model == selection.model
                    && actual.effort.as_deref() == selection.effort
                    && screen.contains("enter send")
                    && (!reject
                        || "preference write rejected"
                            .split_whitespace()
                            .all(|word| screen.contains(word))))
            },
        )?;
    }
    terminal.send(b"/quit\r")?;
    terminal.wait_exit(EXIT_TIMEOUT)?;
    terminal.assert_restored()
}

#[test]
fn terminal_selections_survive_restarts_picker_changes_and_failed_database_writes() -> Result<()> {
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
    )?;
    assert_eq!(sandbox.config()?.mode, Mode::Jungian);
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
    )?;
    preferences_session(&sandbox, &["demo", "freudian", "default"], &[], false)?;
    let current = sandbox.config()?;
    assert_eq!(
        (current.mode, current.model.as_str(), current.effort),
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
    )?;
    assert_eq!(sandbox.config()?.mode, Mode::Freudian);
    let sessions = sandbox.sessions()?;
    assert_eq!(sessions.len(), 4);
    assert!(sessions.iter().all(|session| session.turns == 0));
    assert_eq!(
        sessions
            .iter()
            .map(|session| session.mode)
            .collect::<Vec<_>>(),
        [
            Mode::Jungian,
            Mode::Freudian,
            Mode::Freudian,
            Mode::Freudian
        ]
    );
    Ok(())
}
