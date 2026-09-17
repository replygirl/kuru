//! Compiler-free installed-runtime acceptance. This fixture sets offline=true
//! and removes PATH tools; it does not install an OS egress firewall. Runtime
//! download removal is also checked in the owning memory package/source review.

#[path = "support/update_profiles.rs"]
mod update_profiles;

use anyhow::{Context, Result, ensure};
use kuru_core::MemoryConfig;
use kuru_delivery::{archive, command::Command};
use kuru_memory::{MemoryStore, OpenOptions};
use kuru_platform::fs::{Directory, regular_file_info};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};
#[cfg(unix)]
use std::{os::unix::fs::PermissionsExt, process::Stdio};
#[cfg(unix)]
use tokio::io::{AsyncRead, AsyncReadExt};

const OUTPUT_LIMIT: u64 = 1024 * 1024;
// Cold creation starts staging and active servers, each with the configured
// 30-second bound, then includes their handshakes and bounded shutdowns.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(100);
const MANIFEST: &str = include_str!("../../../packages/kuru-memory/support/dolt-assets.json");
#[cfg(windows)]
const BOOTSTRAP_PHASES: &[&str] = &[
    "Kuru bootstrap phase: entered",
    "Kuru bootstrap phase: loading native bridge",
    "Kuru bootstrap phase: native bridge ready",
    "Kuru bootstrap phase: installation state locked",
    "Kuru bootstrap phase: loading release manifest",
    "Kuru bootstrap phase: loading release archive",
    "Kuru bootstrap phase: release archive verified",
    "Kuru bootstrap phase: release archive validated",
    "Kuru bootstrap phase: installation published",
    "Kuru bootstrap phase: cleanup complete",
];

fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let bytes = Read::read(&mut file, &mut buffer)?;
        if bytes == 0 {
            break;
        }
        hash.update(&buffer[..bytes]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(unix)]
async fn capture(stream: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    stream
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut output)
        .await?;
    ensure!(
        output.len() as u64 <= OUTPUT_LIMIT,
        "fixture command exceeded output bound"
    );
    Ok(output)
}

#[cfg(unix)]
async fn execute(command: &mut Command) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().context("start installed Kuru")?;
    let pid = child.id();
    let stdout = tokio::spawn(capture(child.stdout.take().context("stdout pipe")?));
    let stderr = tokio::spawn(capture(child.stderr.take().context("stderr pipe")?));
    let status = match tokio::time::timeout(COMMAND_TIMEOUT, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            child.kill().await.context("kill timed-out fixture child")?;
            let output = tokio::time::timeout(Duration::from_secs(5), stdout).await;
            let error = tokio::time::timeout(Duration::from_secs(5), stderr).await;
            anyhow::bail!(
                "installed Kuru {pid:?} timed out after {COMMAND_TIMEOUT:?}; stdout={output:?}; stderr={error:?}"
            );
        }
    };
    let stdout = tokio::time::timeout(Duration::from_secs(5), stdout)
        .await
        .context("stdout did not close")???;
    let stderr = tokio::time::timeout(Duration::from_secs(5), stderr)
        .await
        .context("stderr did not close")???;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(windows)]
async fn execute(command: &mut Command) -> Result<Output> {
    let output = kuru_delivery::command::output(command, COMMAND_TIMEOUT).await?;
    ensure!(
        output.stdout.len() as u64 <= OUTPUT_LIMIT && output.stderr.len() as u64 <= OUTPUT_LIMIT,
        "fixture command exceeded output bound"
    );
    Ok(output)
}

#[cfg(windows)]
fn assert_bootstrap_phases(output: &Output) -> Result<()> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let captured = format!("{stdout}\n{stderr}");
    let mut offset = 0;
    for phase in BOOTSTRAP_PHASES {
        let Some(found) = captured[offset..].find(phase) else {
            let stdout_preview =
                String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(64 * 1024)]);
            let stderr_preview =
                String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(64 * 1024)]);
            anyhow::bail!(
                "stock PowerShell omitted bootstrap phase {phase:?}; stdout prefix: {stdout_preview:?}; stderr prefix: {stderr_preview:?}"
            );
        };
        offset += found + phase.len();
    }
    Ok(())
}

