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
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[path = "support/memory.rs"]
mod memory;
#[path = "support/notice_https.rs"]
mod notice_https;
#[path = "support/windows_terminal.rs"]
mod terminal;
#[path = "support/windows_recall.rs"]
mod windows_recall;
use terminal::{EXIT, READY, Terminal};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

#[test]
fn empty_composer_waits_for_cursor_at_placeholder_origin() {
    for placeholder in [
        "What shall we explore or build?",
        "Keep your next thought here…",
    ] {
        let mut parser = vt100::Parser::new(6, 80, 0);
        parser.process(format!("\x1b[4;1H › {placeholder}\x1b[?25h").as_bytes());
        assert!(!terminal::composer_frame_ready(parser.screen(), ""));
        // Painting the placeholder leaves a visible cursor at its end. Only
        // the final composer cursor placement completes the native frame.
        parser.process(b"\x1b[4;4");
        assert!(!terminal::composer_frame_ready(parser.screen(), ""));
        parser.process(b"H");
        assert!(terminal::composer_frame_ready(parser.screen(), ""));
        parser.process(b"\x1b[5;4H");
        assert!(!terminal::composer_frame_ready(parser.screen(), ""));
    }
}

struct Sandbox {
    temporary: memory::ServiceCleanup,
    root: PathBuf,
    project: PathBuf,
    data: PathBuf,
    environment: BTreeMap<String, String>,
    startup: Duration,
}

impl Sandbox {
    /// A sandbox for a plain `fn` test: warms the shared cache synchronously.
    #[allow(dead_code, reason = "every current caller runs inside a Tokio runtime")]
    fn new() -> Result<Self> {
        Self::with_cache(&kuru_memory::test_support::cache_dir()?)
    }

    /// A sandbox for a test inside a Tokio runtime.
    async fn warmed() -> Result<Self> {
        Self::with_cache(&kuru_memory::test_support::warmed_cache_dir().await?)
    }

    fn with_cache(cache: &std::path::Path) -> Result<Self> {
        let temporary = kuru_memory::test_support::tempdir()?;
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
        let config = memory::configuration_with(&root, cache)?;
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
        // The owner open timeline gate travels with the coverage destination.
        for key in ["LLVM_PROFILE_FILE", "KURU_OPEN_TIMELINE"] {
            if let Ok(value) = std::env::var(key) {
                environment.insert(key.into(), value);
            }
        }
        let startup =
            Duration::from_secs((configuration.memory.startup_timeout_secs + 5) * 2 + 13) + READY;
        let temporary = memory::ServiceCleanup::new(temporary, &data);
        // A failing test shows the stderr of every owner this elects.
        environment.insert(
            kuru_memory::test_support::OWNER_DIAGNOSTIC_ENV.into(),
            temporary
                .owner_diagnostic_path()
                .to_string_lossy()
                .into_owned(),
        );
        Ok(Self {
            temporary,
            root,
            project,
            data,
            environment,
            startup,
        })
    }

