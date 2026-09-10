#![cfg(all(windows, feature = "test-support"))]
#![forbid(unsafe_code)]

use anyhow::{Context, Result, ensure};
use kuru_core::{Config, Mode};
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

    fn config(&self) -> Result<Config> {
        Ok(toml::from_str(&self.output("config")?)?)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conpty_chat_selectors_resize_focus_and_persistent_choices() -> Result<()> {
    let _serial = SERIAL.lock().await;
    let sandbox = Sandbox::new()?;
    let mut terminal =
        sandbox.start("first", "app", &["--mode", "freudian"], false, "demo", &[])?;
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
    let settled = terminal.output.len();
    terminal.read_for(Duration::from_millis(450))?;
    ensure!(
        terminal.output.len() == settled,
        "native focus loss continued drawing after its completed composer frame"
    );
    terminal.focus(true)?;
    terminal.send(&[127; 11])?;
    terminal.text(&["What shall we explore"], READY)?;
    terminal.command("hello from native Windows")?;
    terminal.text(&["demo"], READY)?;
    for (keys, label) in [
        (b"\x1bOQ".as_slice(), "Models"),
        (b"\x1bOR".as_slice(), "Efforts"),
        (b"\x1bOS".as_slice(), "Modes"),
    ] {
        terminal.send(keys)?;
        terminal.text(&[label, "Esc back"], READY)?;
        terminal.send(b"\x1b")?;
        terminal.wait("picker dismissed", READY, |terminal| {
            !terminal.screen().contains("Esc back")
        })?;
    }
    terminal.command("/mode jungian")?;
    terminal.command("/model persistent-demo")?;
    terminal.command("/effort high")?;
    terminal.resize(24, 80)?;
    terminal.send(b"navigation draft")?;
    terminal.composer("navigation draft")?;
    terminal.send(b"\x1b[H\x1b[3~")?;
    terminal.text(&["avigation draft"], READY)?;
    terminal.send(b"\x03")?;
    let report = terminal.finish(EXIT)?;
    assert_eq!(report["status"], 0);
    drop(terminal);
    let saved = sandbox.config()?;
    assert_eq!(
        (saved.mode, saved.model.as_str(), saved.effort.as_deref()),
        (Mode::Jungian, "persistent-demo", Some("high"))
    );
    let sessions: Vec<kuru_runtime::Session> = serde_json::from_str(&sandbox.output("sessions")?)?;
    ensure!(
        sessions
            .iter()
            .any(|session| session.label == "hello from native Windows" && session.turns == 1),
        "chat was not durably saved"
    );

    let mut reopened = sandbox.start("reopened", "app", &[], true, "demo", &[])?;
    reopened.text(
        &["persistent-demo", "jungian", "high", "enter send"],
        sandbox.startup,
    )?;
    reopened.send(b"reduced draft")?;
    reopened.composer("reduced draft")?;
    let settled = reopened.output.len();
    reopened.read_for(Duration::from_millis(450))?;
    ensure!(
        reopened.output.len() == settled,
        "reduced-motion console continued drawing"
    );
    reopened.send(b"\x03")?;
    assert_eq!(reopened.finish(EXIT)?["status"], 0);
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
