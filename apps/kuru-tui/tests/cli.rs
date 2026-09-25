use kuru_delivery::command::BlockingCommand as Command;
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Output,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[path = "support/memory.rs"]
mod memory;

#[path = "support/mcp_oauth_https.rs"]
mod mcp_oauth_https;
use mcp_oauth_https::HttpsMcpFixture;

#[cfg(unix)]
#[path = "support/terminal.rs"]
#[allow(
    dead_code,
    reason = "this CLI fixture uses only the owned PTY browser-login path"
)]
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
    // `#[tokio::test]` async tests already driving a Tokio runtime; the
    // shared helper is safe from either call site (see
    // `kuru_connectors::shell_warmup::block_on_dedicated_thread`).
    kuru_connectors::shell_warmup::ensure_stock_powershell_warm();
}

struct Sandbox {
    root: memory::ServiceCleanup,
    project: PathBuf,
    data: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        #[cfg(windows)]
        ensure_powershell_warm();
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project).unwrap();
        memory::configuration(root.path()).unwrap();
        Self {
            root: memory::ServiceCleanup::new(root, &data),
            project,
            data,
        }
    }
    fn command(&self) -> Command {
        self.command_for("demo")
    }
    fn command_for(&self, provider: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        #[cfg(windows)]
        command.fixture_allow_independent_service();
        command
            .arg("-C")
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.data)
            .args(["--provider", provider, "--no-dream"])
            .env("XDG_CONFIG_HOME", self.root.path().join("config"));
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_post_turn_failure_reports_separately_after_completed_json_answer() -> anyhow::Result<()>
{
    use kuru_memory::{MemoryStore, PublicTranscriptEntry, PublicTurnSettlement};
    use kuru_runtime::project_scope;

    let env = Sandbox::new();
    let marker = env.project.join("post-turn-hook-ran");
    let config_path = env.root.path().join("config/kuru/config.toml");
    let mut config = std::fs::read_to_string(&config_path)?;
    config.push_str(&format!(
        "\n[[hooks.post_turn]]\ncommand = '/bin/sh'\nargs = ['-c', 'cat >/dev/null; printf x > \"$1\"; printf RAW_HOOK_SECRET >&2; printf \"{{\"', 'hook', {}]\n",
        toml::Value::String(marker.to_string_lossy().into_owned())
    ));
    std::fs::write(config_path, config)?;

    // Retain a checked managed attachment across the CLI process boundary so
    // the completed public record can be read without a second cold start or
    // a direct local lock that excludes the CLI's own managed attachment.
    let scope = project_scope(&env.project)?;
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope)?;
    let (_, opening) = MemoryStore::open_managed_observed(
        options,
        std::fs::canonicalize(&env.project)?,
        PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
    );
    let memory = opening.await?;

    let output = tokio::task::block_in_place(|| {
        env.run(&["run", "a completed answer survives its post hook", "--json"])
    });
    assert!(
        output.status.success(),
        "settled answer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: Value = serde_json::from_slice(&output.stdout)?;
    assert!(answer["text"].as_str().is_some_and(|text| !text.is_empty()));
    let session = answer["session"].as_str().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("post-turn hook 1: failed"), "{stderr}");
    assert!(!stderr.contains("RAW_HOOK_SECRET"), "{stderr}");
    assert_eq!(std::fs::read(marker)?, b"x");

    let page = memory.public_transcript_page(session, None, 8).await?;
    assert!(page.records.iter().any(|entry| {
        matches!(entry, PublicTranscriptEntry::Turn { record }
            if record.origin_session_id == session
                && record.settlement == PublicTurnSettlement::Completed)
    }));
    memory.close().await?;
    Ok(())
}

#[test]
fn shell_support_generation_bypasses_invalid_workspace_authority_without_state() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("foreign-project");
    std::fs::create_dir(&project).unwrap();
    let invalid_config = project.join("invalid-config.toml");
    std::fs::write(&invalid_config, b"[").unwrap();
    let data = root.path().join("private-data");
    let user_config = root.path().join("user-config");

    for command in [
        &["completions", "bash"][..],
        &["completions", "zsh"],
        &["completions", "fish"],
        &["completions", "powershell"],
        &["man"],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kuru"));
        #[cfg(windows)]
        child.fixture_allow_independent_service();
        let output = child
            .arg("-C")
            .arg(&project)
            .arg("--config")
            .arg(&invalid_config)
            .arg("--data-dir")
            .arg(&data)
            .arg("--provider")
            .arg("responses")
            .env("XDG_CONFIG_HOME", &user_config)
            .args(command)
            .output()
            .unwrap();
        let repeated = child.output().unwrap();
        assert!(
            output.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty() && output.stdout.len() <= 512 * 1024);
        assert!(output.stderr.is_empty(), "{command:?}: {output:?}");
        assert!(repeated.status.success(), "{command:?}: {repeated:?}");
        assert_eq!(output.stdout, repeated.stdout, "{command:?} changed bytes");
        assert_eq!(output.stderr, repeated.stderr, "{command:?} changed stderr");
        if command == ["man"] {
            let manual = String::from_utf8(output.stdout).unwrap();
            assert!(manual.contains("MEMORY EXPORT"), "{manual}");
            assert!(!manual.contains("COMPLETE-WORD"), "{manual}");
        } else {
            let script = String::from_utf8(output.stdout).unwrap();
            assert!(
                script.contains("__complete_word__"),
                "{command:?}: {script}"
            );
            assert!(script.contains("kuru"), "{command:?}: {script}");
        }
        assert!(!data.exists(), "{command:?} created private state");
        assert!(!user_config.exists(), "{command:?} read user configuration");
    }
    assert_eq!(std::fs::read(&invalid_config).unwrap(), b"[");
}

#[test]
fn usage_completion_answer_and_external_option_stay_pure() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("foreign-project");
    std::fs::create_dir(&project).unwrap();
    let invalid_config = project.join("invalid-config.toml");
    std::fs::write(&invalid_config, b"[").unwrap();
    let data = root.path().join("private-data");

    for shell in ["bash", "zsh", "fish", "powershell"] {
        let mut answer = Command::new(env!("CARGO_BIN_EXE_kuru"));
        #[cfg(windows)]
        answer.fixture_allow_independent_service();
        let output = answer
            .current_dir(&project)
            .env("KURU_DATA_DIR", &data)
            .env("XDG_CONFIG_HOME", root.path().join("user-config"))
            .args(["__complete_word__", "--shell", shell, "--line", "kuru mem"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("memory"),
            "{shell}: {output:?}"
        );
        assert!(output.stderr.is_empty(), "{shell}: {output:?}");

        let mut external = Command::new(env!("CARGO_BIN_EXE_kuru"));
        #[cfg(windows)]
        external.fixture_allow_independent_service();
        let script = external
            .args(["completions", shell, "--external-usage"])
            .output()
            .unwrap();
        assert!(script.status.success(), "{shell}: {script:?}");
        let text = String::from_utf8(script.stdout).unwrap();
        assert!(
            text.contains("complete-word") && text.contains("usage"),
            "{shell}: {text}"
        );
    }
    assert!(!data.exists());
    assert_eq!(std::fs::read(invalid_config).unwrap(), b"[");
}

#[test]
#[ignore = "subprocess entry selected only by the MCP refusal fixture"]
fn mcp_stdio_marker_child() {
    let marker = std::env::var_os("KURU_TEST_MCP_STDIO_MARKER").unwrap();
    std::fs::write(marker, b"started").unwrap();
}