    /// Release the fixture after the test's own `outcome`, so a cleanup
    /// failure or guard verdict is attached to that outcome instead of
    /// replacing it with a drop panic.
    fn release<T>(self, outcome: Result<T>) -> Result<T> {
        self.temporary.release(outcome)
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
    let sandbox = Sandbox::warmed().await?;
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
    // The next ambient frame is due within the frame allowance in `READY`.
    terminal.wait("ambient animation frame", READY, |terminal| {
        terminal.output.len() > animated
    })?;
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
    terminal.wait("resumed animation frame", READY, |terminal| {
        terminal.output.len() > resumed_frame
    })?;
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
    reopened.quiet_with_optional_style_reset(
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

    let sandbox = Sandbox::warmed().await?;
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

    let sandbox = Sandbox::warmed().await?;
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
            sandbox.startup,
            |_| false,
        )
        .unwrap_err();
    ensure!(error.to_string().contains("child exited"), "{error:#}");
    ensure!(
        String::from_utf8_lossy(&terminal.output)
            .contains("Responses API authentication environment variable is not set"),
        "approved launch did not reach its missing-key refusal: {}",
        terminal.screen()
    );
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
async fn native_conpty_wait_drains_the_line_written_just_before_exit() -> Result<()> {
    let _serial = SERIAL.lock().await;
    // The child prints one line and exits at once. A wait that sees the exit
    // must not drop that line from what the caller observes afterwards.
    let temporary = kuru_memory::test_support::tempdir()?;
    let system = kuru_platform::windows::process::system_directory()?;
    let root = system.parent().context("System32 parent")?;
    let mut terminal = Terminal::spawn(
        &temporary.path().join("drain-on-exit"),
        json!({
            "mode":"app",
            "binary":system.join("cmd.exe"),
            // `echo` is a cmd.exe builtin, and the platform quotes every
            // argument, so the command is one argument that `/s` unwraps.
            "args":["/d", "/s", "/c", "echo DRAIN_ON_EXIT_MARKER"],
            "environment":{"SystemRoot":root},
            "cwd":temporary.path(),
        }),
        38,
        130,
    )?;
    let error = terminal
        .wait("child prints its marker and exits", READY, |_| false)
        .unwrap_err();
    ensure!(error.to_string().contains("child exited"), "{error:#}");
    ensure!(
        error.to_string().contains("output EOF reached=true"),
        "{error:#}"
    );
    ensure!(
        String::from_utf8_lossy(&terminal.output).contains("DRAIN_ON_EXIT_MARKER"),
        "the line written just before exit was dropped: {error:#}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_console_modes_restore_after_partial_initialization_and_errors() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::warmed().await?;
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
    let sandbox = Sandbox::warmed().await?;
    let mut terminal = sandbox.start("drop-error", "partial-error", &[], true, "demo", &[])?;
    // Observe the exit without the wait's drain, which closes the console:
    // this test wraps the live master below to observe the drop's closure.
    terminal
        .wait_exited(READY)
        .context("expected the real console fixture's exit")?;
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
) -> impl axum::response::IntoResponse {
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
    let completed = json!({"id":"fixture-cancellation","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":if delayed { "LATE_WINDOWS_RESULT" } else { "FRESH_WINDOWS_RESULT" }}]}]});
    let event = json!({"type":"response.completed","response":completed});
    (
        [("content-type", "text/event-stream")],
        format!("data: {event}\n\n"),
    )
}