struct Installation {
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    empty_path: PathBuf,
    temporary: PathBuf,
    project: PathBuf,
    binary: PathBuf,
    memory: MemoryConfig,
}
impl Installation {
    fn new(root: &Path, label: &str, project: &Path, binary: &Path) -> Result<Self> {
        let root = root.join(label);
        fs::create_dir(&root)?;
        let home = root.join("home");
        let config = root.join("config");
        let data = root.join("data");
        let cache = root.join("empty-engine-cache");
        let empty_path = root.join("no-external-tools");
        let temporary = root.join("tmp");
        for directory in [&home, &config, &empty_path, &temporary] {
            fs::create_dir(directory)?;
        }
        fs::create_dir(config.join("kuru"))?;
        let memory = MemoryConfig {
            offline: true,
            cache_dir: Some(cache.clone()),
            ..Default::default()
        };
        let settings = std::collections::BTreeMap::from([("memory", &memory)]);
        fs::write(config.join("kuru/config.toml"), toml::to_string(&settings)?)?;
        ensure!(
            !cache.exists() && !data.exists(),
            "offline fixture must start with absent caches and data"
        );
        Ok(Self {
            home,
            config,
            data,
            cache,
            empty_path,
            temporary,
            project: project.to_owned(),
            binary: binary.to_owned(),
            memory,
        })
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", &self.config)
            .env("LOCALAPPDATA", self.home.join("local"))
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_CACHE_HOME", self.home.join("cache"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("TMPDIR", &self.temporary)
            .env("TMP", &self.temporary)
            .env("TEMP", &self.temporary)
            .env("PATH", &self.empty_path)
            .current_dir(&self.project)
            .arg("-C")
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.data)
            .args(["--mode", "freudian", "--no-dream"]);
        // Preserve only the coverage destination, never ambient provider state.
        if let Some(path) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", path);
        }
        #[cfg(windows)]
        if let Some(path) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", path);
        }
        command
    }
    async fn run(&self, args: &[&str]) -> Result<Value> {
        let output = execute(self.command().args(["--provider", "demo"]).args(args))
            .await
            .with_context(|| format!("execute installed command {args:?}"))?;
        ensure!(
            output.status.success(),
            "installed command {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).context("installed command did not return JSON")
    }
    async fn native_auth_status(&self) -> Result<()> {
        let output = execute(self.command().arg("auth")).await?;
        ensure!(
            output.status.success(),
            "installed native auth failed without PATH tools: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status: Value = serde_json::from_slice(&output.stdout)?;
        ensure!(
            status["authenticated"] == false && status["api_key_available"] == false,
            "isolated installation has unexpected provider credentials"
        );
        ensure!(!self.data.exists(), "fresh auth inspection created data");
        Ok(())
    }

    async fn api_key_access(&self) -> Result<()> {
        use axum::{
            Json, Router,
            extract::State,
            http::{HeaderMap, StatusCode},
            routing::{get, post},
        };
        use std::sync::Arc;
        use tokio::sync::Mutex;
        #[derive(Default)]
        struct Requests {
            models: usize,
            completions: usize,
            valid: bool,
            reject: bool,
        }
        type Shared = Arc<Mutex<Requests>>;
        fn authorized(headers: &HeaderMap) -> bool {
            headers
                .get("authorization")
                .is_some_and(|value| value == "Bearer synthetic-installed-api-key")
                && !headers.contains_key("chatgpt-account-id")
        }
        async fn models(State(state): State<Shared>, headers: HeaderMap) -> Json<Value> {
            let mut requests = state.lock().await;
            requests.models += 1;
            requests.valid &= authorized(&headers);
            Json(serde_json::json!({"data":[{"id":"fixture-openai-model"}]}))
        }
        async fn response(
            State(state): State<Shared>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            let mut requests = state.lock().await;
            requests.completions += 1;
            requests.valid &= authorized(&headers)
                && body["model"] == "fixture-openai-model"
                && body["store"] == false;
            if requests.reject {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error":{"message":"synthetic-installed-api-key"}})),
                );
            }
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Native OpenAI request completed."}]}],"usage":{"input_tokens":1,"output_tokens":1}}),
                ),
            )
        }
        let state = Arc::new(Mutex::new(Requests {
            valid: true,
            ..Default::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let router = Router::new()
            .route("/models", get(models))
            .route("/responses", post(response))
            .with_state(state.clone());
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let settings = self.config.join("api-fixture.toml");
        fs::write(
            &settings,
            toml::to_string(&std::collections::BTreeMap::from([("api_base", &base)]))?,
        )?;
        for args in [
            vec!["models"],
            vec![
                "--model",
                "fixture-openai-model",
                "run",
                "Check the native OpenAI connection.",
                "--json",
            ],
        ] {
            let output = execute(
                self.command()
                    .env("OPENAI_API_KEY", "synthetic-installed-api-key")
                    .arg("--config")
                    .arg(&settings)
                    .args(["--provider", "responses"])
                    .args(&args),
            )
            .await?;
            ensure!(
                output.status.success(),
                "installed API request failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: Value = serde_json::from_slice(&output.stdout)?;
            if args[0] == "models" {
                ensure!(
                    result[0]["id"] == "fixture-openai-model",
                    "API model response was not returned"
                );
            } else {
                ensure!(
                    result["text"] == "Native OpenAI request completed.",
                    "API completion was not returned"
                );
            }
            ensure!(
                !String::from_utf8_lossy(&output.stdout).contains("synthetic-installed-api-key")
                    && !String::from_utf8_lossy(&output.stderr)
                        .contains("synthetic-installed-api-key"),
                "installed command exposed its API key"
            );
        }
        state.lock().await.reject = true;
        let rejected = execute(
            self.command()
                .env("OPENAI_API_KEY", "synthetic-installed-api-key")
                .arg("--config")
                .arg(&settings)
                .args([
                    "--provider",
                    "responses",
                    "--model",
                    "fixture-openai-model",
                    "run",
                    "Check rejected OpenAI access.",
                    "--json",
                ]),
        )
        .await?;
        ensure!(
            !rejected.status.success(),
            "rejected OpenAI request reported success"
        );
        let diagnostic = String::from_utf8_lossy(&rejected.stderr);
        ensure!(
            diagnostic
                .contains("provider errors: Responses completion access was denied (HTTP 403)"),
            "native CLI hid the OpenAI failure: {diagnostic}"
        );
        ensure!(
            !diagnostic.contains("synthetic-installed-api-key"),
            "native CLI echoed the rejected credential"
        );
        let observed = state.lock().await;
        ensure!(
            observed.valid && observed.models >= 1 && observed.completions >= 1,
            "installed API route did not send the expected authenticated requests"
        );
        ensure!(
            !self.data.join("auth").exists(),
            "API-key use created stored OAuth credentials"
        );
        Ok(())
    }
    async fn conversation(&self, marker: &str, asset: &Value, version: &str) -> Result<()> {
        ensure!(
            !self.cache.exists() && !self.data.exists(),
            "cold launch must not reuse an engine or database"
        );
        let first = self
            .run(&["run", marker, "--json"])
            .await
            .context("first offline conversation from empty cache")?;
        let session = first["session"].as_str().context("session ID")?;
        ensure!(
            !first["text"].as_str().context("first response")?.is_empty(),
            "first response is empty"
        );
        let after_first = self.run(&["memory", "status"]).await?;
        ensure!(
            after_first["engine"] == "dolt" && after_first["engine_version"] == version,
            "installed command did not use the pinned full Dolt engine"
        );
        let second = self
            .run(&[
                "--resume",
                session,
                "run",
                "Continue the saved conversation",
                "--json",
            ])
            .await?;
        ensure!(
            second["session"] == session,
            "restart did not resume the durable session"
        );
        let sessions = self.run(&["sessions"]).await?;
        ensure!(
            sessions
                .as_array()
                .context("session list")?
                .iter()
                .any(|entry| entry["id"] == session),
            "saved session missing after restart"
        );
        let after_second = self.run(&["memory", "status"]).await?;
        ensure!(
            after_first["revision"] != after_second["revision"],
            "second conversation did not create a durable revision"
        );
        let revisions = self.run(&["memory", "history", "--limit", "100"]).await?;
        let revisions = revisions.as_array().context("revision history")?;
        for revision in [&after_first["revision"], &after_second["revision"]] {
            ensure!(
                revisions.iter().any(|entry| entry["hash"] == *revision),
                "conversation revision missing from actual Dolt history"
            );
        }

        // Inspect the real persisted transcript, not merely session metadata.
        // The installed executable remains the SQL supervisor for this reader.
        let scope = kuru_runtime::project_scope(&self.project)?;
        let mut options = OpenOptions::new(self.data.clone(), scope.clone());
        options.config = self.memory.clone();
        options.read_only = true;
        options.supervisor = Some(self.binary.clone());
        let store = tokio::time::timeout(COMMAND_TIMEOUT, MemoryStore::open(options))
            .await
            .context("read-only transcript reopen timed out")??;
        let transcript = store
            .history(&format!("{scope}/transcript/{session}"), 100)
            .await;
        let cleanup = store.close().await;
        let transcript = transcript?;
        cleanup?;
        ensure!(
            transcript
                .iter()
                .any(|message| message.role == "user" && message.plain_text() == Some(marker)),
            "original user message was not persisted"
        );
        ensure!(
            transcript.iter().any(|message| message.role == "user"
                && message.plain_text() == Some("Continue the saved conversation")),
            "resumed user message was not persisted"
        );
        ensure!(
            transcript
                .iter()
                .filter(|message| message.role == "assistant")
                .count()
                == 2,
            "persisted transcript does not contain both answers exactly once"
        );

        let runtime = self
            .cache
            .join(version)
            .join(asset["target"].as_str().context("asset target")?);
        for (name, size, sha) in [
            (
                asset["executable_name"]
                    .as_str()
                    .context("manifest executable name")?,
                "executable_bytes",
                "executable_sha256",
            ),
            ("LICENSES", "license_bytes", "license_sha256"),
        ] {
            let path = runtime.join(name);
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "extracted {name} is not an ordinary file"
            );
            ensure!(
                metadata.len() == asset[size].as_u64().context("manifest payload size")?,
                "extracted {name} size differs from upstream manifest"
            );
            ensure!(
                digest(&path)? == asset[sha].as_str().context("manifest payload digest")?,
                "extracted {name} digest differs from upstream manifest"
            );
        }
        let licenses = fs::read_to_string(runtime.join("LICENSES"))?;
        ensure!(
            licenses.contains("Apache License") && licenses.contains("Copyright"),
            "upstream license payload was not preserved"
        );
        ensure!(
            fs::read_dir(&self.empty_path)?.next().is_none(),
            "application installed an external PATH dependency"
        );
        Ok(())
    }
}

#[cfg(unix)]
async fn install_packaged(
    _root: &Path,
    releases: &Path,
    version: &str,
    install_dir: &Path,
    target: &str,
) -> Result<PathBuf> {
    archive::install(
        releases.to_str().context("local release path")?,
        version,
        install_dir,
        Some(target),
    )
    .await
}

#[cfg(windows)]
async fn install_packaged(
    root: &Path,
    releases: &Path,
    version: &str,
    install_dir: &Path,
    target: &str,
) -> Result<PathBuf> {
    let system = kuru_platform::windows::process::system_directory()?;
    let powershell = system.join("WindowsPowerShell/v1.0/powershell.exe");
    ensure!(powershell.is_file(), "stock PowerShell 5.1 is required");
    let isolated = root.join("bootstrap environment");
    fs::create_dir(&isolated)?;
    let empty_path = isolated.join("empty PATH");
    fs::create_dir(&empty_path)?;
    let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/kuru-delivery/support/install.ps1");
    let mut command = Command::new(powershell);
    command
        .env_clear()
        .env(
            "SystemRoot",
            system.parent().context("Windows system root")?,
        )
        .env("PROCESSOR_ARCHITECTURE", "AMD64")
        .env("USERPROFILE", &isolated)
        .env("APPDATA", &isolated)
        .env("LOCALAPPDATA", &isolated)
        .env("TMP", &isolated)
        .env("TEMP", &isolated)
        .env("PATH", &empty_path)
        .current_dir(root)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(bootstrap)
        .args(["-Version", version, "-Target", target, "-ReleaseBase"])
        .arg(releases)
        .arg("-InstallDir")
        .arg(install_dir)
        .arg("-Verbose");
    if let Some(value) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", value);
    }
    let output = execute(&mut command)
        .await
        .context("execute stock PowerShell package installation")?;
    ensure!(
        output.status.success(),
        "stock PowerShell could not install the genuine packaged Kuru: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_bootstrap_phases(&output)?;
    ensure!(
        fs::read_dir(empty_path)?.next().is_none(),
        "bootstrap added a PATH dependency"
    );
    Ok(install_dir.join("kuru.exe"))
}

async fn packaged_roundtrip(root: &Path) -> Result<()> {
    let binary = match std::env::var_os("KURU_EMBEDDED_TEST_BINARY") {
        Some(path) => {
            ensure!(
                Path::new(&path).is_absolute(),
                "KURU_EMBEDDED_TEST_BINARY must be an absolute path"
            );
            PathBuf::from(path)
        }
        None => PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
    };
    let target = archive::host_target()?;
    let manifest: Value = serde_json::from_str(MANIFEST)?;
    let dolt_version = manifest["version"].as_str().context("Dolt version")?;
    let asset = manifest["assets"]
        .as_array()
        .context("Dolt assets")?
        .iter()
        .find(|asset| asset["target"] == target)
        .context("host Dolt asset")?;
    let project = root.join("stable project λ");
    let releases = root.join("local releases");
    let install_dir = root.join("installed bin");
    fs::create_dir(&project)?;
    let version = env!("CARGO_PKG_VERSION");
    let executable_bytes = fs::metadata(&binary)?.len();
    eprintln!(
        "embedded runtime packaging input: target={target} executable_bytes={executable_bytes} expanded_limit_bytes={}",
        archive::MAX_ARCHIVE_BYTES
    );
    let archive_path = archive::package(&binary, target, version, &releases).with_context(|| {
        format!(
            "package embedded runtime: target={target} executable_bytes={executable_bytes} expanded_limit_bytes={}",
            archive::MAX_ARCHIVE_BYTES
        )
    })?;
    let name = archive_path
        .file_name()
        .and_then(OsStr::to_str)
        .context("archive name")?;
    fs::copy(
        releases.join(format!("{name}.sha256")),
        releases.join("SHA256SUMS"),
    )?;
    let installed = install_packaged(root, &releases, version, &install_dir, target)
        .await
        .context("install the genuine packaged executable")?;
    let original_digest = digest(&binary)?;
    ensure!(
        digest(&installed)? == original_digest,
        "direct installation changed the packaged executable"
    );
    ensure!(
        fs::read_dir(&install_dir)?.count() == if cfg!(windows) { 2 } else { 1 },
        "installation must contain only Kuru and the Windows update coordination directory"
    );
    let first = Installation::new(root, "direct", &project, &installed)?;
    first.native_auth_status().await?;
    let reported = execute(first.command().arg("--version"))
        .await
        .context("run the directly installed executable's version command")?;
    ensure!(
        reported.status.success()
            && String::from_utf8_lossy(&reported.stdout).trim() == format!("kuru {version}"),
        "selected packaged executable has an unexpected version"
    );
    first
        .conversation(
            "Remember the offline direct-install marker: amber-731",
            asset,
            dolt_version,
        )
        .await
        .context("verify the direct installation's cold offline memory")?;
    first.api_key_access().await?;

    let previous_identity = regular_file_info(&File::open(&installed)?)?.identity;
    let mut update = first.command();
    let profiles = update_profiles::UpdateProfiles::observe(&mut update)?;
    let output = execute(
        update
            .args(["update", "--version", version, "--release-base"])
            .arg(&releases),
    )
    .await
    .context("run the installed executable's self-update")?;
    ensure!(
        output.status.success(),
        "native self-update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if let Some(profiles) = profiles {
        profiles.verify().await?;
    }
    ensure!(
        regular_file_info(&File::open(&installed)?)?.identity != previous_identity,
        "self-update did not replace the installed executable"
    );
    ensure!(
        digest(&installed)? == original_digest,
        "self-update lost or changed the bundled executable"
    );
    let second = Installation::new(root, "updated", &project, &installed)?;
    second.native_auth_status().await?;
    second
        .conversation(
            "Remember the offline self-update marker: indigo-942",
            asset,
            dolt_version,
        )
        .await
        .context("verify the updated installation's cold offline memory")?;
    second.api_key_access().await?;
    ensure!(
        fs::read_dir(&install_dir)?.count() == if cfg!(windows) { 2 } else { 1 },
        "self-update left a required companion executable"
    );
    eprintln!(
        "embedded runtime accepted: target={target} executable_bytes={} archive_bytes={} engine={dolt_version}; direct install and self-update each persisted chat from an empty offline cache",
        fs::metadata(&binary)?.len(),
        fs::metadata(archive_path)?.len()
    );
    Ok(())
}

#[tokio::test]
async fn packaged_install_and_update_preserve_complete_offline_memory() {
    let mut builder = tempfile::Builder::new();
    builder.prefix("kuru-embedded-acceptance-");
    #[cfg(unix)]
    builder.permissions(fs::Permissions::from_mode(0o700));
    let root = builder.tempdir().unwrap();
    let private = Directory::ensure_private(&root.path().join("private")).unwrap();
    if let Err(error) = packaged_roundtrip(private.path()).await {
        // Keep private diagnostics on failure; a timeout must never remove a
        // directory while an owned SQL supervisor may still be completing EOF.
        let path = root.keep();
        panic!(
            "{error:#}\nprivate embedded-runtime fixture retained at {}",
            path.display()
        );
    }
}