#[test]
fn mcp_cli_refuses_unselected_aliases_without_starting_stdio_or_memory() {
    let env = Sandbox::new();
    let marker = env.root.path().join("unrelated-stdio-started");
    let fixture = std::env::current_exe().unwrap();
    let config_path = env.root.path().join("config/kuru/config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap();
    config.push_str(&format!(
        r#"
[mcp.stdio]
command = {fixture}
args = ["--ignored", "--exact", "mcp_stdio_marker_child", "--nocapture"]
[mcp.stdio.env]
KURU_TEST_MCP_STDIO_MARKER = {marker}

[mcp.disabled]
enabled = false
url = "https://127.0.0.1:9/mcp"
[mcp.disabled.oauth]
enabled = true
client_id = "synthetic-native-client"

[mcp.selected]
url = "https://127.0.0.1:9/mcp"
[mcp.selected.oauth]
enabled = true
client_id = "synthetic-native-client"
"#,
        fixture = toml::Value::String(fixture.to_string_lossy().into_owned()),
        marker = toml::Value::String(marker.to_string_lossy().into_owned())
    ));
    std::fs::write(config_path, config).unwrap();

    let disabled: Value =
        serde_json::from_str(&env.success(&["mcp", "status", "disabled"])).unwrap();
    assert_eq!(disabled["state"], "disabled");
    assert!(!marker.exists(), "disabled status started an STDIO alias");
    let selected: Value =
        serde_json::from_str(&env.success(&["mcp", "status", "selected"])).unwrap();
    assert_eq!(selected["alias"], "selected");
    assert!(
        matches!(
            selected["state"].as_str(),
            Some("login_required" | "native_store_unavailable")
        ),
        "{selected}"
    );
    assert!(
        !marker.exists(),
        "selected status started an unrelated STDIO alias"
    );
    let logout: Value = serde_json::from_str(&env.success(&["mcp", "logout", "selected"])).unwrap();
    assert_eq!(logout["alias"], "selected");
    assert_eq!(logout["local_deleted"], false);
    assert_eq!(logout["remote"], "no_local_credential");
    assert!(!marker.exists(), "selected logout started unrelated STDIO");
    let refuses = |args: &[&str], expected: &str| {
        let output = env.run(args);
        assert!(!output.status.success(), "{args:?}: {output:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains(expected), "{args:?}: {diagnostic}");
    };

    for alias in ["missing", "stdio", "disabled"] {
        let expected = if alias == "stdio" {
            "selected MCP alias does not enable OAuth".to_owned()
        } else {
            format!("configured MCP alias {alias} is unavailable")
        };
        refuses(&["mcp", "login", alias, "--no-browser"], &expected);
        refuses(&["mcp", "logout", alias], &expected);
        assert!(
            !marker.exists(),
            "unselected STDIO alias started for {alias}"
        );
        assert!(!env.data.join("memory").exists());
    }

    for alias in ["missing", "stdio"] {
        let expected = if alias == "stdio" {
            "selected MCP alias does not enable OAuth".to_owned()
        } else {
            format!("configured MCP alias {alias} is unavailable")
        };
        refuses(&["mcp", "status", alias], &expected);
        assert!(
            !marker.exists(),
            "unselected STDIO alias started for {alias}"
        );
        assert!(!env.data.join("memory").exists());
    }
    assert!(!env.data.join("memory").exists());

    let local = env.project.join(".kuru");
    std::fs::create_dir(&local).unwrap();
    std::fs::write(
        local.join("config.toml"),
        "[mcp.automatic]\nurl = 'https://127.0.0.1:9/mcp'\n[mcp.automatic.oauth]\nenabled = true\nclient_id = 'synthetic-native-client'\n",
    )
    .unwrap();
    for args in [
        ["mcp", "status", "automatic"].as_slice(),
        ["mcp", "logout", "automatic"].as_slice(),
        ["mcp", "login", "automatic", "--no-browser"].as_slice(),
    ] {
        let output = env.run(args);
        assert!(!output.status.success(), "{args:?}: {output:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("workspace authority"),
            "{args:?}: {diagnostic}"
        );
        assert!(!marker.exists(), "{args:?} started unrelated STDIO");
        assert!(!env.data.join("memory").exists());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_cli_device_login_status_logout_uses_synthetic_verified_https() {
    let env = Sandbox::new();
    let server = HttpsMcpFixture::start(env.root.path()).await;
    let config = env.root.path().join("config/kuru/config.toml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(&format!("\n[mcp.secure]\nurl = {:?}\n[mcp.secure.oauth]\nenabled = true\nclient_id = 'synthetic-client'\nscopes = ['mcp.read']\n", format!("{}/mcp", server.base)));
    std::fs::write(config, text).unwrap();
    let root = server.ca_path.clone();
    let run = |args: &'static [&'static str]| {
        let mut command = env.command();
        command
            .env("KURU_TEST_MCP_CA_PEM", &root)
            .arg("--trust-workspace-once")
            .args(args);
        tokio::task::spawn_blocking(move || command.output().unwrap())
    };
    let login = run(&["mcp", "login", "secure", "--device"]).await.unwrap();
    assert!(
        login.status.success(),
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(String::from_utf8_lossy(&login.stdout).contains("Signed in to MCP secure"));
    assert_eq!(server.device_requests.load(Ordering::Relaxed), 1);
    assert_eq!(server.token_requests.load(Ordering::Relaxed), 1);
    let status = run(&["mcp", "status", "secure"]).await.unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["state"], "authorized");
    let logout = run(&["mcp", "logout", "secure"]).await.unwrap();
    assert!(
        logout.status.success(),
        "{}",
        String::from_utf8_lossy(&logout.stderr)
    );
    let value: Value = serde_json::from_slice(&logout.stdout).unwrap();
    assert_eq!(value["local_deleted"], true);
    assert_eq!(server.revocations.load(Ordering::Relaxed), 1);
    let status = run(&["mcp", "status", "secure"]).await.unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["state"], "login_required");

    // The same synthetic CA must not make a different DNS identity valid.
    let wrong_host = server.base.replace("localhost", "127.0.0.1");
    let config = env.root.path().join("config/kuru/config.toml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(&format!("\n[mcp.wrong_host]\nurl = {:?}\n[mcp.wrong_host.oauth]\nenabled = true\nclient_id = 'synthetic-client'\n", format!("{wrong_host}/mcp")));
    std::fs::write(config, text).unwrap();
    let rejected = run(&["mcp", "login", "wrong_host", "--device"])
        .await
        .unwrap();
    assert!(!rejected.status.success());
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        diagnostic.contains("certificate") || diagnostic.contains("TLS"),
        "hostname mismatch failed for an unrelated reason: {diagnostic}"
    );
    assert_eq!(server.device_requests.load(Ordering::Relaxed), 1);
}

#[cfg(target_os = "linux")]
async fn read_linux_child_pipe(
    pipe: impl tokio::io::AsyncRead + Unpin,
    limit: u64,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    let mut pipe = pipe.take(limit + 1);
    pipe.read_to_end(&mut output).await?;
    Ok(output)
}

#[cfg(target_os = "linux")]
async fn bounded_linux_cli_child(
    env: &Sandbox,
    ca_path: &Path,
    disconnected_bus: Option<(&str, &Path)>,
    args: &[&str],
    phase: &str,
) -> anyhow::Result<Output> {
    use anyhow::{Context, ensure};
    use std::{process::Stdio, time::Duration};

    // Match the existing installed-CLI fixture's concurrent, bounded pipe
    // capture and explicit kill/reap path. The 125s ceiling includes the
    // managed owner's existing 120s outer startup allowance.
    const CHILD_TIMEOUT: Duration = Duration::from_secs(125);
    const OUTPUT_LIMIT: u64 = 64 * 1024;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_kuru"));
    command
        .arg("-C")
        .arg(&env.project)
        .arg("--data-dir")
        .arg(&env.data)
        .args(["--provider", "demo", "--no-dream"])
        .env("XDG_CONFIG_HOME", env.root.path().join("config"))
        .env("KURU_TEST_MCP_CA_PEM", ca_path)
        .env("KURU_TEST_STATIC_HEADER", "Bearer synthetic-static-proof")
        .arg("--trust-workspace-once")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some((address, runtime)) = disconnected_bus {
        command
            .env("DBUS_SESSION_BUS_ADDRESS", address)
            .env("XDG_RUNTIME_DIR", runtime);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("start {phase} child"))?;
    let stdout = child.stdout.take().context("capture stdout")?;
    let stderr = child.stderr.take().context("capture stderr")?;
    let stdout = tokio::spawn(read_linux_child_pipe(stdout, OUTPUT_LIMIT));
    let stderr = tokio::spawn(read_linux_child_pipe(stderr, OUTPUT_LIMIT));
    let status = match tokio::time::timeout(CHILD_TIMEOUT, child.wait()).await {
        Ok(status) => status.with_context(|| format!("wait for {phase} child"))?,
        Err(_) => {
            child
                .kill()
                .await
                .with_context(|| format!("reap timed-out {phase} child"))?;
            let out = tokio::time::timeout(Duration::from_secs(5), stdout).await;
            let err = tokio::time::timeout(Duration::from_secs(5), stderr).await;
            anyhow::bail!(
                "{phase} child exceeded {CHILD_TIMEOUT:?}; stdout={out:?}; stderr={err:?}"
            );
        }
    };
    let stdout = tokio::time::timeout(Duration::from_secs(5), stdout)
        .await
        .with_context(|| format!("{phase} stdout did not close"))???;
    let stderr = tokio::time::timeout(Duration::from_secs(5), stderr)
        .await
        .with_context(|| format!("{phase} stderr did not close"))???;
    ensure!(
        stdout.len() as u64 <= OUTPUT_LIMIT && stderr.len() as u64 <= OUTPUT_LIMIT,
        "{phase} child output exceeded {OUTPUT_LIMIT} bytes"
    );
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn mcp_cli_missing_secret_service_refuses_without_fallback_or_static_alias_loss()
-> anyhow::Result<()> {
    use anyhow::ensure;
    let env = Sandbox::new();
    let server = HttpsMcpFixture::start(env.root.path()).await;
    let config = env.root.path().join("config/kuru/config.toml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(&format!(
        "\n[mcp.secure]\nurl = {:?}\n[mcp.secure.oauth]\nenabled = true\nclient_id = 'synthetic-client'\nscopes = ['mcp.read']\n\n[mcp.static]\nurl = {:?}\n[mcp.static.header_env]\nAuthorization = 'KURU_TEST_STATIC_HEADER'\n",
        format!("{}/mcp", server.base),
        format!("{}/static-mcp", server.base)
    ));
    std::fs::write(config, text).unwrap();

    // CI supplies a real owned D-Bus Secret Service. Only these children get
    // the missing-bus address; the runner's bus and native collection remain.
    let absent_runtime = env.root.path().join("absent-bus-runtime");
    kuru_platform::fs::Directory::ensure_private(&absent_runtime).unwrap();
    let absent_bus = format!(
        "unix:path={}",
        absent_runtime.join("missing-session-bus").display()
    );
    let run = |args: &'static [&'static str], disconnected: bool, phase: &'static str| {
        bounded_linux_cli_child(
            &env,
            &server.ca_path,
            disconnected.then_some((absent_bus.as_str(), absent_runtime.as_path())),
            args,
            phase,
        )
    };

    let proof: anyhow::Result<()> = async {
        let login = run(&["mcp", "login", "secure", "--device"], false, "seed login").await?;
        ensure!(
            login.status.success(),
            "real Secret Service seed failed: {}",
            String::from_utf8_lossy(&login.stderr)
        );
        ensure!(server.device_requests.load(Ordering::Relaxed) == 1);
        ensure!(server.token_requests.load(Ordering::Relaxed) == 1);
        let baseline = run(&["mcp", "status", "secure"], false, "baseline status").await?;
        ensure!(baseline.status.success(), "{baseline:?}");
        let baseline: Value = serde_json::from_slice(&baseline.stdout)?;
        ensure!(baseline["state"] == "authorized", "{baseline}");

        let unavailable = run(&["mcp", "status", "secure"], true, "missing-bus status").await?;
        ensure!(unavailable.status.success(), "{unavailable:?}");
        let unavailable: Value = serde_json::from_slice(&unavailable.stdout)?;
        ensure!(
            unavailable["state"] == "native_store_unavailable",
            "{unavailable}"
        );
        ensure!(
            unavailable["diagnostic"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "missing bus produced no actionable status: {unavailable}"
        );
        let refused = run(
            &["mcp", "login", "secure", "--device"],
            true,
            "missing-bus login",
        )
        .await?;
        ensure!(!refused.status.success(), "missing bus admitted login");
        let diagnostic = String::from_utf8_lossy(&refused.stderr);
        ensure!(
            diagnostic.contains("native") || diagnostic.contains("credential"),
            "missing bus failed for an unrelated reason: {diagnostic}"
        );
        ensure!(server.device_requests.load(Ordering::Relaxed) == 1);
        ensure!(server.token_requests.load(Ordering::Relaxed) == 1);

        let catalog = run(&["tools"], true, "missing-bus static catalog").await?;
        ensure!(catalog.status.success(), "{catalog:?}");
        let catalog: Value = serde_json::from_slice(&catalog.stdout)?;
        ensure!(catalog["tools"].as_array().is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool["description"]
                    .as_str()
                    .is_some_and(|text| text.contains("MCP static/static-proof"))
            })
        }));
        ensure!(server.static_requests.load(Ordering::Relaxed) > 0);
        ensure!(server.static_bad_headers.load(Ordering::Relaxed) == 0);

        // Broken-bus children must not read a fallback or change the record.
        let restored = run(&["mcp", "status", "secure"], false, "restored status").await?;
        ensure!(restored.status.success(), "{restored:?}");
        let restored: Value = serde_json::from_slice(&restored.stdout)?;
        ensure!(restored["state"] == "authorized", "{restored}");
        Ok(())
    }
    .await;

    // Always attempt exact native cleanup, including when an assertion or a
    // bounded child fails. A failed cleanup is reported with the proof error.
    let cleanup = run(&["mcp", "logout", "secure"], false, "native logout").await;
    if let Err(error) = proof {
        anyhow::bail!("missing-bus proof failed: {error:#}; native cleanup: {cleanup:?}");
    }
    let logout = cleanup?;
    ensure!(logout.status.success(), "{logout:?}");
    let logout: Value = serde_json::from_slice(&logout.stdout)?;
    ensure!(logout["local_deleted"] == true, "{logout}");
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn mcp_cli_no_browser_prints_local_callback_guidance_and_settles_once() {
    let env = Sandbox::new();
    let server = HttpsMcpFixture::start(env.root.path()).await;
    let config = env.root.path().join("config/kuru/config.toml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(&format!(
        "\n[mcp.browser]\nurl = {:?}\n[mcp.browser.oauth]\nenabled = true\nclient_id = 'synthetic-client'\nscopes = ['mcp.read']\n",
        format!("{}/mcp", server.base)
    ));
    std::fs::write(config, text).unwrap();

    let mut command = env.command();
    command.env("KURU_TEST_MCP_CA_PEM", &server.ca_path).args([
        "--trust-workspace-once",
        "mcp",
        "login",
        "browser",
        "--no-browser",
    ]);
    let (url_send, url_receive) = std::sync::mpsc::sync_channel(1);
    let child = std::thread::spawn(move || -> anyhow::Result<String> {
        let mut terminal = terminal::Terminal::spawn(command, 24, 120)?;
        terminal.wait(
            "no-browser URL and forwarded-loopback guidance",
            std::time::Duration::from_secs(15),
            |terminal| {
                let output = String::from_utf8_lossy(&terminal.output);
                Ok(output.contains("Sign in to MCP browser:")
                    && output
                        .contains("use same-host browsing or forward the printed loopback port"))
            },
        )?;
        let output = String::from_utf8_lossy(&terminal.output);
        let url = output
            .split_whitespace()
            .find(|word| word.starts_with("https://localhost:") && word.contains("/authorize?"))
            .ok_or_else(|| anyhow::anyhow!("CLI did not print its authorization URL"))?
            .to_owned();
        url_send.send(url).unwrap();
        terminal.wait_exit(std::time::Duration::from_secs(15))?;
        Ok(String::from_utf8_lossy(&terminal.output).into_owned())
    });
    let authorization = tokio::task::spawn_blocking(move || {
        url_receive.recv_timeout(std::time::Duration::from_secs(20))
    })
    .await
    .unwrap()
    .unwrap();
    let authorization = reqwest::Url::parse(&authorization).unwrap();
    let parameters = authorization
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let redirect = parameters
        .get("redirect_uri")
        .expect("authorization omitted redirect_uri");
    let state = parameters
        .get("state")
        .expect("authorization omitted state");
    let mut callback = reqwest::Url::parse(redirect).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "synthetic-callback-code")
        .append_pair("state", state)
        .append_pair("iss", &format!("{}/", server.base));
    // Model a browser on another machine through a local TCP forwarder. Keep
    // the owner's advertised callback authority in the HTTP request itself.
    let advertised = reqwest::Url::parse(redirect).unwrap();
    assert_eq!(advertised.scheme(), "http");
    assert_eq!(advertised.host_str(), Some("127.0.0.1"));
    assert_eq!(callback.scheme(), advertised.scheme());
    assert_eq!(callback.host_str(), advertised.host_str());
    assert_eq!(callback.port(), advertised.port());
    assert_eq!(callback.path(), advertised.path());
    let owner_port = advertised.port().unwrap();
    let forwarder = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forwarded_port = forwarder.local_addr().unwrap().port();
    assert_ne!(forwarded_port, owner_port);
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_by_forwarder = Arc::clone(&accepted);
    let forward_task = tokio::spawn(async move {
        let (mut browser, _) =
            tokio::time::timeout(std::time::Duration::from_secs(5), forwarder.accept())
                .await
                .unwrap()
                .unwrap();
        accepted_by_forwarder.fetch_add(1, Ordering::Relaxed);
        let mut owner = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::net::TcpStream::connect(("127.0.0.1", owner_port)),
        )
        .await
        .unwrap()
        .unwrap();
        let _ = tokio::io::copy_bidirectional(&mut browser, &mut owner).await;
    });
    let mut forwarded = callback.clone();
    forwarded.set_port(Some(forwarded_port)).unwrap();
    assert_eq!(forwarded.path(), advertised.path());
    assert_eq!(forwarded.query(), callback.query());
    let response = reqwest::Client::new()
        .get(forwarded)
        .header(reqwest::header::HOST, format!("127.0.0.1:{owner_port}"))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "forwarded callback refused: {}",
        response.status()
    );
    let _ = response.bytes().await.unwrap();
    forward_task.abort();
    let _ = forward_task.await;
    assert_eq!(accepted.load(Ordering::Relaxed), 1);
    let output = child.join().unwrap().unwrap();
    assert!(output.contains("Signed in to MCP browser."), "{output}");
    assert_eq!(server.device_requests.load(Ordering::Relaxed), 0);
    assert_eq!(server.token_requests.load(Ordering::Relaxed), 1);
    let forms = server.token_forms();
    assert_eq!(forms.len(), 1);
    let form = &forms[0];
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(
        form.get("code").map(String::as_str),
        Some("synthetic-callback-code")
    );
    assert_eq!(
        form.get("redirect_uri").map(String::as_str),
        Some(redirect.as_str())
    );
    assert_eq!(
        parameters.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    let verifier = form
        .get("code_verifier")
        .expect("token exchange omitted PKCE verifier");
    use base64::Engine;
    use sha2::Digest;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    assert_eq!(parameters.get("code_challenge"), Some(&challenge));
}

