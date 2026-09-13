#![cfg(all(windows, feature = "test-support"))]
#![forbid(unsafe_code)]

use anyhow::{Context, Result, ensure};
use kuru_core::Config;
use kuru_delivery::command::BlockingCommand;
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[path = "support/memory.rs"]
mod memory;
#[path = "support/windows_terminal.rs"]
mod terminal;
use terminal::{READY, Terminal};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
// Includes the actual memory shutdown grace/escalation and one pending commit.
const EXIT: Duration = Duration::from_secs(30);

#[test]
fn composer_coordinates_use_physical_rows_after_conpty_autowrap() {
    let mut parser = vt100::Parser::new(6, 20, 0);
    // A full-width grid causes autowrap rather than explicit row separators.
    parser.process("x".repeat(60).as_bytes());
    parser.process(" › focus draft      footer\x1b[?25l".as_bytes());
    assert!(parser.screen().row_wrapped(0));
    assert!(
        parser
            .screen()
            .contents()
            .lines()
            .next()
            .unwrap()
            .contains("focus draft")
    );
    assert!(!terminal::composer_frame_ready(
        parser.screen(),
        "focus draft"
    ));

    parser.process(b"\x1b[4;15");
    assert!(!terminal::composer_frame_ready(
        parser.screen(),
        "focus draft"
    ));
    parser.process(b"H");
    assert_eq!(parser.screen().cursor_position(), (3, 14));
    assert!(!terminal::composer_frame_ready(
        parser.screen(),
        "focus draft"
    ));
    parser.process(b"\x1b[?25");
    assert!(!terminal::composer_frame_ready(
        parser.screen(),
        "focus draft"
    ));
    parser.process(b"h");
    assert!(terminal::composer_frame_ready(
        parser.screen(),
        "focus draft"
    ));
    parser.process(b"\x1b[5;1H");
    assert!(!terminal::composer_frame_ready(
        parser.screen(),
        "focus draft"
    ));
}

struct Sandbox {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    data: PathBuf,
    environment: BTreeMap<String, String>,
    startup: Duration,
}

impl Sandbox {
    fn new() -> Result<Self> {
        let temporary = tempfile::tempdir()?;
        let parent = Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable)?;
        let root = parent
            .create_private_directory("terminal 日本語".as_ref())?
            .path()
            .to_path_buf();
        let project = root.join("project with spaces");
        let data = root.join("data");
        for path in [
            &project,
            &root.join("home"),
            &root.join("local"),
            &root.join("tmp"),
        ] {
            std::fs::create_dir(path)?;
        }
        let config = memory::configuration(&root)?;
        let configuration: Config =
            toml::from_str(&std::fs::read_to_string(config.join("kuru/config.toml"))?)?;
        let system = kuru_platform::windows::process::system_directory()?;
        let mut environment = BTreeMap::from([
            (
                "SystemRoot".into(),
                system
                    .parent()
                    .context("System32 parent")?
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("PATH".into(), system.to_string_lossy().into_owned()),
            (
                "HOME".into(),
                root.join("home").to_string_lossy().into_owned(),
            ),
            (
                "USERPROFILE".into(),
                root.join("home").to_string_lossy().into_owned(),
            ),
            (
                "LOCALAPPDATA".into(),
                root.join("local").to_string_lossy().into_owned(),
            ),
            ("APPDATA".into(), config.to_string_lossy().into_owned()),
            (
                "XDG_CONFIG_HOME".into(),
                config.to_string_lossy().into_owned(),
            ),
            (
                "TEMP".into(),
                root.join("tmp").to_string_lossy().into_owned(),
            ),
            (
                "TMP".into(),
                root.join("tmp").to_string_lossy().into_owned(),
            ),
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
        ]);
        if let Ok(profile) = std::env::var("LLVM_PROFILE_FILE") {
            environment.insert("LLVM_PROFILE_FILE".into(), profile);
        }
        let startup =
            Duration::from_secs((configuration.memory.startup_timeout_secs + 5) * 2 + 13) + READY;
        Ok(Self {
            _temporary: temporary,
            root,
            project,
            data,
            environment,
            startup,
        })
    }

    fn args(&self, provider: &str) -> Vec<String> {
        vec![
            "-C".into(),
            self.project.to_string_lossy().into_owned(),
            "--data-dir".into(),
            self.data.to_string_lossy().into_owned(),
            "--provider".into(),
            provider.into(),
            "--no-dream".into(),
        ]
    }