struct ProviderServer(tokio::task::JoinHandle<()>);
impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_session_controls_share_cli_identity_and_public_history() -> Result<()> {
    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use crossterm::event::{KeyCode, KeyModifiers};

    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::warmed().await?;
    let outcome = async {
        let cli_action = |args: &[&str]| -> Result<Value> {
            let output = BlockingCommand::new(env!("CARGO_BIN_EXE_kuru"))
                .args(sandbox.args("demo"))
                .args(["-c", "max_rounds=1"])
                .args(args)
                .env_clear()
                .envs(&sandbox.environment)
                .output()?;
            ensure!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(serde_json::from_slice(&output.stdout)?)
        };
        let created = cli_action(&["run", "NATIVE-SESSION-ORIGIN", "--json"])?;
        let source = created["session"]
            .as_str()
            .context("CLI source session missing")?
            .to_owned();
        ensure!(cli_action(&["sessions", "rename", &source, "CLI-LABEL"])?["session_id"] == source);
        let catalog = cli_action(&["sessions"])?;
        let source_record = catalog
            .as_array()
            .context("CLI catalog is not an array")?
            .iter()
            .find(|row| row["id"] == source)
            .context("CLI source is absent")?;
        let source_node = source_record["head_node_id"]
            .as_str()
            .context("source head missing")?
            .to_owned();

        // Command dispatch must not run an inference turn. Unexpected requests
        // are counted and fail closed, while the real model catalog remains
        // available to the application's ordinary provider initialization.
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        let router = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }),
            )
            .route(
                "/v1/responses",
                post(move || {
                    let counted = Arc::clone(&counted);
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::SERVICE_UNAVAILABLE
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = sandbox.root.join("session-controls.toml");
        std::fs::write(
            &config,
            format!(
                "api_base='http://{}/v1'\napi_key_env='KURU_NATIVE_CONTROL_KEY'\nmax_rounds=1\n",
                listener.local_addr()?
            ),
        )?;
        let _server = ProviderServer(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let config_path = config
            .to_str()
            .context("fixture config path is not UTF-8")?;
        let mut terminal = sandbox.start(
            "session-controls",
            "app",
            &[
                "--model",
                "fixture",
                "--mode",
                "freudian",
                "--config",
                config_path,
                "--resume",
                &source,
            ],
            true,
            "responses",
            &[("KURU_NATIVE_CONTROL_KEY", "fixture")],
        )?;
        terminal.frame_text(&["NATIVE-SESSION-ORIGIN", "enter send"], sandbox.startup)?;
        let submit = |terminal: &mut Terminal, command: &str| -> Result<()> {
            terminal.frame_text(&["enter send"], READY)?;
            terminal.committed_text(command)?;
            terminal.composer(command)?;
            terminal.committed_key(KeyCode::Enter, KeyModifiers::NONE)
        };
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
            submit(&mut terminal, command)?;
            terminal.frame_text(&[result, "enter send"], READY)?;
            ensure!(
                !terminal.screen().contains("Unknown command"),
                "registered command was not dispatched: {command}"
            );
        }

        submit(
            &mut terminal,
            &format!("/session-rename {source} NATIVE-LABEL"),
        )?;
        terminal.text(&["Sessions", "NATIVE-LABEL"], READY)?;
        terminal.committed_key(KeyCode::Esc, KeyModifiers::NONE)?;
        terminal.frame_text(&["enter send"], READY)?;
        let catalog = cli_action(&["sessions"])?;
        ensure!(
            catalog
                .as_array()
                .context("renamed catalog is not an array")?
                .iter()
                .any(|row| row["id"] == source && row["label"] == "NATIVE-LABEL")
        );

        submit(&mut terminal, &format!("/session-boundaries {source}"))?;
        terminal.text(&["Settled boundaries"], READY)?;
        terminal.committed_key(KeyCode::Enter, KeyModifiers::NONE)?;
        terminal.frame_text(
            &[
                "current project memory remains",
                "shared",
                "NATIVE-SESSION-ORIGIN",
                "enter send",
            ],
            READY,
        )?;
        let catalog = cli_action(&["sessions"])?;
        let child = catalog
            .as_array()
            .context("forked catalog is not an array")?
            .iter()
            .find(|row| {
                row["id"] != source && row["fork_provenance"]["source_session_id"] == source
            })
            .context("native fork is missing from CLI catalog")?;
        ensure!(child["fork_provenance"]["source_node_id"] == source_node);
        let child_id = child["id"]
            .as_str()
            .context("native child ID missing")?
            .to_owned();
        submit(&mut terminal, "/status")?;
        terminal.frame_text(&[&format!("Session: {child_id}"), "enter send"], READY)?;

        // The child owns the active claim, so removing its source must preserve
        // the child's public history and refuse a direct resume until restore.
        submit(&mut terminal, &format!("/session-remove {source}"))?;
        terminal.text(&["Sessions", "NATIVE-LABEL", "removed"], READY)?;
        terminal.committed_key(KeyCode::Esc, KeyModifiers::NONE)?;
        terminal.frame_text(&["enter send"], READY)?;
        ensure!(
            cli_action(&["sessions"])?
                .as_array()
                .context("active catalog is not an array")?
                .iter()
                .all(|row| row["id"] != source)
        );
        submit(&mut terminal, &format!("/resume {source}"))?;
        terminal.frame_text(
            &[
                "session is removed; restore it before resuming",
                "enter send",
            ],
            READY,
        )?;
        // Clear only visible history so an earlier child status cannot satisfy
        // this check of the active claim after the refused source resume.
        submit(&mut terminal, "/clear")?;
        terminal.frame_text(
            &["View cleared · stored history unchanged", "enter send"],
            READY,
        )?;
        submit(&mut terminal, "/status")?;
        terminal.frame_text(&[&format!("Session: {child_id}"), "enter send"], READY)?;
        submit(&mut terminal, &format!("/session-restore {source}"))?;
        terminal.text(&["Sessions", "NATIVE-LABEL"], READY)?;
        terminal.committed_key(KeyCode::Esc, KeyModifiers::NONE)?;
        terminal.frame_text(&["enter send"], READY)?;
        submit(&mut terminal, &format!("/resume {source}"))?;
        terminal.frame_text(&["NATIVE-SESSION-ORIGIN", "enter send"], READY)?;
        submit(&mut terminal, "/status")?;
        terminal.frame_text(&[&format!("Session: {source}"), "enter send"], READY)?;
        submit(&mut terminal, "/export native-session-export.md")?;
        terminal.frame_text(&["Exported public session", "enter send"], READY)?;
        let exported = std::fs::read_to_string(sandbox.project.join("native-session-export.md"))?;
        ensure!(exported.contains("NATIVE-SESSION-ORIGIN"));
        ensure!(
            requests.load(Ordering::SeqCst) == 0,
            "session controls dispatched an inference turn"
        );
        submit(&mut terminal, "/quit")?;
        ensure!(terminal.finish(EXIT)?["status"] == 0);
        drop(terminal);

        let mut reopened = sandbox.start(
            "session-controls-reopened",
            "app",
            &[
                "--model",
                "fixture",
                "--mode",
                "freudian",
                "--config",
                config_path,
                "--resume",
                &child_id,
            ],
            true,
            "responses",
            &[("KURU_NATIVE_CONTROL_KEY", "fixture")],
        )?;
        reopened.frame_text(&["NATIVE-SESSION-ORIGIN", "enter send"], sandbox.startup)?;
        submit(&mut reopened, "/status")?;
        reopened.frame_text(&[&format!("Session: {child_id}"), "enter send"], READY)?;
        submit(&mut reopened, "/sessions")?;
        reopened.text(&["Sessions", "NATIVE-LABEL", &child_id], READY)?;
        reopened.committed_key(KeyCode::Esc, KeyModifiers::NONE)?;
        reopened.frame_text(&["enter send"], READY)?;
        let catalog = cli_action(&["sessions"])?;
        ensure!(
            catalog
                .as_array()
                .context("reopened catalog is not an array")?
                .iter()
                .any(|row| row["id"] == source && row["label"] == "NATIVE-LABEL")
        );
        submit(&mut reopened, "/quit")?;
        ensure!(reopened.finish(EXIT)?["status"] == 0);
        drop(reopened);
        ensure!(requests.load(Ordering::SeqCst) == 0);
        Ok(())
    }
    .await;
    sandbox.release(outcome)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_cancels_provider_work_without_losing_the_next_draft() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::warmed().await?;
    let outcome = cancel_provider_work_and_keep_the_next_draft(&sandbox).await;
    sandbox.release(outcome)
}

async fn cancel_provider_work_and_keep_the_next_draft(sandbox: &Sandbox) -> Result<()> {
    use axum::{
        Json, Router,
        routing::{get, post},
    };
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

fn native_notice_start(
    sandbox: &Sandbox,
    peer: &notice_https::NoticeHttps,
    label: &str,
    path: &str,
) -> Result<Terminal> {
    let endpoint = format!("{}{path}", peer.base);
    let ca = peer.ca.to_str().context("fixture CA path")?;
    sandbox.start(
        label,
        "app",
        &["-c", "update.notice=true"],
        true,
        "demo",
        &[
            ("KURU_TEST_UPDATE_NOTICE_ENDPOINT", &endpoint),
            ("KURU_TEST_UPDATE_NOTICE_CA_PEM", ca),
            ("NO_PROXY", "localhost,127.0.0.1"),
        ],
    )
}

fn native_notice_cache(sandbox: &Sandbox) -> Result<Value> {
    let bytes = std::fs::read(sandbox.data.join("update/notice.json"))?;
    ensure!(bytes.len() <= 4096, "notice cache exceeded its limit");
    Ok(serde_json::from_slice(&bytes)?)
}

fn finish_native_notice(terminal: &mut Terminal, advice: bool) -> Result<()> {
    terminal.send(b"/quit\r")?;
    let report = terminal.finish(EXIT)?;
    ensure!(report["status"] == 0, "{report}");
    let output = String::from_utf8_lossy(&terminal.output);
    ensure!(
        output.matches("Kuru 999.0.0 is available").count() == usize::from(advice),
        "unexpected native notice output: {output}"
    );
    ensure!(
        !output.contains("PRIVATE_HOSTILE") && !output.contains("PRIVATE_ERROR_BODY"),
        "raw release response reached terminal output"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_notice_https_bounds_failure_cache_and_console_restoration() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::warmed().await?;
    let outcome = async {
        let peer = notice_https::NoticeHttps::start(&sandbox.root).await?;
        let cache = sandbox.data.join("update/notice.json");
        for (index, (path, failure)) in [
            ("/redirect", None),
            ("/malformed", Some("malformed")),
            ("/oversize", Some("malformed")),
            ("/loop", Some("offline")),
            ("/downgrade", Some("offline")),
            ("/unavailable", Some("http")),
        ]
        .into_iter()
        .enumerate()
        {
            if cache.exists() {
                std::fs::remove_file(&cache)?;
            }
            let before = peer.requests().len();
            let mut terminal =
                native_notice_start(&sandbox, &peer, &format!("notice-{index}"), path)?;
            terminal.resize(30, 80)?;
            terminal.text(&["enter send"], sandbox.startup)?;
            terminal.wait("native notice cache published", READY, |_| cache.is_file())?;
            let stored = native_notice_cache(&sandbox)?;
            match failure {
                Some(failure) => ensure!(
                    stored["outcome"] == "failed"
                        && stored["failure"] == failure
                        && stored["latest"].is_null(),
                    "unexpected failed cache for {path}: {stored}; requests={}; transport={:?}",
                    peer.requests().len(),
                    peer.failures()
                ),
                None => ensure!(
                    stored["outcome"] == "newer" && stored["latest"] == "999.0.0",
                    "unexpected successful cache for {path}: {stored}; requests={}; transport={:?}",
                    peer.requests().len(),
                    peer.failures()
                ),
            }
            finish_native_notice(&mut terminal, failure.is_none())?;
            let after = peer.requests().len();
            ensure!(after > before && after <= before + 6);
            if failure.is_none() {
                ensure!(after == before + 2, "HTTPS redirect was not followed");
            }
            // A fresh native process must honor both successful and failed
            // cache entries without sending another release request.
            let mut cached =
                native_notice_start(&sandbox, &peer, &format!("cached-{index}"), path)?;
            cached.text(&["enter send"], sandbox.startup)?;
            finish_native_notice(&mut cached, failure.is_none())?;
            ensure!(peer.requests().len() == after, "native cache was ignored");
        }
        for request in peer.requests() {
            let lower = request.to_ascii_lowercase();
            ensure!(request.starts_with("GET "));
            ensure!(lower.contains("user-agent: kuru-update-notice\r\n"));
            for forbidden in [
                "authorization:",
                "cookie:",
                "x-api-key:",
                "project",
                "session",
                "provider",
            ] {
                ensure!(
                    !lower.contains(forbidden),
                    "identifying notice request field"
                );
            }
        }
        peer.close().await;
        Ok(())
    }
    .await;
    sandbox.release(outcome)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_notice_quit_releases_held_https_response_without_cache() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::warmed().await?;
    let outcome = async {
        let peer = notice_https::NoticeHttps::start(&sandbox.root).await?;
        let mut terminal = native_notice_start(&sandbox, &peer, "notice-held", "/held")?;
        terminal.text(&["enter send"], sandbox.startup)?;
        tokio::time::timeout(READY, peer.held.notified())
            .await
            .with_context(|| {
                format!(
                    "held notice request absent; requests={}; transport={:?}",
                    peer.requests().len(),
                    peer.failures()
                )
            })?;
        finish_native_notice(&mut terminal, false)?;
        tokio::time::timeout(READY, peer.disconnected.notified())
            .await
            .with_context(|| {
                format!(
                    "held notice response remained open; requests={}; transport={:?}",
                    peer.requests().len(),
                    peer.failures()
                )
            })?;
        ensure!(!sandbox.data.join("update/notice.json").exists());
        ensure!(peer.requests().len() == 1);
        peer.close().await;
        Ok(())
    }
    .await;
    sandbox.release(outcome)
}

async fn native_permission_response(
    axum::Json(request): axum::Json<Value>,
) -> impl axum::response::IntoResponse {
    let speaking = request["instructions"]
        .as_str()
        .is_some_and(|text| text.contains("Phase: speak and act"));
    let continued = request["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    let output = if speaking && !continued {
        json!([{"type":"function_call","call_id":"native-permission","name":"file_write","arguments":json!({"path":"literal[1].txt","content":"approved"}).to_string()}])
    } else {
        json!([{"type":"message","content":[{"type":"output_text","text":"NATIVE_PERMISSION_FINAL"}]}])
    };
    let event = json!({"type":"response.completed","response":{"id":"native-permission","status":"completed","output":output,"usage":{"input_tokens":8,"output_tokens":5}}});
    (
        [("content-type", "text/event-stream")],
        format!("data: {event}\n\n"),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_permission_choices_preserve_exact_scope_and_revoke_authority() -> Result<()>
{
    use axum::{
        Json, Router,
        routing::{get, post},
    };
    let _serial = SERIAL.lock().await;
    for (choice, granted, remembered) in [
        (b'1', true, false),
        (b'2', true, true),
        (b'3', true, true),
        (b'4', false, false),
    ] {
        let sandbox = Sandbox::warmed().await?;
        let outcome = async {
            let marker = sandbox.project.join("literal[1].txt");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let config = sandbox.root.join("native-permission.toml");
            std::fs::write(&config, format!("api_base='http://{}/v1'\napi_key_env='KURU_NATIVE_PERMISSION_KEY'\nmax_rounds=3\n", listener.local_addr()?))?;
            let app = Router::new()
                .route("/v1/models", get(|| async { Json(json!({"data":[{"id":"fixture"}]})) }))
                .route("/v1/responses", post(native_permission_response));
            let _server = ProviderServer(tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); }));
            let mut terminal = sandbox.start(
                "native-permission", "app", &["--model", "fixture", "--mode", "freudian", "--config", config.to_str().context("provider fixture path")?],
                true, "responses", &[("KURU_NATIVE_PERMISSION_KEY", "fixture")],
            )?;
            terminal.resize(30, if choice % 2 == 0 { 80 } else { 120 })?;
            terminal.frame_text(&["enter send"], sandbox.startup)?;
            terminal.send(b"Native permission turn")?;
            terminal.composer("Native permission turn")?;
            terminal.send(b"\r")?;
            terminal.frame_text(&["Permission request", "literal[1].txt", "1 once", "esc cancel"], READY)?;
            ensure!(!marker.exists(), "native effect preceded a permission choice");
            terminal.send(&[choice])?;
            terminal.wait("native permission choice settled", READY, |terminal| {
                !terminal.screen().contains("Permission request") && terminal.screen().contains("NATIVE_PERMISSION_FINAL") && terminal.screen().contains("enter send")
            })?;
            terminal.frame_text(&["NATIVE_PERMISSION_FINAL", "enter send"], READY)?;
            ensure!(marker.exists() == granted, "unexpected native permission effect");
            if granted { ensure!(std::fs::read(&marker)? == b"approved"); }
            terminal.send(b"/permissions\r")?;
            if remembered {
                terminal.frame_text(&["Selected exact scope", "literal[1].txt"], READY)?;
                terminal.send(b"\x1b[3~")?;
            }
            terminal.frame_text(&["No session or always grants"], READY)?;
            terminal.send(b"\x1b")?;
            terminal.wait("native permission inspector closed", READY, |terminal| !terminal.screen().contains("Permissions · ↑↓ select"))?;
            terminal.frame_text(&["enter send"], READY)?;
            if granted { std::fs::remove_file(&marker)?; }
            terminal.send(b"Verify revoked native authority")?;
            terminal.composer("Verify revoked native authority")?;
            terminal.send(b"\r")?;
            terminal.frame_text(&["Permission request", "literal[1].txt"], READY)?;
            ensure!(!marker.exists(), "revoked native grant still authorized the write");
            terminal.send(b"4")?;
            terminal.wait("native repeat denied and settled", READY, |terminal| {
                !terminal.screen().contains("Permission request") && terminal.screen().contains("enter send")
            })?;
            terminal.frame_text(&["enter send"], READY)?;
            ensure!(!marker.exists());
            terminal.send(b"/quit\r")?;
            ensure!(terminal.finish(EXIT)?["status"] == 0);
            Ok(())
        }.await;
        sandbox.release(outcome)?;
    }
    Ok(())
}