#[test]
fn failed_service_cleanup_retains_the_fixture_at_its_original_path() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().to_path_buf();
    let data = original.join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("memory"), b"not a fixture memory directory").unwrap();
    let mut cleanup = memory::ServiceCleanup::new(root, &data);

    let error = cleanup.finish().unwrap_err();
    assert!(
        format!("{error:#}").contains("fixture root retained in place"),
        "{error:#}"
    );
    assert!(original.is_dir());
    assert!(data.join("memory").is_file());

    drop(cleanup);
    std::fs::remove_dir_all(original).unwrap();

    let root = tempfile::tempdir().unwrap();
    let original = root.path().to_path_buf();
    let data = original.join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("memory"), b"drop must fail the test").unwrap();
    let panic = std::panic::catch_unwind(|| {
        let _cleanup = memory::ServiceCleanup::new(root, &data);
    })
    .unwrap_err();
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap();
    assert!(message.contains("managed-memory fixture cleanup failed"));
    assert!(original.is_dir());
    std::fs::remove_dir_all(original).unwrap();

    let root = tempfile::tempdir().unwrap();
    let original = root.path().to_path_buf();
    let data = original.join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("memory"), b"second invalid memory root").unwrap();
    let panic = std::panic::catch_unwind(|| {
        let _cleanup = memory::ServiceCleanup::new(root, &data);
        panic!("original fixture assertion");
    })
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"original fixture assertion")
    );
    assert!(original.is_dir());
    assert!(data.join("memory").is_file());
    std::fs::remove_dir_all(original).unwrap();
}

fn assert_memory_progress(stderr: &str) {
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(
        lines.first(),
        Some(&"Memory: waiting for project ownership…"),
        "{stderr}"
    );
    let ready = lines
        .iter()
        .position(|line| *line == "Memory: ready.")
        .expect("memory startup omitted ready progress");
    let mut previous = 0;
    for stage in [
        "Memory: waiting for verified runtime cache…",
        "Memory: extracting embedded runtime…",
        "Memory: verifying cached runtime…",
        "Memory: checking runtime version…",
        "Memory: preparing database…",
        "Memory: opening database…",
    ] {
        if let Some(position) = lines.iter().position(|line| *line == stage) {
            assert!(position > previous && position < ready, "{stderr}");
            previous = position;
        }
    }
}

#[tokio::test]
async fn candidate_commands_discover_and_abandon_one_exact_retained_ref() {
    let env = Sandbox::new();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope).unwrap();
    let memory = kuru_memory::test_support::open_fixture(options.clone())
        .await
        .unwrap();
    let candidate = memory
        .begin_candidate("retained CLI candidate")
        .await
        .unwrap();
    let branch = candidate.branch().to_owned();
    let base = candidate.base().to_owned();
    let view = candidate.view();
    view.append("fixture/candidate", "user", "private CLI value")
        .await
        .unwrap();
    let head = view.revision().await.unwrap();
    drop(view);
    drop(candidate);
    memory.close().await.unwrap();

    let diagnostic_path = env.data.join("candidate-owner-diagnostic.log");
    let private = kuru_platform::fs::Directory::open(
        &env.data,
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Pinned,
    )
    .unwrap();
    let diagnostic = private
        .create_new(std::ffi::OsStr::new("candidate-owner-diagnostic.log"))
        .unwrap();
    let owner = kuru_memory::test_support::spawn_logged_owner(
        &options,
        &env.project.canonicalize().unwrap(),
        Path::new(env!("CARGO_BIN_EXE_kuru")),
        diagnostic,
    )
    .await
    .unwrap();

    let run_json = |args: &[&str]| -> anyhow::Result<Value> {
        let output = env.run(args);
        anyhow::ensure!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    };
    let commands = (|| -> anyhow::Result<(Output, Value, Value, Value, Value)> {
        Ok((
            env.run(&[
                "memory",
                "candidate-abandon",
                &branch,
                "--base",
                &base,
                "--head",
                &base,
            ]),
            run_json(&["memory", "candidates", "--limit", "1"])?,
            run_json(&["memory", "candidate-status", &branch])?,
            run_json(&[
                "memory",
                "candidate-abandon",
                &branch,
                "--base",
                &base,
                "--head",
                &head,
            ])?,
            run_json(&["memory", "candidate-status", &branch])?,
        ))
    })();
    let controlled = if commands.is_ok() {
        // Prove the actual owner's private stderr capture only after the
        // original CLI sequence; no extra client can race its selected action.
        async {
            let (_, opening) = kuru_memory::MemoryStore::open_managed_observed(
                options.clone(),
                env.project.canonicalize()?,
                PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
            );
            let diagnostic_client = opening.await?;
            let result = diagnostic_client
                .abandon_candidate_ref(&branch, &base, &head)
                .await;
            let closed = diagnostic_client.close().await;
            closed?;
            anyhow::ensure!(
                matches!(
                    result
                        .err()
                        .as_ref()
                        .and_then(|error| error.downcast_ref::<kuru_memory::CandidateRefRejected>())
                        .map(|rejected| rejected.0),
                    Some(kuru_memory::CandidateRefRefusal::Changed)
                ),
                "controlled missing-ref request did not return a definite changed refusal"
            );
            Ok::<(), anyhow::Error>(())
        }
        .await
    } else {
        Ok(())
    };
    let retirement = kuru_memory::test_support::retire_idle_service(&options).await;
    let owner_exit = owner.wait_for_exit().await;
    let (rejected, inventory, status, abandoned, missing) = commands.unwrap_or_else(|error| {
        panic!(
            "{error:#}; {}",
            candidate_owner_diagnostic_records(&diagnostic_path)
        )
    });
    retirement.unwrap();
    owner_exit.unwrap();
    controlled.unwrap();
    let diagnostics = candidate_owner_diagnostic_records(&diagnostic_path);
    assert!(
        diagnostics.contains(
            "candidate_owner stage=ref_inspection class=non_sql sqlstate=none vendor=0 reason=other fault=ref_rejected"
        ),
        "the actual owner did not report the controlled changed-head refusal: {diagnostics}"
    );

    assert!(!rejected.status.success());
    let rejection = String::from_utf8(rejected.stderr).unwrap();
    assert!(
        rejection.contains("selected candidate ref changed or its outcome is unproved"),
        "{rejection}"
    );
    assert!(!rejection.contains("retained request identity"));
    assert!(!rejection.contains("outcome recovery"));
    assert_eq!(inventory["candidates"][0]["branch"], branch);
    assert_eq!(inventory["candidates"][0]["base"], base);
    assert_eq!(inventory["candidates"][0]["head"], head);
    assert_eq!(inventory["candidates"][0]["state"], "open_unchanged");

    assert_eq!(status["candidate"]["branch"], branch);
    assert_eq!(status["candidate"]["base"], base);
    assert_eq!(status["candidate"]["head"], head);
    assert_eq!(status["candidate"]["state"], "open_unchanged");
    assert_eq!(status["operation_outcome"], "not_queried");

    assert_eq!(abandoned["branch"], branch);
    assert_eq!(abandoned["state"], "abandoned");

    assert_eq!(missing["candidate"]["state"], "missing");
    assert_eq!(missing["operation_outcome"], "unproved");
}

fn candidate_owner_diagnostic_records(path: &Path) -> String {
    let mut bytes = Vec::new();
    let Ok(file) = std::fs::File::open(path) else {
        return "candidate owner diagnostic unavailable".into();
    };
    if file.take(4097).read_to_end(&mut bytes).is_err() || bytes.len() > 4096 {
        return "candidate owner diagnostic unreadable".into();
    }
    let Ok(contents) = std::str::from_utf8(&bytes) else {
        return "candidate owner diagnostic invalid".into();
    };
    let records = contents
        .lines()
        .filter(|line| {
            let mut fields = line.split_ascii_whitespace();
            let Some("candidate_owner") = fields.next() else {
                return false;
            };
            let Some(stage) = fields.next().and_then(|field| field.strip_prefix("stage=")) else {
                return false;
            };
            let Some(class) = fields.next().and_then(|field| field.strip_prefix("class=")) else {
                return false;
            };
            let Some(state) = fields
                .next()
                .and_then(|field| field.strip_prefix("sqlstate="))
            else {
                return false;
            };
            let Some(vendor) = fields
                .next()
                .and_then(|field| field.strip_prefix("vendor="))
            else {
                return false;
            };
            let Some(reason) = fields
                .next()
                .and_then(|field| field.strip_prefix("reason="))
            else {
                return false;
            };
            let Some(fault) = fields.next().and_then(|field| field.strip_prefix("fault=")) else {
                return false;
            };
            fields.next().is_none()
                && matches!(
                    stage,
                    "ref_inspection"
                        | "schema_validation"
                        | "pool_retirement"
                        | "working_set_inspection"
                        | "branch_rename"
                        | "outcome_reconciliation"
                        | "cleanup"
                        | "main_merge"
                )
                && matches!(class, "non_sql" | "sqlx" | "database")
                && (matches!(state, "none" | "other")
                    || (state.len() == 5
                        && state
                            .bytes()
                            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())))
                && vendor.parse::<u16>().is_ok()
                && matches!(reason, "branch_in_use" | "other")
                && matches!(
                    fault,
                    "ref_rejected"
                        | "candidate_conflict"
                        | "receipt_conflict"
                        | "storage_failed"
                        | "generation_changed"
                )
        })
        .take(8)
        .collect::<Vec<_>>();
    if records.is_empty() {
        "candidate owner diagnostic absent".into()
    } else {
        format!("candidate owner diagnostics: {}", records.join(" | "))
    }
}

fn git_fixture(directory: &std::path::Path, args: &[&str]) -> Output {
    // Git exports repository selectors into hooks. Fixture setup must target
    // its own temporary repository even when tests run from pre-push.
    let mut command = std::process::Command::new("git");
    command.args(args).current_dir(directory);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command.output().unwrap()
}

#[test]
fn tools_command_projects_the_shared_catalog_without_starting_memory_or_a_provider() {
    let env = Sandbox::new();
    kuru_platform::fs::Directory::ensure_private(&env.data).unwrap();
    std::fs::write(env.data.join("memory"), b"hostile memory path").unwrap();
    let catalog: Value = serde_json::from_str(&env.success(&["tools"])).unwrap();
    assert!(
        catalog["tools"]
            .as_array()
            .is_some_and(|tools| { tools.iter().any(|tool| tool["name"] == "file_read") })
    );
    assert_eq!(catalog["mcp"], serde_json::json!([]));
    assert_eq!(
        std::fs::read(env.data.join("memory")).unwrap(),
        b"hostile memory path",
        "catalog inspection must not inspect or initialize project memory"
    );
    std::fs::remove_file(env.data.join("memory")).unwrap();
}