    fn start(
        &self,
        label: &str,
        mode: &str,
        extra: &[&str],
        reduced: bool,
        provider: &str,
        overrides: &[(&str, &str)],
    ) -> Result<Terminal> {
        let mut environment = self.environment.clone();
        if reduced {
            environment.insert("KURU_REDUCED_MOTION".into(), "1".into());
        }
        for (key, value) in overrides {
            environment.insert((*key).into(), (*value).into());
        }
        let mut args = self.args(provider);
        args.extend(extra.iter().map(|arg| (*arg).into()));
        Terminal::spawn(
            &self.root.join(label),
            json!({"mode":mode,"binary":env!("CARGO_BIN_EXE_kuru"),"args":args,"environment":environment,"cwd":self.project}),
            38,
            130,
        )
    }

    fn output(&self, subcommand: &str) -> Result<String> {
        let output = BlockingCommand::new(env!("CARGO_BIN_EXE_kuru"))
            .args(self.args("demo"))
            .arg(subcommand)
            .env_clear()
            .envs(&self.environment)
            .output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_chat_selectors_resize_focus_and_persistent_choices() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::new()?;
    let mut terminal = sandbox.start(
        "first",
        "app",
        &[
            "--mode",
            "freudian",
            "--model",
            "persistent-demo",
            "--effort",
            "high",
        ],
        false,
        "demo",
        &[],
    )?;
    terminal.text(&["KURU", "enter send"], sandbox.startup)?;
    let animated = terminal.output.len();
    terminal.read_for(Duration::from_millis(700))?;
    ensure!(
        terminal.output.len() > animated,
        "native ambient animation did not draw"
    );
    terminal.focus(false)?;
    terminal.send(b"focus draft")?;
    terminal.composer("focus draft")?;
    terminal.quiet(
        "native focus loss continued drawing after its completed composer frame",
        Duration::from_millis(450),
    )?;
    terminal.focus(true)?;
    terminal.send(b"!")?;
    terminal.composer("focus draft!")?;
    let resumed_frame = terminal.output.len();
    terminal.read_for(Duration::from_millis(700))?;
    ensure!(
        terminal.output.len() > resumed_frame,
        "native focus return did not resume animation after a completed frame"
    );
    terminal.send(&[127; 12])?;
    terminal.text(&["What shall we explore"], READY)?;
    terminal.command("hello from native Windows")?;
    terminal.text(&["demo"], READY)?;
    // Start from different persisted values, then select through real picker
    // keys. Demo offers one model and its default effort; no catalog is faked.
    terminal.send(b"\x1bOQ")?;
    terminal.text(&["Models", "demo", "Esc back"], READY)?;
    terminal.send(b"\r")?;
    terminal.text(
        &["Model: demo", "saved for this project", "enter send"],
        READY,
    )?;
    terminal.command("/effort high")?;
    for (keys, label, selection, receipt) in [
        (
            b"\x1bOR".as_slice(),
            "Efforts",
            b"\r".as_slice(),
            "Effort: default",
        ),
        (
            b"\x1bOS".as_slice(),
            "Modes",
            b"\x1b[B\r".as_slice(),
            "Mode: jungian",
        ),
    ] {
        terminal.send(keys)?;
        terminal.text(&[label, "Esc back"], READY)?;
        terminal.send(selection)?;
        terminal.wait("picker selection persisted", READY, |terminal| {
            let screen = terminal.screen();
            !screen.contains("Esc back")
                && screen.contains(receipt)
                && screen.contains("saved for this project")
                && screen.contains("enter send")
        })?;
    }
    terminal.resize(24, 80)?;
    terminal.send(b"navigation draft")?;
    terminal.composer("navigation draft")?;
    terminal.send(b"\x1b[H\x1b[3~")?;
    terminal.text(&["avigation draft"], READY)?;
    terminal.send(b"\x03")?;
    let report = terminal.finish(EXIT)?;
    assert_eq!(report["status"], 0);
    drop(terminal);
    let sessions: Vec<kuru_runtime::Session> = serde_json::from_str(&sandbox.output("sessions")?)?;
    ensure!(
        sessions
            .iter()
            .any(|session| session.label == "hello from native Windows" && session.turns == 1),
        "chat was not durably saved"
    );

    // Reopening memory is the authoritative persistence check. The storage-free
    // config snapshot intentionally omits project preferences.
    let mut reopened = sandbox.start("reopened", "app", &[], true, "demo", &[])?;
    reopened.text(
        &["demo", "Jungian", "default", "enter send"],
        sandbox.startup,
    )?;
    reopened.send(b"reduced draft")?;
    reopened.composer("reduced draft")?;
    reopened.quiet(
        "reduced-motion console continued drawing",
        Duration::from_millis(450),
    )?;
    reopened.send(b"\x03")?;
    assert_eq!(reopened.finish(EXIT)?["status"], 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_trust_refusal_and_persistent_choice_precede_the_alternate_screen()
-> Result<()> {
    let _serial = SERIAL.lock().await;

    let sandbox = Sandbox::new()?;
    std::fs::create_dir(sandbox.root.join(".kuru"))?;
    std::fs::write(
        sandbox.root.join(".kuru/config.toml"),
        "allow_shell = true\nprovider = 'responses'\nmodel = 'fixture-model'\napi_key_env = 'KURU_ABSENT_CONPTY_KEY'\n",
    )?;
    let mut terminal = sandbox.start("trust-refusal", "app", &[], true, "responses", &[])?;
    terminal.text(
        &["Continue once", "Approve this complete configuration"],
        READY,
    )?;
    ensure!(
        !sandbox.data.exists(),
        "trust refusal created memory before a choice"
    );
    terminal.send(b"3\r")?;
    let error = terminal
        .wait("trust refusal exits before the TUI", READY, |_| false)
        .unwrap_err();
    ensure!(error.to_string().contains("child exited"), "{error:#}");
    ensure!(
        !terminal
            .output
            .windows(8)
            .any(|bytes| bytes == b"\x1b[?1049h"),
        "trust refusal entered the alternate screen"
    );
    ensure!(
        !sandbox.data.exists(),
        "trust refusal created memory after the declined choice"
    );

    let sandbox = Sandbox::new()?;
    std::fs::create_dir(sandbox.root.join(".kuru"))?;
    std::fs::write(
        sandbox.root.join(".kuru/config.toml"),
        "allow_shell = true\nprovider = 'responses'\nmodel = 'fixture-model'\napi_key_env = 'KURU_ABSENT_CONPTY_KEY'\n",
    )?;
    let mut terminal = sandbox.start("trust-persistent", "app", &[], true, "responses", &[])?;
    terminal.text(
        &["Continue once", "Approve this complete configuration"],
        READY,
    )?;
    terminal.send(b"2\r")?;
    let error = terminal
        .wait(
            "missing responses route exits before the TUI",
            READY,
            |_| false,
        )
        .unwrap_err();
    ensure!(error.to_string().contains("child exited"), "{error:#}");
    ensure!(
        !terminal
            .output
            .windows(8)
            .any(|bytes| bytes == b"\x1b[?1049h"),
        "persistent trust choice entered the alternate screen"
    );

    let status = BlockingCommand::new(env!("CARGO_BIN_EXE_kuru"))
        .args(sandbox.args("responses"))
        .arg("trust")
        .arg("status")
        .env_clear()
        .envs(&sandbox.environment)
        .output()?;
    ensure!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    ensure!(
        String::from_utf8_lossy(&status.stdout).contains("Status: approved"),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_console_modes_restore_after_partial_initialization_and_errors() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::new()?;
    for mode in ["partial-error", "error-unwind"] {
        let mut terminal = sandbox.start(mode, mode, &[], true, "demo", &[])?;
        let report = terminal.finish(READY)?;
        assert_eq!(report["status"], 1);
        assert_eq!(report["before"], report["after"]);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_error_drop_returns_while_console_close_is_delayed() -> Result<()> {
    use portable_pty::{MasterPty, PtySize};
    use std::{
        io::{Read, Write},
        sync::mpsc,
        thread::{self, ThreadId},
        time::Instant,
    };

    struct GatedMaster {
        inner: Option<Box<dyn MasterPty + Send>>,
        entered: mpsc::Sender<ThreadId>,
        release: mpsc::Receiver<()>,
        closed: mpsc::Sender<bool>,
    }
    impl MasterPty for GatedMaster {
        fn resize(&self, size: PtySize) -> Result<()> {
            self.inner.as_ref().unwrap().resize(size)
        }
        fn get_size(&self) -> Result<PtySize> {
            self.inner.as_ref().unwrap().get_size()
        }
        fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>> {
            self.inner.as_ref().unwrap().try_clone_reader()
        }
        fn take_writer(&self) -> Result<Box<dyn Write + Send>> {
            self.inner.as_ref().unwrap().take_writer()
        }
    }
    impl Drop for GatedMaster {
        fn drop(&mut self) {
            let _ = self.entered.send(thread::current().id());
            // Models delayed closure around a real native console. Channel EOF
            // releases it even if an assertion fails; the fallback is bounded.
            let released = self.release.recv_timeout(READY * 2).is_ok();
            drop(self.inner.take());
            let _ = self.closed.send(released);
        }
    }

    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::new()?;
    let mut terminal = sandbox.start("drop-error", "partial-error", &[], true, "demo", &[])?;
    let error = terminal
        .text(&["marker-the-error-fixture-never-renders"], READY)
        .unwrap_err();
    ensure!(
        error.to_string().contains("child exited"),
        "expected the real console fixture's exit, got {error:#}"
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(sandbox.root.join("drop-error/report.json"))?)?;
    assert_eq!(report["status"], 1);
    assert_eq!(report["before"], report["after"]);

    let (entered, entering) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (closed, completion) = mpsc::channel();
    terminal.wrap_master(|master| {
        Box::new(GatedMaster {
            inner: Some(master),
            entered,
            release: resume,
            closed,
        })
    })?;
    let (returned, returning) = mpsc::channel();
    let dropping = thread::spawn(move || {
        let caller = thread::current().id();
        drop(terminal);
        let _ = returned.send(caller);
    });
    // Capture all observations before asserting, so every failure releases the
    // gate and permits owned native closure. Never join a still-running thread.
    let close_thread = entering.recv_timeout(READY);
    let bounded_return = returning.recv_timeout(READY);
    let release_result = release.send(());
    let actual_close = completion.recv_timeout(READY);
    let deadline = Instant::now() + READY;
    while !dropping.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    ensure!(
        dropping.is_finished(),
        "ConPTY drop observer remains active"
    );
    dropping
        .join()
        .map_err(|_| anyhow::anyhow!("ConPTY drop observer panicked"))?;
    release_result.context("close gate exited without its release")?;
    ensure!(actual_close?, "native close gate timed out before release");
    let caller = bounded_return.context("Terminal::drop blocked on native console closure")?;
    ensure!(
        close_thread? != caller,
        "native close ran on the dropping thread"
    );
    Ok(())
}

#[derive(Clone)]
struct ProviderState {
    started: Arc<AtomicBool>,
    release: tokio::sync::watch::Receiver<bool>,
}

async fn response(
    axum::extract::State(mut state): axum::extract::State<ProviderState>,
    axum::Json(_): axum::Json<Value>,
) -> axum::Json<Value> {
    let delayed = !*state.release.borrow();
    state.started.store(true, Ordering::SeqCst);
    if delayed {
        tokio::time::timeout(
            Duration::from_secs(30),
            state.release.wait_for(|released| *released),
        )
        .await
        .unwrap()
        .unwrap();
    }
    axum::Json(
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":if delayed { "LATE_WINDOWS_RESULT" } else { "FRESH_WINDOWS_RESULT" }}]}]}),
    )
}

struct ProviderServer(tokio::task::JoinHandle<()>);
impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_cancels_provider_work_without_losing_the_next_draft() -> Result<()> {
    use axum::{
        Json, Router,
        routing::{get, post},
    };
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::new()?;
    let (release, receiver) = tokio::sync::watch::channel(false);
    let started = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let config = sandbox.root.join("provider.toml");
    std::fs::write(
        &config,
        format!(
            "api_base='http://{}/v1'\napi_key_env='KURU_WINDOWS_FIXTURE_KEY'\nmax_rounds=1\n",
            listener.local_addr()?
        ),
    )?;
    let router = Router::new()
        .route(
            "/v1/models",
            get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
        )
        .route("/v1/responses", post(response))
        .with_state(ProviderState {
            started: started.clone(),
            release: receiver,
        });
    let _server = ProviderServer(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let mut terminal = sandbox.start(
        "cancellation",
        "app",
        &[
            "--model",
            "fixture",
            "--mode",
            "freudian",
            "--config",
            config.to_str().unwrap(),
        ],
        true,
        "responses",
        &[("KURU_WINDOWS_FIXTURE_KEY", "fixture")],
    )?;
    terminal.text(&["enter send"], sandbox.startup)?;
    terminal.send(b"Slow native request\r")?;
    terminal.wait("native provider started", READY, |_| {
        started.load(Ordering::SeqCst)
    })?;
    terminal.send(b"Next native thought")?;
    terminal.send(b"\x1b")?;
    terminal.text(&["Cancelled", "Next native thought", "enter send"], READY)?;
    release.send(true)?;
    terminal.send(b"\r")?;
    terminal.text(&["FRESH_WINDOWS_RESULT", "enter send"], READY)?;
    ensure!(
        !String::from_utf8_lossy(&terminal.output).contains("LATE_WINDOWS_RESULT"),
        "cancelled completion reached the native UI"
    );
    terminal.send(b"/quit\r")?;
    assert_eq!(terminal.finish(EXIT)?["status"], 0);
    drop(terminal);
    let sessions: Vec<kuru_runtime::Session> = serde_json::from_str(&sandbox.output("sessions")?)?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].turns, 1);
    Ok(())
}