#[test]
fn imported_project_instructions_require_exact_workspace_review_before_demo_dispatch() {
    let env = Sandbox::new();
    std::fs::create_dir_all(env.project.join("docs")).unwrap();
    std::fs::write(env.project.join("AGENTS.md"), "AGENT-ONLY\n").unwrap();
    std::fs::write(env.project.join("docs/rules.md"), "IMPORTED-ONLY\n").unwrap();
    std::fs::write(
        env.project.join("CLAUDE.md"),
        "@AGENTS.md\n@docs/rules.md\nCLAUDE-ONLY\n",
    )
    .unwrap();
    let status = env.success(&["trust", "status"]);
    assert!(status.contains("3 ordered automatic sources"), "{status}");
    assert!(status.contains("AGENTS.md"));
    assert!(status.contains("CLAUDE.md"));
    assert!(status.contains("rules.md"));
    let denied = env.run(&["run", "hello"]);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("workspace authority"));
    assert!(
        !env.data.exists(),
        "preflight must precede memory/provider effects"
    );
    let once = env.run(&["--trust-workspace-once", "run", "hello"]);
    assert!(
        once.status.success(),
        "{}",
        String::from_utf8_lossy(&once.stderr)
    );
    env.success(&["trust", "approve", "--yes"]);
    assert!(env.run(&["run", "hello again"]).status.success());
    std::fs::write(env.project.join("docs/rules.md"), "CHANGED-IMPORT\n").unwrap();
    let stale = env.run(&["run", "after change"]);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("workspace authority"));
}

#[test]
fn oversized_instruction_is_reported_before_usable_project_dispatch() {
    let env = Sandbox::new();
    std::fs::write(env.project.join("AGENTS.md"), "X".repeat(256 * 1024 + 1)).unwrap();
    std::fs::write(env.project.join("CLAUDE.md"), "USEFUL-INSTRUCTION\n").unwrap();
    let status = env.run(&["trust", "status"]);
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stderr).contains("256 KiB file limit"));
    assert!(String::from_utf8_lossy(&status.stdout).contains("1 ordered automatic source"));
    let run = env.run(&["--trust-workspace-once", "run", "hello"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stderr).contains("256 KiB file limit"));
}

#[test]
fn cli_supports_all_modes_model_discovery_persistent_sessions_and_dreaming() {
    let env = Sandbox::new();
    assert!(env.success(&["--help"]).contains("peer"));
    assert_eq!(
        env.success(&["--version"]),
        concat!("kuru ", env!("CARGO_PKG_VERSION"), "\n")
    );
    let models: Value = serde_json::from_str(&env.success(&["models"])).unwrap();
    assert_eq!(models[0]["id"], "demo");
    assert!(env.success(&["config"]).contains("provider = \"demo\""));
    for mode in ["ifs", "polyvagal", "freudian", "jungian"] {
        let result: Value = serde_json::from_str(&env.success(&[
            "--mode",
            mode,
            "run",
            "Remember the colour emerald",
            "--json",
        ]))
        .unwrap();
        assert_eq!(result.as_object().unwrap().len(), 10);
        for field in [
            "session",
            "speaker",
            "text",
            "relationship",
            "input_tokens",
            "output_tokens",
            "limited",
            "limit_reasons",
            "response_outcome",
            "events",
        ] {
            assert!(
                result.get(field).is_some(),
                "missing TurnOutput field {field}"
            );
        }
        assert!(result["text"].as_str().unwrap().contains("demo"));
        let session = result["session"].as_str().unwrap();
        let resumed: Value =
            serde_json::from_str(&env.success(&["--resume", session, "run", "Continue", "--json"]))
                .unwrap();
        assert_eq!(resumed["session"], session);
        let dream: Value =
            serde_json::from_str(&env.success(&["--resume", session, "dream"])).unwrap();
        assert!(dream["summaries"].as_u64().unwrap() >= 3);
    }
    let sessions: Value = serde_json::from_str(&env.success(&["sessions"])).unwrap();
    assert!(sessions.as_array().unwrap().len() >= 4);
    assert!(
        !env.run(&["--resume", "missing", "run", "no"])
            .status
            .success()
    );
    assert!(!env.run(&["undo-dream"]).status.success());
    assert!(env.success(&["run", "plain answer"]).contains("demo"));
}

#[test]
fn discovered_local_config_requires_untracked_git_provenance_and_preserves_repo_claims() {
    let env = Sandbox::new();
    let git = git_fixture(&env.project, &["init", "--quiet"]);
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
    );
    let config_dir = env.project.join(".kuru");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "allow_shell=true\n[mcp.worker]\ncommand='repo-runner'",
    )
    .unwrap();
    let local = config_dir.join("config.local.toml");
    std::fs::write(&local, "mode='freudian'\nallow_shell=false").unwrap();
    let output = env.success(&["-c", "max_rounds=4", "config"]);
    assert!(output.contains("mode = \"freudian\""));
    assert!(output.contains("allow_shell = false"));
    assert!(output.contains("max_rounds = 4"));
    let status = env.success(&["trust", "status"]);
    assert!(status.contains("worker"), "{status}");
    assert!(!status.contains("shell enabled"), "{status}");
    let run = env.run(&["run", "hello"]);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("workspace authority"));

    let git = git_fixture(&env.project, &["add", "--", ".kuru/config.local.toml"]);
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
    );
    let rejected = env.run(&["config"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Git tracks"));
    let rejected_run = env.run(&["run", "hello"]);
    assert!(!rejected_run.status.success());
    assert!(String::from_utf8_lossy(&rejected_run.stderr).contains("Git tracks"));

    let foreign = env.root.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    let initialized = git_fixture(&foreign, &["init", "--quiet"]);
    assert!(initialized.status.success());
    std::fs::write(foreign.join("other.toml"), "unrelated=true").unwrap();
    let added = git_fixture(&foreign, &["add", "--", "other.toml"]);
    assert!(added.status.success());
    let redirected_index = env
        .command()
        .env("GIT_INDEX_FILE", foreign.join(".git/index"))
        .arg("config")
        .output()
        .unwrap();
    assert!(!redirected_index.status.success());
    assert!(String::from_utf8_lossy(&redirected_index.stderr).contains("Git tracks"));
    let redirected_directory = env
        .command()
        .env("GIT_DIR", foreign.join(".git"))
        .env("GIT_WORK_TREE", &env.project)
        .arg("config")
        .output()
        .unwrap();
    assert!(!redirected_directory.status.success());
    assert!(String::from_utf8_lossy(&redirected_directory.stderr).contains("Git tracks"));
}

#[test]
fn discovered_local_config_accepts_non_git_roots_and_rejects_ambiguous_index_status() {
    let env = Sandbox::new();
    let config_dir = env.project.join(".kuru");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.local.toml"), "mode='freudian'").unwrap();
    assert!(env.success(&["config"]).contains("mode = \"freudian\""));

    std::fs::create_dir(env.project.join(".git")).unwrap();
    let rejected = env.run(&["config"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("index status is ambiguous"));
}

#[test]
fn managed_locks_and_typed_overrides_apply_before_demo_dispatch() {
    let env = Sandbox::new();
    let git = git_fixture(&env.project, &["init", "--quiet"]);
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
    );
    let config_dir = env.project.join(".kuru");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.local.toml"),
        "mode='freudian'\nmax_rounds=3",
    )
    .unwrap();
    let managed = env.root.path().join("managed.toml");
    std::fs::write(
        &managed,
        "[defaults]\nmax_rounds=2\n[constraints]\nallow_shell=false\nmax_tool_calls=12\npermissions=[]",
    )
    .unwrap();
    let output = env
        .command()
        .env("KURU_MANAGED_CONFIG", &managed)
        .args([
            "-c",
            "max_rounds=4",
            "-c",
            "memory.offline=true",
            "-c",
            "mcp.fixture={command='runner',env={TOKEN='fake-secret'}}",
            "config",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = String::from_utf8(output.stdout).unwrap();
    assert!(config.contains("max_rounds = 4"));
    assert!(config.contains("mode = \"freudian\""));
    assert!(config.contains("offline = true"));
    assert!(config.contains("[redacted]"));
    assert!(!config.contains("fake-secret"));
    let run = env
        .command()
        .env("KURU_MANAGED_CONFIG", &managed)
        .args(["-c", "max_rounds=4", "run", "hello"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8(run.stdout).unwrap().contains("demo"));
    for args in [
        vec!["--allow-shell", "config"],
        vec!["-c", "max_tool_calls=5", "config"],
        vec!["-c", "unknown_setting=true", "config"],
        vec!["-c", "max_tool_calls='wrong'", "config"],
    ] {
        let output = env
            .command()
            .env("KURU_MANAGED_CONFIG", &managed)
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
    }
}

#[test]
fn cli_turn_id_reuses_only_the_exact_request_in_its_session() {
    let env = Sandbox::new();
    assert!(env.success(&["run", "--help"]).contains("--turn-id"));
    assert!(
        !env.run(&["dream", "--turn-id", "not-valid-here"])
            .status
            .success()
    );
    let prompt = "retain this exact scripted request";
    let turn_id = "scripted-turn-1";
    let first: Value =
        serde_json::from_str(&env.success(&["run", prompt, "--turn-id", turn_id, "--json"]))
            .unwrap();
    let session = first["session"].as_str().unwrap();
    let replay: Value = serde_json::from_str(&env.success(&[
        "--resume",
        session,
        "run",
        prompt,
        "--turn-id",
        turn_id,
        "--json",
    ]))
    .unwrap();
    assert_eq!(replay, first);

    let sessions: Value = serde_json::from_str(&env.success(&["sessions"])).unwrap();
    let resumed = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == session)
        .unwrap();
    assert_eq!(resumed["turns"], 1);

    let changed = env.run(&[
        "--resume",
        session,
        "run",
        "changed request",
        "--turn-id",
        turn_id,
        "--json",
    ]);
    assert!(!changed.status.success());
    assert!(
        String::from_utf8_lossy(&changed.stderr)
            .contains("turn ID is already associated with a different request")
    );
}

#[tokio::test]
async fn session_lifecycle_cli_is_provider_free_and_matches_resume_continue_and_export() {
    use kuru_memory::MemoryStore;
    use kuru_runtime::project_scope;

    let env = Sandbox::new();
    let scope = project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    let memory = MemoryStore::open(options).await.unwrap();
    memory
        .append(
            &format!("{scope}/ifs/identity/export-private/notes"),
            "note",
            "PRIVATE_NOTE_EXCLUDED_FROM_SESSION_EXPORT",
        )
        .await
        .unwrap();
    memory
        .put(
            &format!("{scope}/private/export-probe"),
            &serde_json::json!({"secret": "PRIVATE_STATE_EXCLUDED_FROM_SESSION_EXPORT"}),
        )
        .await
        .unwrap();
    memory.close().await.unwrap();
    let first: Value = serde_json::from_str(&env.success(&[
        "run",
        "create one public session boundary",
        "--json",
    ]))
    .unwrap();
    let source = first["session"].as_str().unwrap().to_owned();
    let sessions: Value = serde_json::from_str(&env.success(&["sessions"])).unwrap();
    let source_record = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == source)
        .unwrap();
    let source_node = source_record["head_node_id"].as_str().unwrap().to_owned();

    let provider_free = |args: &[&str]| {
        let output = env
            .command_for("responses")
            .env_remove("OPENAI_API_KEY")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let renamed: Value = serde_json::from_slice(
        &provider_free(&["sessions", "rename", &source, "durable source"]).stdout,
    )
    .unwrap();
    assert_eq!(renamed["session_id"], source);
    let child = "00000000-0000-4000-8000-000000000011";
    let forked: Value = serde_json::from_slice(
        &provider_free(&[
            "sessions",
            "fork",
            &source,
            &source_node,
            "--child-id",
            child,
            "--label",
            "durable fork",
        ])
        .stdout,
    )
    .unwrap();
    assert_eq!(forked["session_id"], child);
    let newest: Value = serde_json::from_str(&env.success(&[
        "run",
        "create a newer session that will be removed",
        "--json",
    ]))
    .unwrap();
    let newest_id = newest["session"].as_str().unwrap();
    let newest_removed: Value =
        serde_json::from_slice(&provider_free(&["sessions", "remove", newest_id]).stdout).unwrap();
    assert_eq!(newest_removed["lifecycle_state"], "removed");
    let removed: Value =
        serde_json::from_slice(&provider_free(&["sessions", "remove", &source]).stdout).unwrap();
    assert_eq!(removed["lifecycle_state"], "removed");
    let active: Value = serde_json::from_slice(&provider_free(&["sessions"]).stdout).unwrap();
    assert!(
        active
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != source)
    );
    assert!(
        active
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == child)
    );

    let refused = env.run(&["--resume", &source, "run", "must not dispatch"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("session is removed; restore it before resuming")
    );
    let missing = env.run(&[
        "--resume",
        "00000000-0000-4000-8000-000000000099",
        "run",
        "must not dispatch a missing session",
    ]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("session is absent"),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    let continued: Value = serde_json::from_str(&env.success(&[
        "--continue",
        "run",
        "append only to the latest active fork",
        "--json",
    ]))
    .unwrap();
    assert_eq!(continued["session"], child);

    let restored: Value =
        serde_json::from_slice(&provider_free(&["sessions", "restore", &source]).stdout).unwrap();
    assert_eq!(restored["lifecycle_state"], "active");
    let exact: Value = serde_json::from_str(&env.success(&[
        "--resume",
        &source,
        "run",
        "resume the restored exact source",
        "--json",
    ]))
    .unwrap();
    assert_eq!(exact["session"], source);

    let exported = env.root.path().join("fork.jsonl");
    std::fs::write(&exported, "selected output sentinel").unwrap();
    let export_path = exported.to_str().unwrap();
    let output = provider_free(&[
        "sessions",
        "export",
        child,
        "--format",
        "jsonl",
        "--output",
        export_path,
    ]);
    assert!(output.stdout.is_empty());
    let lines = std::fs::read_to_string(exported).unwrap();
    let records = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records[0]["kind"], "manifest");
    assert_eq!(records[0]["session"]["session_id"], child);
    assert!(records.iter().any(|record| {
        record["kind"] == "turn"
            && record["record"]["terminal_entries"]
                .to_string()
                .contains("demo")
    }));
    assert!(lines.contains("create one public session boundary"));
    assert!(lines.contains("append only to the latest active fork"));
    assert!(!lines.contains("resume the restored exact source"));
    assert!(!lines.contains("selected output sentinel"));
    assert!(!lines.contains("PRIVATE_NOTE_EXCLUDED_FROM_SESSION_EXPORT"));
    assert!(!lines.contains("PRIVATE_STATE_EXCLUDED_FROM_SESSION_EXPORT"));

    let markdown = env.root.path().join("fork.md");
    let markdown_path = markdown.to_str().unwrap();
    let output = provider_free(&[
        "sessions",
        "export",
        child,
        "--format",
        "markdown",
        "--output",
        markdown_path,
    ]);
    assert!(output.stdout.is_empty());
    let markdown = std::fs::read_to_string(markdown).unwrap();
    assert!(markdown.contains("# Kuru public session export"));
    let inherited = markdown.find("create one public session boundary").unwrap();
    let own = markdown
        .find("append only to the latest active fork")
        .unwrap();
    assert!(inherited < own, "Markdown transcript was not chronological");
    assert!(!markdown.contains("resume the restored exact source"));
    assert!(!markdown.contains("PRIVATE_NOTE_EXCLUDED_FROM_SESSION_EXPORT"));
    assert!(!markdown.contains("PRIVATE_STATE_EXCLUDED_FROM_SESSION_EXPORT"));

    let complete_memory = provider_free(&["memory", "export"]);
    let complete_memory = String::from_utf8(complete_memory.stdout).unwrap();
    assert!(complete_memory.contains("PRIVATE_NOTE_EXCLUDED_FROM_SESSION_EXPORT"));
    assert!(complete_memory.contains("PRIVATE_STATE_EXCLUDED_FROM_SESSION_EXPORT"));
}

#[tokio::test]
async fn session_export_keeps_legacy_speaker_and_turn_unknown_in_both_formats() {
    use kuru_core::{Message, Mode};
    use kuru_memory::{
        LEGACY_PREFIX_RECORD_FORMAT, LegacyTranscriptPrefix, MemoryStore,
        SESSION_CATALOG_RECORD_FORMAT, SessionCatalogRecord, SessionLifecycleState,
    };
    use kuru_runtime::project_scope;

    let env = Sandbox::new();
    let scope = project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    let session = "00000000-0000-4000-8000-000000000031";
    let namespace = format!("{scope}/transcript/{session}");
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    memory
        .append_session_message(
            &namespace,
            session,
            &Message::text("assistant", "legacy public answer"),
        )
        .await
        .unwrap();
    let window = memory
        .session_history_window_after(&namespace, session, 0, 16)
        .await
        .unwrap();
    let sequence = window.rows[0].sequence;
    let prefix = LegacyTranscriptPrefix {
        namespace,
        source_session_id: session.into(),
        source_revision: window.revision,
        first_sequence: sequence,
        through_sequence: sequence,
        row_count: 1,
        record_format: LEGACY_PREFIX_RECORD_FORMAT.into(),
    };
    memory.close().await.unwrap();
    kuru_memory::test_support::seed_public_session(
        options,
        &SessionCatalogRecord {
            session_id: session.into(),
            mode: Mode::Ifs,
            label: "legacy attribution".into(),
            created_order: 1,
            updated_order: 1,
            lifecycle_generation: 0,
            lifecycle_state: SessionLifecycleState::Active,
            head_node_id: None,
            pending_node_id: None,
            legacy_prefix: Some(prefix),
            fork_provenance: None,
            record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
        },
        &[],
    )
    .await
    .unwrap();

    for format in ["jsonl", "markdown"] {
        let path = env.root.path().join(format!("legacy.{format}"));
        let output = env
            .command_for("responses")
            .env_remove("OPENAI_API_KEY")
            .args([
                "sessions",
                "export",
                session,
                "--format",
                format,
                "--output",
                path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{format}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        let contents = std::fs::read_to_string(path).unwrap();
        let records = contents
            .lines()
            .filter(|line| line.starts_with('{'))
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 2, "{format}: {contents}");
        assert_eq!(records[0]["kind"], "manifest");
        assert_eq!(records[0]["session"]["session_id"], session);
        assert_eq!(records[0]["total_rows"], 1);
        assert_eq!(records[1]["kind"], "legacy");
        assert_eq!(records[1]["sequence"], sequence);
        assert_eq!(
            records[1]["message"],
            serde_json::to_value(Message::text("assistant", "legacy public answer")).unwrap()
        );
        assert!(records[1].get("speaker_id").is_none());
        assert!(records[1].get("turn_id").is_none());
    }
}

#[tokio::test]
async fn normal_cli_exports_large_parent_and_fork_in_complete_chronological_records() {
    use kuru_core::{Message, Mode};
    use kuru_memory::{
        MemoryStore, PUBLIC_TURN_RECORD_FORMAT, PublicTurnKind, PublicTurnRecord,
        PublicTurnSettlement, SESSION_CATALOG_RECORD_FORMAT, SessionCatalogRecord,
        SessionLifecycleState, SessionTurnCheckpoint, public_turn_node_id,
    };
    use kuru_runtime::project_scope;
    use std::io::{BufRead, BufReader};

    let env = Sandbox::new();
    let scope = project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    let parent = "00000000-0000-4000-8000-000000000021";
    let child = "00000000-0000-4000-8000-000000000022";
    let large_answer = "🪶".repeat(8_192);
    let mut turns = Vec::with_capacity(1_025);
    let mut predecessor = None;
    for index in 0..1_025 {
        let turn_id = format!("bulk-{index:04}");
        let node_id = public_turn_node_id(parent, &turn_id).unwrap();
        turns.push(PublicTurnRecord {
            node_id: node_id.clone(),
            origin_session_id: parent.into(),
            turn_id,
            kind: PublicTurnKind::Primary,
            continuation_of_node_id: None,
            predecessor_node_id: predecessor,
            settlement: PublicTurnSettlement::Completed,
            user_entry: Some(Message::text("user", format!("question-{index:04}"))),
            speaker_id: Some("fixture-speaker".into()),
            terminal_entries: vec![Message::text(
                "assistant",
                format!("answer-{index:04}:{large_answer}"),
            )],
            record_format: PUBLIC_TURN_RECORD_FORMAT.into(),
        });
        predecessor = Some(node_id);
    }
    let catalog = SessionCatalogRecord {
        session_id: parent.into(),
        mode: Mode::Ifs,
        label: "long parent".into(),
        created_order: 1,
        updated_order: 1,
        lifecycle_generation: 0,
        lifecycle_state: SessionLifecycleState::Active,
        head_node_id: predecessor,
        pending_node_id: None,
        legacy_prefix: None,
        fork_provenance: None,
        record_format: SESSION_CATALOG_RECORD_FORMAT.into(),
    };
    kuru_memory::test_support::seed_public_session(options.clone(), &catalog, &turns)
        .await
        .unwrap();
    drop(turns);

    let memory = MemoryStore::open(options).await.unwrap();
    memory
        .put(
            &format!("{scope}/private/export-probe"),
            &serde_json::json!("PRIVATE_STATE_EXCLUDED_FROM_LONG_EXPORT"),
        )
        .await
        .unwrap();
    memory
        .append(
            &format!("{scope}/ifs/identity/private/notes"),
            "note",
            "PRIVATE_NOTE_EXCLUDED_FROM_LONG_EXPORT",
        )
        .await
        .unwrap();
    let fork_node = public_turn_node_id(parent, "bulk-1023").unwrap();
    memory
        .fork_session(parent, 0, &fork_node, child, "long fork")
        .await
        .unwrap();
    let child_namespace = format!("{scope}/transcript/{child}");
    memory
        .checkpoint_session_turn(
            &child_namespace,
            child,
            &[Message::text("user", "CHILD-OWN-QUESTION")],
            &[],
            &SessionTurnCheckpoint::Admit {
                expected_generation: 0,
                turn_id: "child-own".into(),
                label: None,
                expected_transcript_rows: Some(0),
            },
        )
        .await
        .unwrap();
    memory
        .checkpoint_session_turn(
            &child_namespace,
            child,
            &[Message::text("assistant", "CHILD-OWN-ANSWER")],
            &[],
            &SessionTurnCheckpoint::Settle {
                expected_generation: 0,
                turn_id: "child-own".into(),
                settlement: PublicTurnSettlement::Completed,
                speaker_id: "fixture-speaker".into(),
            },
        )
        .await
        .unwrap();
    memory.close().await.unwrap();

    for (session, fork) in [(parent, false), (child, true)] {
        for format in ["jsonl", "markdown"] {
            let path = env.root.path().join(format!("{session}.{format}"));
            let output = env
                .command_for("responses")
                .env_remove("OPENAI_API_KEY")
                .args([
                    "sessions",
                    "export",
                    session,
                    "--format",
                    format,
                    "--output",
                    path.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{session} {format}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            assert!(
                std::fs::metadata(&path).unwrap().len() > 32 * 1024 * 1024,
                "{session} {format} export did not exceed the page-memory bound"
            );
            let mut manifest = false;
            let mut rows = 0;
            for line in BufReader::new(std::fs::File::open(&path).unwrap()).lines() {
                let line = line.unwrap();
                assert!(!line.contains("PRIVATE_STATE_EXCLUDED_FROM_LONG_EXPORT"));
                assert!(!line.contains("PRIVATE_NOTE_EXCLUDED_FROM_LONG_EXPORT"));
                if !line.starts_with('{') {
                    continue;
                }
                let record: Value = serde_json::from_str(&line).unwrap();
                match record["kind"].as_str().unwrap() {
                    "manifest" => {
                        assert!(!manifest);
                        assert_eq!(record["session"]["session_id"], session);
                        assert_eq!(record["total_rows"], 1_025);
                        if fork {
                            assert_eq!(
                                record["session"]["fork_provenance"]["source_session_id"],
                                parent
                            );
                            assert_eq!(
                                record["session"]["fork_provenance"]["source_node_id"],
                                fork_node
                            );
                        }
                        manifest = true;
                    }
                    "turn" => {
                        assert!(manifest);
                        let expected = if fork && rows == 1_024 {
                            "child-own".to_owned()
                        } else {
                            format!("bulk-{rows:04}")
                        };
                        assert_eq!(record["record"]["turn_id"], expected);
                        assert_eq!(record["record"]["speaker_id"], "fixture-speaker");
                        assert_eq!(record["record"]["settlement"], "completed");
                        let turn: PublicTurnRecord =
                            serde_json::from_value(record["record"].clone()).unwrap();
                        let (question, answer) = if fork && rows == 1_024 {
                            (
                                "CHILD-OWN-QUESTION".to_owned(),
                                "CHILD-OWN-ANSWER".to_owned(),
                            )
                        } else {
                            (
                                format!("question-{rows:04}"),
                                format!("answer-{rows:04}:{large_answer}"),
                            )
                        };
                        assert_eq!(turn.user_entry, Some(Message::text("user", question)));
                        assert_eq!(
                            turn.terminal_entries,
                            vec![Message::text("assistant", answer)]
                        );
                        assert_eq!(
                            turn.origin_session_id,
                            if fork && rows == 1_024 { child } else { parent }
                        );
                        rows += 1;
                    }
                    kind => panic!("unexpected {format} export record {kind}"),
                }
            }
            assert!(manifest);
            assert_eq!(rows, 1_025);
        }
    }
}

#[test]
fn cli_requires_explicit_project_purge_confirmation_and_removes_its_diagnostics_ring() {
    let env = Sandbox::new();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    let hash = scope.strip_prefix("project/").unwrap();
    env.success(&["--debug", "run", "create managed project memory"]);
    let ring = env.data.join("diagnostics").join(hash);
    assert!(ring.is_dir(), "debug run did not create the project ring");

    let mut help_command = env.command_for("responses");
    let help = help_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "purge", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--yes"));
    assert!(help.contains("history"));
    assert!(help.contains("legacy SQLite"));
    assert!(help.contains("exports"));
    assert!(help.contains("diagnostics"));
    assert!(help.contains("recorded remaining identities"));
    let mut refusal_command = env.command_for("responses");
    let refusal = refusal_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "purge"])
        .output()
        .unwrap();
    assert!(!refusal.status.success());
    assert!(refusal.stdout.is_empty());
    assert!(String::from_utf8_lossy(&refusal.stderr).contains("--yes"));
    assert!(ring.is_dir(), "refusal changed the diagnostics ring");

    let mut purge_command = env.command_for("responses");
    let output = purge_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "purge", "--yes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["project"], scope);
    assert_eq!(result["legacy_import_suppressed"], true);
    assert!(!ring.exists());

    let reopened = env.run(&["run", "fresh memory after purge", "--json"]);
    assert!(
        reopened.status.success(),
        "{}",
        String::from_utf8_lossy(&reopened.stderr)
    );
    assert!(serde_json::from_slice::<Value>(&reopened.stdout).is_ok());
    assert!(
        String::from_utf8_lossy(&reopened.stderr).contains("Memory is ready at"),
        "{}",
        String::from_utf8_lossy(&reopened.stderr)
    );
}

#[tokio::test]
async fn cli_project_purge_preserves_shared_legacy_export_engine_and_other_project() {
    use kuru_memory::MemoryStore;
    use std::io::Write;

    let env = Sandbox::new();
    let other_project = env.root.path().join("other-project");
    std::fs::create_dir(&other_project).unwrap();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    let other_scope = kuru_runtime::project_scope(&other_project).unwrap();

    kuru_platform::fs::Directory::ensure_private(&env.data).unwrap();
    let legacy_path = env.data.join("memory.sqlite3");
    let legacy = rusqlite::Connection::open(&legacy_path).unwrap();
    legacy
        .execute_batch(
            "PRAGMA application_id=1263882837;
             PRAGMA user_version=1;
             CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL);
             CREATE TABLE state (`key` TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
        )
        .unwrap();
    let selected_key = format!("{scope}/purge-acceptance");
    let other_key = format!("{other_scope}/purge-acceptance");
    legacy
        .execute(
            "INSERT INTO state VALUES (?1, ?2)",
            rusqlite::params![selected_key, r#"{"owner":"selected"}"#],
        )
        .unwrap();
    legacy
        .execute(
            "INSERT INTO state VALUES (?1, ?2)",
            rusqlite::params![other_key, r#"{"owner":"other"}"#],
        )
        .unwrap();
    drop(legacy);
    let legacy_before = std::fs::read(&legacy_path).unwrap();

    let selected_options =
        kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    let selected = MemoryStore::open(selected_options.clone()).await.unwrap();
    assert_eq!(
        selected.get(&selected_key).await.unwrap(),
        Some(serde_json::json!({"owner":"selected"}))
    );
    selected.close().await.unwrap();
    let other_options =
        kuru_memory::test_support::open_options(env.data.clone(), other_scope.clone()).unwrap();
    let other = MemoryStore::open(other_options.clone()).await.unwrap();
    assert_eq!(
        other.get(&other_key).await.unwrap(),
        Some(serde_json::json!({"owner":"other"}))
    );
    other.close().await.unwrap();

    let snapshots = env.data.join("memory/legacy");
    let mut snapshot_before: Vec<_> = std::fs::read_dir(&snapshots)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect();
    snapshot_before.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(snapshot_before.len(), 1, "legacy import snapshot drifted");

    let export_path = env.root.path().join("selected-memory.json");
    let mut export_command = env.command_for("responses");
    let export = export_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "export", "--output", "../selected-memory.json"])
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(export.stdout.is_empty());
    assert_memory_progress(&String::from_utf8_lossy(&export.stderr));
    let export_before = std::fs::read(&export_path).unwrap();

    let engine =
        kuru_platform::fs::Directory::ensure_private(&env.data.join("tools/dolt")).unwrap();
    engine
        .create_new(std::ffi::OsStr::new("purge-sentinel"))
        .unwrap()
        .write_all(b"shared engine cache survives")
        .unwrap();
    drop(engine);

    let mut purge_command = env.command_for("responses");
    let output = purge_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "purge", "--yes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["project"], scope);
    assert_eq!(result["legacy_import_suppressed"], true);

    assert_eq!(std::fs::read(&legacy_path).unwrap(), legacy_before);
    let mut snapshot_after: Vec<_> = std::fs::read_dir(&snapshots)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect();
    snapshot_after.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(snapshot_after, snapshot_before);
    assert_eq!(std::fs::read(&export_path).unwrap(), export_before);
    assert_eq!(
        std::fs::read(env.data.join("tools/dolt/purge-sentinel")).unwrap(),
        b"shared engine cache survives"
    );

    let other = MemoryStore::open(other_options).await.unwrap();
    assert_eq!(
        other.get(&other_key).await.unwrap(),
        Some(serde_json::json!({"owner":"other"}))
    );
    other.close().await.unwrap();
    let fresh = MemoryStore::open(selected_options).await.unwrap();
    assert_eq!(fresh.get(&selected_key).await.unwrap(), None);
    fresh.close().await.unwrap();
}

#[test]
fn cli_file_checkpoint_edit_undo_and_prune_survive_process_restart() {
    let env = Sandbox::new();
    assert!(!env.data.exists());
    assert_eq!(env.success(&["file", "list"]), "[]\n");
    assert!(!env.data.exists(), "inspection created private state");
    std::fs::write(env.project.join("note.txt"), "alpha\none\n").unwrap();
    let edited = env.success(&[
        "--allow-write",
        "tool",
        "file_edit",
        "--args",
        r#"{"path":"note.txt","hunks":[{"before":"alpha\n","old":"one","after":"\n","replacement":"un"}]}"#,
    ]);
    let id = edited
        .trim()
        .strip_prefix("file_edit completed; checkpoint ")
        .expect("file edit reports selected checkpoint");
    assert_eq!(
        std::fs::read_to_string(env.project.join("note.txt")).unwrap(),
        "alpha\nun\n"
    );
    let inspected: Value = serde_json::from_str(&env.success(&["file", "inspect", id])).unwrap();
    assert_eq!(inspected["id"], id);
    assert_eq!(inspected["path"], "note.txt");
    assert_eq!(inspected["state"], "applied");
    assert!(
        inspected.get("before").is_none(),
        "private snapshots leaked into inspection"
    );
    let malformed_config = env.root.path().join("malformed-file-config.toml");
    std::fs::write(&malformed_config, "[").unwrap();
    let independent = env
        .command()
        .arg("--config")
        .arg(&malformed_config)
        .args(["file", "inspect", id])
        .output()
        .unwrap();
    assert!(
        independent.status.success(),
        "checkpoint inspection activated unrelated config: {}",
        String::from_utf8_lossy(&independent.stderr)
    );
    let undone: Value =
        serde_json::from_str(&env.success(&["--allow-write", "file", "undo", id])).unwrap();
    assert_eq!(undone["effect"], "undo");
    assert_eq!(
        std::fs::read_to_string(env.project.join("note.txt")).unwrap(),
        "alpha\none\n"
    );
    let list: Value = serde_json::from_str(&env.success(&["file", "list"])).unwrap();
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|record| record["id"] == id)
    );
    assert!(env.success(&["file", "prune", id]).contains("pruned"));
    assert!(!env.run(&["file", "inspect", id]).status.success());
}

#[test]
fn cli_file_crud_and_shell_require_real_capabilities() {
    const TOOL_TOKEN: &str = "sk-proj-abcdefghijklmnop0123456789";
    const ORDINARY_CONTROL: &str = "ordinary-control-remains-exact";
    const MARKER: &str = "[REDACTED:recognized-secret]";
    let env = Sandbox::new();
    let tools: Value = serde_json::from_str(&env.success(&["tools"])).unwrap();
    let shell_args = if cfg!(windows) {
        r#"{"command":"[IO.File]::WriteAllText('shell-entered', 'entered'); [Console]::Write('shell-ok'); [IO.File]::WriteAllText('shell-completed', 'completed')"}"#
    } else {
        r#"{"command":"printf shell-ok"}"#
    };
    assert!(
        tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "file_read")
    );
    let args = r#"{"path":"notes.txt","content":"A real note"}"#;
    assert!(
        !env.run(&["tool", "file_write", "--args", args])
            .status
            .success()
    );
    env.success(&["--allow-write", "tool", "file_write", "--args", args]);
    assert_eq!(
        std::fs::read_to_string(env.project.join("notes.txt")).unwrap(),
        "A real note"
    );
    assert!(
        env.success(&["tool", "file_read", "--args", r#"{"path":"notes.txt"}"#])
            .contains("A real note")
    );
    let projected_source = format!("openai_api_key={TOOL_TOKEN}\n{ORDINARY_CONTROL}");
    let projected_path = env.project.join("projection.txt");
    std::fs::write(&projected_path, &projected_source).unwrap();
    let projected_identity =
        kuru_platform::fs::regular_file_info(&std::fs::File::open(&projected_path).unwrap())
            .unwrap()
            .identity;
    let projected = env.success(&[
        "tool",
        "file_read",
        "--args",
        r#"{"path":"projection.txt"}"#,
    ]);
    assert_eq!(
        projected,
        format!("openai_api_key={MARKER}\n{ORDINARY_CONTROL}\n")
    );
    assert_eq!(
        std::fs::read_to_string(&projected_path).unwrap(),
        projected_source
    );
    assert_eq!(
        kuru_platform::fs::regular_file_info(&std::fs::File::open(&projected_path).unwrap())
            .unwrap()
            .identity,
        projected_identity,
        "file_read replaced the source object"
    );
    let oversized_path = env.project.join("oversized-projection.txt");
    let oversized_source = format!(
        "HEAD openai_api_key={TOOL_TOKEN}\n{}:TAIL",
        "x".repeat(2 * 1024 * 1024),
    );
    std::fs::write(&oversized_path, &oversized_source).unwrap();
    let oversized_identity =
        kuru_platform::fs::regular_file_info(&std::fs::File::open(&oversized_path).unwrap())
            .unwrap()
            .identity;
    let oversized = env.success(&[
        "tool",
        "file_read",
        "--args",
        r#"{"path":"oversized-projection.txt"}"#,
    ]);
    let oversized = oversized.strip_suffix('\n').unwrap();
    let expected_head = format!("HEAD openai_api_key={MARKER}\n");
    assert!(oversized.len() <= 2 * 1024 * 1024);
    assert!(oversized.starts_with(&expected_head));
    assert!(oversized.ends_with(":TAIL"));
    assert!(oversized.contains("[truncated]"));
    assert!(!oversized.contains(TOOL_TOKEN));
    assert_eq!(
        std::fs::read_to_string(&oversized_path).unwrap(),
        oversized_source
    );
    assert_eq!(
        kuru_platform::fs::regular_file_info(&std::fs::File::open(&oversized_path).unwrap())
            .unwrap()
            .identity,
        oversized_identity,
        "oversized file_read replaced the source object"
    );
    assert!(
        env.success(&["tool", "file_list", "--args", r#"{"path":"."}"#])
            .contains("notes.txt")
    );
    env.success(&[
        "--allow-write",
        "tool",
        "file_delete",
        "--args",
        r#"{"path":"notes.txt"}"#,
    ]);
    assert!(!env.project.join("notes.txt").exists());
    assert!(
        !env.run(&["tool", "shell", "--args", shell_args])
            .status
            .success()
    );
    // A denied request must not reach the controlled shell source.
    #[cfg(windows)]
    for name in ["shell-entered", "shell-completed"] {
        assert!(!env.project.join(name).exists());
    }
    let output = env.run(&["--allow-shell", "tool", "shell", "--args", shell_args]);
    // Observe only this fixture's bounded markers after the actual CLI returns.
    // They distinguish source progress from captured pipe output, without a
    // second shell invocation or changing the original Console.Write call.
    #[cfg(windows)]
    let markers = ["shell-entered", "shell-completed"].map(|name| {
        use std::io::Read;
        std::fs::File::open(env.project.join(name)).and_then(|file| {
            let mut bytes = Vec::new();
            file.take(32).read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    });
    let diagnostic = format!(
        "status {}; stdout {:?}; stderr {:?}",
        output.status,
        String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(4096)]),
        String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(4096)]),
    );
    #[cfg(windows)]
    let diagnostic = format!("{diagnostic}; source entry/completion markers: {markers:?}");
    assert!(output.status.success(), "{diagnostic}");
    assert!(output.stderr.is_empty(), "{diagnostic}");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["success"], true, "{diagnostic}");
    assert_eq!(result["exit_code"], 0, "{diagnostic}");
    assert_eq!(result["stdout"], "shell-ok", "{diagnostic}");
    assert_eq!(result["stderr"], "", "{diagnostic}");
    let projected_shell_command = if cfg!(windows) {
        format!("[Console]::Write('{ORDINARY_CONTROL} openai_api_key={TOOL_TOKEN}')")
    } else {
        format!("printf '{ORDINARY_CONTROL} openai_api_key={TOOL_TOKEN}'")
    };
    let projected_shell_args = serde_json::json!({"command": projected_shell_command}).to_string();
    let projected_shell = env.run(&[
        "--allow-shell",
        "tool",
        "shell",
        "--args",
        &projected_shell_args,
    ]);
    let projected_diagnostic = format!(
        "status {}; stdout {:?}; stderr {:?}",
        projected_shell.status,
        String::from_utf8_lossy(&projected_shell.stdout[..projected_shell.stdout.len().min(4096)]),
        String::from_utf8_lossy(&projected_shell.stderr[..projected_shell.stderr.len().min(4096)]),
    );
    assert!(projected_shell.status.success(), "{projected_diagnostic}");
    assert!(projected_shell.stderr.is_empty(), "{projected_diagnostic}");
    let projected_shell: Value = serde_json::from_slice(&projected_shell.stdout).unwrap();
    let stdout = projected_shell["stdout"].as_str().unwrap();
    assert_eq!(projected_shell["success"], true, "{projected_diagnostic}");
    assert_eq!(projected_shell["exit_code"], 0, "{projected_diagnostic}");
    assert_eq!(projected_shell["stderr"], "", "{projected_diagnostic}");
    assert_eq!(
        stdout,
        format!("{ORDINARY_CONTROL} openai_api_key={MARKER}"),
        "{projected_diagnostic}"
    );
    #[cfg(windows)]
    {
        assert_eq!(markers[0].as_deref().unwrap(), b"entered");
        assert_eq!(markers[1].as_deref().unwrap(), b"completed");
    }
    assert!(
        !env.run(&["tool", "file_read", "--args", "bad-json"])
            .status
            .success()
    );
}

#[test]
fn cli_configuration_errors_and_nonterminal_start_are_actionable() {
    let env = Sandbox::new();
    for args in [
        vec!["--mode", "invalid", "config"],
        vec!["--provider", "missing", "run", "hello"],
        vec!["--config", "missing.toml", "config"],
        vec!["update"],
        vec!["serve", "--bind", "0.0.0.0:7437"],
    ] {
        assert!(!env.run(&args).status.success(), "{args:?}");
    }
    let output = env.run(&[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
    let nested = env.project.join("memory");
    let output = env
        .command()
        .arg("--data-dir")
        .arg(nested)
        .args(["run", "hello"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let config = env.root.path().join("local.toml");
    std::fs::write(
        &config,
        "mode = 'freudian'\nmodel = 'custom'\neffort = 'future-level'\n",
    )
    .unwrap();
    let output = env
        .command()
        .arg("--config")
        .arg(&config)
        .args([
            "--mode", "jungian", "--model", "demo", "--effort", "medium", "config",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("mode = \"jungian\""));
    assert!(text.contains("effort = \"medium\""));
}

#[test]
fn fresh_inspection_never_provisions_memory_and_history_is_read_only() {
    let env = Sandbox::new();
    let config_path = env.root.path().join("config/kuru/config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    // A deliberately unavailable explicit engine makes accidental provisioning observable.
    std::fs::write(
        &config_path,
        format!(
            "[memory]\noffline = true\ndolt_binary = {:?}\n",
            env.root.path().join("does-not-exist").to_str().unwrap()
        ),
    )
    .unwrap();
    for args in [
        vec!["config"],
        vec!["models"],
        vec!["tools"],
        vec!["sessions"],
    ] {
        env.success(&args);
        assert!(!env.data.exists(), "fresh {args:?} created memory state");
    }
    let output = env.run(&["memory", "status"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert!(!env.data.exists());
    let output = env.run(&["memory", "notes", "missing"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert!(
        !env.data.exists(),
        "fresh notes inspection created memory state"
    );
    let output = env.run(&["memory", "forget", "missing", "--note", "1"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert!(
        !env.data.exists(),
        "fresh selected-note deletion created memory state"
    );
    let output = env.run(&["memory", "export"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert!(
        !env.data.exists(),
        "fresh memory export created memory state"
    );
    std::fs::write(&config_path, config).unwrap();
    env.success(&["run", "Make a durable revision"]);
    let sessions = env.success(&["sessions"]);
    let status: Value = serde_json::from_str(&env.success(&["memory", "status"])).unwrap();
    let history: Value =
        serde_json::from_str(&env.success(&["memory", "history", "--limit", "2"])).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert!(
        history[0]["hash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
    assert_eq!(
        serde_json::from_str::<Value>(&env.success(&["memory", "status"])).unwrap(),
        status
    );
    assert_eq!(env.success(&["sessions"]), sessions);
    for limit in ["0", "1001"] {
        let output = env.run(&["memory", "history", "--limit", limit]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("between 1 and 1000"));
    }
}

#[tokio::test]
async fn memory_export_is_provider_free_and_publishes_one_committed_snapshot() {
    use kuru_memory::MemoryStore;
    use kuru_runtime::project_scope;

    let env = Sandbox::new();
    let scope = project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope).unwrap();
    let memory = MemoryStore::open(options).await.unwrap();
    memory
        .append(
            "export/notes",
            "dream",
            "literal fence ```\nremains content",
        )
        .await
        .unwrap();
    memory
        .put("export/unknown", &serde_json::json!({"future":[true, 7]}))
        .await
        .unwrap();
    let revision = memory.revision().await.unwrap();
    memory.close().await.unwrap();

    let mut json_command = env.command_for("responses");
    json_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "export"]);
    let output = json_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_memory_progress(&String::from_utf8_lossy(&output.stderr));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["manifest"]["snapshot"], "committed active main");
    assert_eq!(json["manifest"]["provenance"]["revision"], revision);
    assert_eq!(
        json["manifest"]["excludes"],
        serde_json::json!([
            "previous revisions",
            "candidate branches",
            "uncommitted working rows",
            "operations and schema tables"
        ])
    );
    let records = json["records"].as_array().unwrap();
    assert!(records.iter().any(|record| {
        record["kind"] == "message"
            && record["role"] == "dream"
            && record["content"] == "literal fence ```\nremains content"
    }));
    assert!(
        records
            .iter()
            .any(|record| { record["kind"] == "state" && record["key"] == "export/unknown" })
    );

    let output_path = env.root.path().join("committed-memory.md");
    let mut markdown_command = env.command_for("responses");
    markdown_command.env_remove("OPENAI_API_KEY").args([
        "memory",
        "export",
        "--format",
        "markdown",
        "--output",
        "../committed-memory.md",
    ]);
    let output = markdown_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_memory_progress(&String::from_utf8_lossy(&output.stderr));
    let markdown = std::fs::read_to_string(&output_path).unwrap();
    assert!(markdown.contains("# Kuru committed memory export"));
    assert!(markdown.contains("committed active main"));
    let markdown_values: Vec<Value> = markdown
        .split("```json\n")
        .skip(1)
        .map(|block| serde_json::from_str(block.split("\n```").next().unwrap()).unwrap())
        .collect();
    assert_eq!(markdown_values[0], json["manifest"]);
    assert_eq!(&markdown_values[1..], records.as_slice());
    let original = std::fs::read(&output_path).unwrap();
    let output = env.run(&[
        "memory",
        "export",
        "--format",
        "markdown",
        "--output",
        "../committed-memory.md",
    ]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Rejected publication"));
    assert_eq!(std::fs::read(&output_path).unwrap(), original);
}

#[tokio::test]
async fn malformed_export_fails_after_private_staging_without_publishing_a_partial_file() {
    use kuru_memory::MemoryStore;
    use kuru_runtime::project_scope;

    let env = Sandbox::new();
    let scope = project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope).unwrap();
    let memory = MemoryStore::open(options).await.unwrap();
    kuru_memory::test_support::commit_malformed_state(&memory, "export/malformed")
        .await
        .unwrap();
    memory.close().await.unwrap();

    let mut before: Vec<_> = std::fs::read_dir(env.root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    before.sort();
    let output_path = env.root.path().join("failed-memory.json");
    let mut command = env.command_for("responses");
    command.env_remove("OPENAI_API_KEY").args([
        "memory",
        "export",
        "--output",
        "../failed-memory.json",
    ]);
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid JSON"));
    assert!(!output_path.exists());
    let mut after: Vec<_> = std::fs::read_dir(env.root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    after.sort();
    assert_eq!(after, before, "failed export retained a staging directory");
}

#[tokio::test]
async fn notes_cli_reads_existing_modes_without_provider_or_legacy_import() {
    use kuru_core::{Framework, Mode, ProjectPreferences};
    use kuru_memory::MemoryStore;
    use kuru_runtime::{Topology, project_scope};

    let env = Sandbox::new();
    let scope = project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    let memory = MemoryStore::open(options).await.unwrap();
    let freudian = Topology {
        parts: Framework::builtin(Mode::Freudian).parts,
        relationships: vec![],
        states: Default::default(),
        focus: None,
    };
    let ifs = Topology {
        parts: Framework::builtin(Mode::Ifs).parts,
        relationships: vec![],
        states: Default::default(),
        focus: None,
    };
    let freudian_id = freudian.parts[0].id.clone();
    let ifs_id = ifs.parts[0].id.clone();
    memory
        .put(
            &format!("{scope}/freudian/topology"),
            &serde_json::to_value(&freudian).unwrap(),
        )
        .await
        .unwrap();
    memory
        .put(
            &format!("{scope}/ifs/topology"),
            &serde_json::to_value(&ifs).unwrap(),
        )
        .await
        .unwrap();
    memory
        .put(
            &format!("{scope}/preferences"),
            &serde_json::to_value(ProjectPreferences {
                mode: Some(Mode::Freudian),
                ..ProjectPreferences::default()
            })
            .unwrap(),
        )
        .await
        .unwrap();
    memory
        .append(
            &format!("{scope}/freudian/identity/{freudian_id}/notes"),
            "note",
            "FREUDIAN-NOTE",
        )
        .await
        .unwrap();
    memory
        .append(
            &format!("{scope}/freudian/identity/{freudian_id}/notes"),
            "dream",
            "FREUDIAN-DREAM",
        )
        .await
        .unwrap();
    memory
        .append(
            &format!("{scope}/ifs/identity/{ifs_id}/notes"),
            "note",
            "IFS-NOTE",
        )
        .await
        .unwrap();
    memory.close().await.unwrap();

    let mut saved_command = env.command_for("responses");
    saved_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "notes", &freudian_id]);
    let output = saved_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(saved["mode"], "freudian");
    assert_eq!(saved["identity"], freudian_id);
    assert_eq!(saved["requested_limit"], 100);
    assert_eq!(saved["notes"][0]["content"], "FREUDIAN-NOTE");
    assert_eq!(saved["notes"][1]["role"], "dream");
    let dream_sequence = saved["notes"][1]["sequence"].as_i64().unwrap();

    let mut forget_command = env.command_for("responses");
    forget_command.env_remove("OPENAI_API_KEY").args([
        "memory",
        "forget",
        &freudian_id,
        "--note",
        &dream_sequence.to_string(),
    ]);
    let output = forget_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let forgotten: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(forgotten["mode"], "freudian");
    assert_eq!(forgotten["identity"], freudian_id);
    assert_eq!(forgotten["sequence"], dream_sequence);
    assert_eq!(forgotten["history_retained"], true);

    let mut after_command = env.command_for("responses");
    after_command
        .env_remove("OPENAI_API_KEY")
        .args(["memory", "notes", &freudian_id]);
    let output = after_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(after["notes"].as_array().map(Vec::len), Some(1));
    assert_eq!(after["notes"][0]["content"], "FREUDIAN-NOTE");

    let mut explicit_command = env.command_for("responses");
    explicit_command
        .env_remove("OPENAI_API_KEY")
        .args(["--mode", "ifs", "memory", "notes", &ifs_id, "--limit", "1"]);
    let output = explicit_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let explicit: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(explicit["mode"], "ifs");
    assert_eq!(explicit["notes"][0]["content"], "IFS-NOTE");
    let mut max_command = env.command_for("responses");
    max_command.env_remove("OPENAI_API_KEY").args([
        "memory",
        "notes",
        &freudian_id,
        "--limit",
        "1000",
    ]);
    let output = max_command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let max: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(max["requested_limit"], 1000);
    assert_eq!(max["notes"].as_array().map(Vec::len), Some(1));
    assert_eq!(max["truncated"], false);
    for limit in ["0", "1001"] {
        let output = env.run(&["memory", "notes", &freudian_id, "--limit", limit]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("between 1 and 1000"));
    }

    let legacy = Sandbox::new();
    std::fs::create_dir(&legacy.data).unwrap();
    let original = b"legacy notes must not be imported";
    let path = legacy.data.join("memory.sqlite3");
    std::fs::write(&path, original).unwrap();
    let output = legacy.run(&["memory", "notes", "missing"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!legacy.data.join("memory").exists());
    let output = legacy.run(&["memory", "forget", "missing", "--note", "1"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let output = legacy.run(&["memory", "export"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no memory yet"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!legacy.data.join("memory").exists());
}

#[cfg(unix)]
#[test]
fn a_dangling_legacy_link_is_rejected_instead_of_treated_as_fresh_memory() {
    let env = Sandbox::new();
    std::fs::create_dir(&env.data).unwrap();
    let legacy = env.data.join("memory.sqlite3");
    let missing = env.root.path().join("missing-legacy-target");
    std::os::unix::fs::symlink(&missing, &legacy).unwrap();
    let output = env.run(&["sessions"]);
    assert!(!output.status.success());
    assert_eq!(std::fs::read_link(&legacy).unwrap(), missing);
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn cli_refuses_an_unsafe_legacy_directory_before_import() {
    use std::os::unix::fs::PermissionsExt;

    let env = Sandbox::new();
    let data = env.root.path().join("unsafe legacy data; literal-dollar");
    kuru_platform::fs::Directory::ensure_private(&data).unwrap();
    let source = data.join("memory.sqlite3");
    let original = b"legacy source remains untouched";
    std::fs::write(&source, original).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
    command
        .arg("-C")
        .arg(&env.project)
        .arg("--data-dir")
        .arg(&data)
        .args(["--provider", "demo", "--no-dream", "sessions"])
        .env("XDG_CONFIG_HOME", env.root.path().join("config"));
    let output = command.output().unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("memory data directory"), "{stderr}");
    assert!(stderr.contains("mode 0700"), "{stderr}");
    assert!(stderr.contains(&format!("{data:?}")), "{stderr}");
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert!(!data.join("memory").exists());
}

#[cfg(unix)]
#[test]
fn cli_explains_owner_owned_unsafe_data_directory_without_legacy_sqlite() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    let env = Sandbox::new();
    let data = env.root.path().join("unsafe ordinary data; literal-dollar");
    kuru_platform::fs::Directory::ensure_private(&data).unwrap();
    let sentinel = data.join("sentinel");
    std::fs::write(&sentinel, b"ordinary contents remain untouched").unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    let before = std::fs::symlink_metadata(&data).unwrap();
    assert_eq!(before.uid(), nix::unistd::geteuid().as_raw());

    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
    command
        .arg("-C")
        .arg(&env.project)
        .arg("--data-dir")
        .arg(&data)
        .args(["--provider", "demo", "--no-dream", "run", "do not start"])
        .env("XDG_CONFIG_HOME", env.root.path().join("config"));
    let output = command.output().unwrap();
    let after = std::fs::symlink_metadata(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("memory data directory"), "{stderr}");
    assert!(stderr.contains("mode 0700"), "{stderr}");
    assert!(stderr.contains(&format!("{data:?}")), "{stderr}");
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"ordinary contents remain untouched"
    );
    assert_eq!(
        before.permissions().mode() & 0o777,
        after.permissions().mode() & 0o777
    );
    assert!(!data.join("memory.sqlite3").exists());
    assert!(!data.join("memory").exists());

    let target = env.root.path().join("linked-target");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    let linked = env.root.path().join("linked-data");
    symlink(&target, &linked).unwrap();
    let mut linked_command = Command::new(env!("CARGO_BIN_EXE_kuru"));
    linked_command
        .arg("-C")
        .arg(&env.project)
        .arg("--data-dir")
        .arg(&linked)
        .args(["--provider", "demo", "--no-dream", "run", "do not start"])
        .env("XDG_CONFIG_HOME", env.root.path().join("config"));
    let linked_output = linked_command.output().unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!linked_output.status.success());
    assert!(
        std::fs::symlink_metadata(&linked)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!String::from_utf8_lossy(&linked_output.stderr).contains("mode 0700"));
}

#[test]
fn cli_memory_progress_is_bounded_and_keeps_json_on_stdout() {
    let env = Sandbox::new();
    let cache = env.root.path().join("fresh verified runtime cache");
    let memory = kuru_core::MemoryConfig {
        cache_dir: Some(cache),
        offline: true,
        ..Default::default()
    };
    std::fs::write(
        env.root.path().join("config/kuru/config.toml"),
        toml::to_string(&std::collections::BTreeMap::from([("memory", memory)])).unwrap(),
    )
    .unwrap();

    let cold_started = std::time::Instant::now();
    let cold = env
        .command()
        .env("KURU_TEST_MEMORY_STARTUP_STAGES", "1")
        .args(["run", "cold memory", "--json"])
        .output()
        .unwrap();
    let cold_elapsed = cold_started.elapsed();
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let cold_json: Value = serde_json::from_slice(&cold.stdout).unwrap();
    assert!(cold_json["text"].as_str().unwrap().contains("demo"));
    let cold_stderr = String::from_utf8_lossy(&cold.stderr);
    assert_memory_progress(&cold_stderr);
    assert!(cold_stderr.contains("Memory is ready at"), "{cold_stderr}");
    assert!(
        cold_stderr.contains("kuru memory notes ID"),
        "{cold_stderr}"
    );

    let warm_started = std::time::Instant::now();
    let warm = env.run(&["run", "warm memory", "--json"]);
    let warm_elapsed = warm_started.elapsed();
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );
    let warm_json: Value = serde_json::from_slice(&warm.stdout).unwrap();
    assert!(warm_json["text"].as_str().unwrap().contains("demo"));
    assert_memory_progress(&String::from_utf8_lossy(&warm.stderr));
    eprintln!(
        "observed isolated CLI startup wall time: cold={cold_elapsed:?}; warm={warm_elapsed:?}; no optimization claim"
    );
}

#[test]
fn headless_json_remains_machine_readable_when_old_context_is_omitted() {
    let env = Sandbox::new();
    let sentinel = "older-context-remains-stored";
    let long_prompt = format!("{sentinel} {}", "x".repeat(12_000));
    let first = env.run(&["run", &long_prompt, "--json"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let config_path = env.root.path().join("config/kuru/config.toml");
    let config = format!(
        "assumed_context_window_tokens = 4000\ncontext_output_reserve_tokens = 256\n{}",
        std::fs::read_to_string(&config_path).unwrap()
    );
    std::fs::write(&config_path, config).unwrap();

    let second = env.run(&["run", "short follow-up", "--json"]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let result: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert!(result["text"].as_str().is_some());
    assert!(!String::from_utf8_lossy(&second.stdout).contains(sentinel));
    assert!(!String::from_utf8_lossy(&second.stderr).contains(sentinel));

    let export = env.run(&["memory", "export"]);
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(String::from_utf8_lossy(&export.stdout).contains(sentinel));
}

#[tokio::test]
async fn cli_undo_dream_shows_one_notice_without_constructing_a_provider() {
    use kuru_memory::MemoryStore;

    let env = Sandbox::new();
    kuru_platform::fs::Directory::ensure_private(&env.data).unwrap();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    MemoryStore::open(options.clone())
        .await
        .unwrap()
        .close()
        .await
        .unwrap();

    let mut first = env.command_for("responses");
    let first = first
        .env_remove("OPENAI_API_KEY")
        .args(["undo-dream"])
        .output()
        .unwrap();
    assert!(
        !first.status.success(),
        "undo without a dream must still fail"
    );
    assert!(first.stdout.is_empty());
    let first_stderr = String::from_utf8_lossy(&first.stderr);
    assert!(
        first_stderr.contains("Memory is ready at"),
        "{first_stderr}"
    );
    assert!(
        !first_stderr.contains("OPENAI_API_KEY"),
        "undo-dream attempted provider setup: {first_stderr}"
    );

    let mut observed_options = options.clone();
    observed_options.read_only = true;
    let project = env.project.canonicalize().unwrap();
    let (_, opening) = MemoryStore::open_managed_observed(
        observed_options,
        project,
        PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
    );
    let reopened = opening.await.unwrap();
    assert_eq!(
        reopened
            .get(&format!("{scope}/notice/memory-storage"))
            .await
            .unwrap(),
        Some(serde_json::json!({"version": 1}))
    );
    reopened.close().await.unwrap();

    let mut second = env.command_for("responses");
    let second = second
        .env_remove("OPENAI_API_KEY")
        .args(["undo-dream"])
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(
        !String::from_utf8_lossy(&second.stderr).contains("Memory is ready at"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn cli_imports_a_real_legacy_wal_without_changing_its_layout() {
    let env = Sandbox::new();
    kuru_platform::fs::Directory::ensure_private(&env.data).unwrap();
    let legacy_path = env.data.join("memory.sqlite3");
    let connection = rusqlite::Connection::open(&legacy_path).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA application_id=1263882837;
             PRAGMA user_version=1;
             CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL);
             CREATE TABLE state (`key` TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
        )
        .unwrap();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    connection
        .execute(
            "INSERT INTO messages (namespace, role, content) VALUES (?1, 'user', 'committed WAL message')",
            [format!("{scope}/transcript/legacy")],
        )
        .unwrap();
    let original_main = std::fs::read(&legacy_path).unwrap();
    let wal_path = env.data.join("memory.sqlite3-wal");
    let original_wal = std::fs::read(&wal_path).unwrap();
    assert!(env.data.join("memory.sqlite3-shm").is_file());

    let output = env
        .command()
        .env("KURU_TEST_MEMORY_STARTUP_STAGES", "1")
        .arg("sessions")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap()
            .is_array()
    );
    assert_eq!(std::fs::read(&legacy_path).unwrap(), original_main);
    assert_eq!(std::fs::read(&wal_path).unwrap(), original_wal);
    assert!(env.data.join("memory.sqlite3-shm").is_file());
    let snapshots = env.data.join("memory/legacy");
    assert_eq!(
        std::fs::read_dir(snapshots)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sqlite3"))
            .count(),
        1
    );
    let exported = env.run(&["memory", "export"]);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let records = serde_json::from_slice::<Value>(&exported.stdout).unwrap();
    assert!(records["records"].as_array().unwrap().iter().any(|record| {
        record["kind"] == "message"
            && record["namespace"] == format!("{scope}/transcript/legacy")
            && record["role"] == "user"
            && record["content"] == "committed WAL message"
    }));
    drop(connection);
}

#[tokio::test]
async fn sequential_commands_release_clients_reuse_warm_memory_and_reap_on_retirement() {
    let env = Sandbox::new();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    let store_path = env
        .data
        .join("memory")
        .join(scope.strip_prefix("project/").unwrap());
    let endpoint_path = store_path.join("endpoint.json");
    let stopped = || {
        assert!(
            matches!(std::fs::symlink_metadata(store_path.join("endpoint.json")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound),
            "CLI returned while its owned endpoint was still published"
        );
        #[cfg(unix)]
        let lifecycle_path = store_path.join("lifecycle.lock");
        #[cfg(windows)]
        let lifecycle_path = {
            use kuru_platform::fs::{Directory, NameRetention, Privacy};
            let directory =
                Directory::open(&store_path, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
            let key: String = directory
                .identity()
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            env.data
                .join("memory/lifecycles")
                .join(format!("{key}.lock"))
        };
        let lease = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lifecycle_path)
            .unwrap();
        lease
            .try_lock()
            .expect("CLI returned before its supervisor released the lifecycle lease");
    };
    env.success(&["run", "Seed memory for sequential inspection"]);
    let generation = std::fs::read(&endpoint_path).unwrap();
    let warm = |stage: &str| {
        assert_eq!(
            std::fs::read(&endpoint_path)
                .unwrap_or_else(|error| panic!("{stage}: warm endpoint is unavailable: {error}")),
            generation,
            "{stage}: sequential CLI command replaced the warm service generation"
        );
    };
    warm("seed");
    env.success(&["memory", "status"]);
    warm("successful memory client");
    let output = env.run(&["memory", "history", "--limit", "0"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("between 1 and 1000"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    warm("failing memory client");
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    kuru_memory::test_support::retire_idle_service(&options)
        .await
        .unwrap();
    stopped();
}
