//! Compiler-free installed-runtime acceptance. This fixture sets offline=true
//! and removes PATH tools; it does not install an OS egress firewall. Runtime
//! download removal is also checked in the owning memory package/source review.

#[path = "support/update_profiles.rs"]
mod update_profiles;

use anyhow::{Context, Result, ensure};
use kuru_core::MemoryConfig;
use kuru_delivery::{archive, command::Command, shell_support};
use kuru_memory::{MemoryStore, OpenOptions};
use kuru_platform::fs::{Directory, regular_file_info};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions as FsOpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};
#[cfg(unix)]
use std::{os::unix::fs::PermissionsExt, process::Stdio};
#[cfg(unix)]
use tokio::io::{AsyncRead, AsyncReadExt};

const OUTPUT_LIMIT: u64 = 1024 * 1024;
const BOOTSTRAP_INVENTORY_ENTRIES: usize = 64;
const BOOTSTRAP_INVENTORY_BYTES: usize = 4096;
// Cold creation starts staging and active servers, each with the configured
// 30-second bound, then includes their handshakes and bounded shutdowns.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(100);
const PREPARE_INPUT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_INSTRUMENTED_INPUT_BYTES: u64 = archive::MAX_ARCHIVE_BYTES as u64 + 32 * 1024 * 1024;
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
    "Kuru bootstrap phase: installation stage opened",
    "Kuru bootstrap phase: publication starting",
    "Kuru bootstrap phase: installation published",
    "Kuru bootstrap phase: cleanup complete",
];
#[cfg(windows)]
const BOOTSTRAP_DIRECT_CHECKPOINTS: &[&str] = &[
    "Kuru bootstrap direct checkpoint: script entered",
    "Kuru bootstrap direct checkpoint: native bridge starting",
    "Kuru bootstrap direct checkpoint: native bridge ready",
];

fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    digest_file(&mut file)
}

fn digest_file(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let bytes = file.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        hash.update(&buffer[..bytes]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

async fn prepare_instrumented_packaging_input(root: &Path, source: &Path) -> Result<PathBuf> {
    let mut held = File::open(source).context("open instrumented Cargo artifact")?;
    let before = regular_file_info(&held)?;
    ensure!(
        before.len > 0 && before.len <= MAX_INSTRUMENTED_INPUT_BYTES,
        "instrumented Cargo artifact exceeds the bounded fixture input"
    );
    let source_digest = digest_file(&mut held)?;
    let staged = root.join(if cfg!(windows) {
        "instrumented-package-input.exe"
    } else {
        "instrumented-package-input"
    });
    let mut candidate = FsOpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .context("create private instrumented packaging input")?;
    let copied_bytes = std::io::copy(
        &mut Read::by_ref(&mut held).take(MAX_INSTRUMENTED_INPUT_BYTES + 1),
        &mut candidate,
    )?;
    ensure!(
        copied_bytes == before.len,
        "instrumented Cargo artifact changed length while snapshotting"
    );
    candidate.set_permissions(held.metadata()?.permissions())?;
    candidate.flush()?;
    candidate.sync_all()?;
    drop(candidate);
    let mut copied = File::open(&staged)?;
    let copied_info = regular_file_info(&copied)?;
    ensure!(
        copied_info.identity != before.identity
            && copied_info.links == 1
            && copied_info.len == before.len
            && digest_file(&mut copied)? == source_digest,
        "instrumented packaging input is not an independent exact copy"
    );

    #[cfg(target_os = "macos")]
    let mut strip = Command::new("/usr/bin/strip");
    #[cfg(target_os = "macos")]
    strip.args(["-x", "-S"]).arg(&staged);
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut strip = Command::new("strip");
    #[cfg(all(unix, not(target_os = "macos")))]
    strip.arg("--strip-all").arg(&staged);
    #[cfg(unix)]
    {
        let output = kuru_delivery::command::output(&mut strip, PREPARE_INPUT_TIMEOUT)
            .await
            .context("remove symbols from the private coverage copy")?;
        ensure!(
            output.status.success(),
            "debug-symbol removal failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let current = File::open(source).context("reopen instrumented Cargo artifact")?;
    let after = regular_file_info(&held)?;
    let named = regular_file_info(&current)?;
    ensure!(
        before.identity == after.identity
            && before.identity == named.identity
            && before.len == after.len
            && before.len == named.len
            && digest_file(&mut held)? == source_digest,
        "instrumented Cargo artifact changed while preparing its private copy"
    );
    let staged_info = regular_file_info(&File::open(&staged)?)?;
    ensure!(
        staged_info.identity != before.identity
            && staged_info.links == 1
            && staged_info.len > 0
            && staged_info.len <= archive::MAX_ARCHIVE_BYTES as u64,
        "prepared instrumented packaging input is not a bounded independent file: distinct_identity={}, links={}, bytes={}, maximum_bytes={}",
        staged_info.identity != before.identity,
        staged_info.links,
        staged_info.len,
        archive::MAX_ARCHIVE_BYTES,
    );
    #[cfg(unix)]
    ensure!(
        staged_info.len < before.len,
        "symbol removal did not reduce the instrumented packaging input"
    );

    let destination = std::env::var_os("LLVM_PROFILE_FILE")
        .map(PathBuf::from)
        .context("instrumented packaging preparation requires a coverage destination")?;
    ensure!(
        destination.is_absolute(),
        "coverage destination must be absolute"
    );
    let profile_dir = destination.parent().context("coverage directory")?;
    let reservation = tempfile::Builder::new()
        .prefix("kuru-package-input-")
        .tempfile_in(profile_dir)?;
    let prefix = format!(
        "{}-",
        reservation
            .path()
            .file_name()
            .and_then(OsStr::to_str)
            .context("coverage reservation filename")?
    );
    let probe_root = root.join("instrumented-packaging-probe");
    let probe_home = probe_root.join("home");
    let probe_config = probe_root.join("config");
    let probe_cache = probe_root.join("cache");
    let probe_data = probe_root.join("data");
    let probe_temporary = probe_root.join("temporary");
    let probe_workspace = probe_root.join("workspace");
    let probe_empty_path = probe_root.join("empty-path");
    for directory in [
        &probe_home,
        &probe_config,
        &probe_cache,
        &probe_data,
        &probe_temporary,
        &probe_workspace,
        &probe_empty_path,
    ] {
        fs::create_dir_all(directory)?;
    }
    let mut probe = Command::new(&staged);
    probe
        .env_clear()
        .env("HOME", &probe_home)
        .env("USERPROFILE", &probe_home)
        .env("APPDATA", &probe_config)
        .env("LOCALAPPDATA", &probe_cache)
        .env("XDG_CONFIG_HOME", &probe_config)
        .env("XDG_CACHE_HOME", &probe_cache)
        .env("XDG_DATA_HOME", &probe_data)
        .env("TMPDIR", &probe_temporary)
        .env("TMP", &probe_temporary)
        .env("TEMP", &probe_temporary)
        .env("PATH", &probe_empty_path)
        .env(
            "LLVM_PROFILE_FILE",
            profile_dir.join(format!("{prefix}%p-%m.profraw")),
        )
        .current_dir(&probe_workspace)
        .arg("-C")
        .arg(&probe_workspace)
        .arg("--data-dir")
        .arg(&probe_data)
        .arg("config");
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("SystemRoot") {
        probe.env("SystemRoot", path);
    }
    let output = kuru_delivery::command::output(&mut probe, PREPARE_INPUT_TIMEOUT)
        .await
        .context("run prepared instrumented packaging input")?;
    ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stderr).contains("memory was not opened"),
        "prepared instrumented packaging input did not complete isolated configuration inspection: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let mut profiles = Vec::new();
    for entry in fs::read_dir(profile_dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".profraw"))
        {
            profiles.push(entry);
        }
    }
    let profile_metadata = profiles.first().map(|entry| entry.metadata()).transpose()?;
    ensure!(
        profiles.len() == 1
            && profile_metadata.is_some_and(|metadata| metadata.is_file() && metadata.len() > 0),
        "prepared packaging input did not emit one nonempty coverage profile"
    );
    drop(reservation);
    Ok(staged)
}

fn bounded_bootstrap_inventory(install_dir: &Path) -> String {
    fn stage_name(name: &str) -> bool {
        let Some(uuid) = name.strip_prefix(".kuru-install-") else {
            return false;
        };
        uuid.len() == 36
            && uuid.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
    }

    fn entry(path: &Path, name: &str) -> String {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                let kind = if metadata.file_type().is_file() {
                    "file"
                } else if metadata.file_type().is_dir() {
                    "directory"
                } else if metadata.file_type().is_symlink() {
                    "symlink"
                } else {
                    "other"
                };
                format!("{name}={kind}:{}", metadata.len())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                format!("{name}=absent")
            }
            Err(error) => format!("{name}=error:{:?}", error.kind()),
        }
    }

    let root = entry(install_dir, "install");
    let Ok(metadata) = fs::symlink_metadata(install_dir) else {
        return root;
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return root;
    }
    let mut entries = match fs::read_dir(install_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .take(BOOTSTRAP_INVENTORY_ENTRIES)
            .collect::<Vec<_>>(),
        Err(error) => return format!("{root}; entries=error:{:?}", error.kind()),
    };
    entries.sort_by_key(|entry| entry.file_name());
    let target = entry(&install_dir.join("kuru.exe"), "target");
    let stage = entries.into_iter().find(|entry| {
        entry.file_name().to_str().is_some_and(stage_name)
            && fs::symlink_metadata(entry.path()).is_ok_and(|metadata| {
                metadata.file_type().is_dir() && !metadata.file_type().is_symlink()
            })
    });
    let detail = stage.map_or_else(
        || "stage=absent; candidate=absent".to_owned(),
        |stage| {
            let name = stage.file_name();
            let name = name.to_string_lossy().chars().take(128).collect::<String>();
            let path = stage.path();
            let stage = entry(&path, &format!("stage:{name}"));
            let candidate = entry(&path.join("kuru.exe"), "candidate");
            format!("{stage}; {candidate}")
        },
    );
    let inventory = format!("{root}; {target}; {detail}");
    let mut end = inventory.len().min(BOOTSTRAP_INVENTORY_BYTES);
    while !inventory.is_char_boundary(end) {
        end -= 1;
    }
    inventory[..end].to_owned()
}

#[cfg(test)]
mod bootstrap_inventory_tests {
    use super::*;

    #[test]
    fn bootstrap_inventory_is_direct_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        let absent = bounded_bootstrap_inventory(&root.path().join("absent"));
        assert_eq!(absent, "install=absent");

        let install = root.path().join("install");
        fs::create_dir(&install).unwrap();
        fs::write(install.join("kuru.exe"), b"target").unwrap();
        let first = install.join(".kuru-install-00000000-0000-0000-0000-000000000000");
        let second = install.join(".kuru-install-11111111-1111-1111-1111-111111111111");
        fs::create_dir(install.join(".kuru-install-not-a-uuid")).unwrap();
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join("kuru.exe"), b"candidate").unwrap();
        let inventory = bounded_bootstrap_inventory(&install);
        assert!(inventory.contains("target=file:6"), "{inventory}");
        assert_eq!(
            inventory.matches("stage:.kuru-install-").count(),
            1,
            "{inventory}"
        );
        assert!(!inventory.contains("not-a-uuid"), "{inventory}");
        assert!(inventory.contains("candidate=file:9"), "{inventory}");
        assert!(inventory.len() <= BOOTSTRAP_INVENTORY_BYTES);

        let unsupported = root.path().join("ordinary-file");
        fs::write(&unsupported, b"").unwrap();
        assert!(
            bounded_bootstrap_inventory(&unsupported).starts_with("install=file:0"),
            "regular-file roots must not be opened as directories"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_inventory_does_not_follow_install_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("kuru.exe"), b"target").unwrap();
        let link = root.path().join("install-link");
        symlink(&target, &link).unwrap();
        let inventory = bounded_bootstrap_inventory(&link);
        assert!(inventory.starts_with("install=symlink:"), "{inventory}");
        assert!(!inventory.contains("target="), "{inventory}");
    }
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
    let mut checkpoint_offset = 0;
    for checkpoint in BOOTSTRAP_DIRECT_CHECKPOINTS {
        let Some(found) = stderr[checkpoint_offset..].find(checkpoint) else {
            anyhow::bail!(
                "stock PowerShell omitted direct bootstrap checkpoint {checkpoint:?}; stderr prefix: {:?}",
                String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(64 * 1024)])
            );
        };
        checkpoint_offset += found + checkpoint.len();
    }
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
        #[cfg(windows)]
        command.fixture_allow_independent_service();
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

    async fn retire_memory(&self) -> Result<()> {
        let scope = kuru_runtime::project_scope(&self.project)?;
        let mut options = OpenOptions::new(self.data.clone(), scope);
        options.config = self.memory.clone();
        options.supervisor = Some(self.binary.clone());
        kuru_memory::test_support::retire_idle_service(&options).await
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
            response::IntoResponse,
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
        ) -> axum::response::Response {
            let mut requests = state.lock().await;
            requests.completions += 1;
            requests.valid &= authorized(&headers)
                && body["model"] == "fixture-openai-model"
                && body["store"] == false
                && body["stream"] == true
                && headers
                    .get("accept")
                    .is_some_and(|value| value == "text/event-stream");
            if requests.reject {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error":{"message":"synthetic-installed-api-key"}})),
                )
                    .into_response();
            }
            let completed = serde_json::json!({
                "type":"response.completed",
                "response": {
                    "id":"installed-fixture-response",
                    "status":"completed",
                    "output":[{"type":"message","content":[{"type":"output_text","text":"Native OpenAI request completed."}]}],
                    "usage":{"input_tokens":1,"output_tokens":1}
                }
            });
            (
                StatusCode::OK,
                [("content-type", "text/event-stream")],
                format!("data: {completed}\n\n"),
            )
                .into_response()
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

// The install step's COMMAND_TIMEOUT bounds the install; it must not also
// have to absorb the stock PowerShell 5.1 engine's own cold-start cost
// (loading System.Management.Automation.dll, types.ps1xml/format.ps1xml,
// building the initial runspace), which happens before any script line runs
// and which -NoProfile/-NonInteractive do not skip. Evidence: 3/35 CI runs
// stalled at the very first powershell.exe invocation with zero stdout/stderr
// bytes over the full 100s window and the script's own first checkpoint line
// (gated only on -Verbose, always passed) never executed — see
// plan-flake-windows-installer.md. Giving cold start its own generous,
// distinctly labelled bound here, ahead of the timed install step, also
// primes the OS/Defender file-scan cache for powershell.exe's assemblies
// before the install invocation's own fresh COMMAND_TIMEOUT begins.
#[cfg(windows)]
const ENGINE_WARM_UP_TIMEOUT: Duration = Duration::from_secs(100);

#[cfg(windows)]
async fn warm_up_powershell_engine(
    powershell: &Path,
    root: &Path,
    system: &Path,
    isolated: &Path,
    empty_path: &Path,
) -> Result<()> {
    let mut command = Command::new(powershell);
    command
        .env_clear()
        .env(
            "SystemRoot",
            system.parent().context("Windows system root")?,
        )
        .env("PROCESSOR_ARCHITECTURE", "AMD64")
        .env("USERPROFILE", isolated)
        .env("APPDATA", isolated)
        .env("LOCALAPPDATA", isolated)
        .env("TMP", isolated)
        .env("TEMP", isolated)
        .env("PATH", empty_path)
        .current_dir(root)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "[Console]::Error.WriteLine('warm'); exit 0",
        ]);
    kuru_delivery::command::output(&mut command, ENGINE_WARM_UP_TIMEOUT)
        .await
        .context(
            "warm up the stock PowerShell 5.1 engine ahead of the timed install step \
             (distinct from and not counted against the install bound); a stall here \
             points at PowerShell engine/host cold start, not install.ps1",
        )?;
    Ok(())
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
    warm_up_powershell_engine(&powershell, root, &system, &isolated, &empty_path).await?;
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
    let output = match execute(&mut command).await {
        Ok(output) => output,
        Err(error) => {
            let inventory = bounded_bootstrap_inventory(install_dir);
            return Err(error).context(format!(
                "execute stock PowerShell package installation; bootstrap inventory: {inventory}"
            ));
        }
    };
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
    let explicit = std::env::var_os("KURU_EMBEDDED_TEST_BINARY");
    let selected = match explicit.as_ref() {
        Some(path) => {
            ensure!(
                Path::new(&path).is_absolute(),
                "KURU_EMBEDDED_TEST_BINARY must be an absolute path"
            );
            PathBuf::from(path)
        }
        None => PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
    };
    let binary = if explicit.is_none() && std::env::var_os("LLVM_PROFILE_FILE").is_some() {
        prepare_instrumented_packaging_input(root, &selected).await?
    } else {
        selected
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
    // Package the marked core's support from this exact selected executable,
    // including the private instrumented copy under coverage.
    let generated = root.join("generated shell support");
    for (name, args) in [
        ("completions/kuru.bash", &["completions", "bash"][..]),
        ("completions/_kuru", &["completions", "zsh"]),
        ("completions/kuru.fish", &["completions", "fish"]),
        ("completions/kuru.ps1", &["completions", "powershell"]),
        ("man/kuru.1", &["man"][..]),
    ] {
        let path = generated.join(name);
        fs::create_dir_all(path.parent().context("support file has no parent")?)?;
        let output = execute(Command::new(&binary).args(args)).await?;
        ensure!(
            output.status.success(),
            "selected binary could not generate {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(path, output.stdout)?;
    }
    let support_path = shell_support::package(&generated, target, version, &releases)?;
    let support_name = support_path
        .file_name()
        .and_then(OsStr::to_str)
        .context("shell support archive name")?;
    fs::write(
        releases.join("SHA256SUMS"),
        format!(
            "{}{}",
            fs::read_to_string(releases.join(format!("{name}.sha256")))?,
            fs::read_to_string(releases.join(format!("{support_name}.sha256")))?
        ),
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
        fs::read_dir(&install_dir)?.count() == 2 + usize::from(cfg!(windows)),
        "installation must contain Kuru, shell support and only the Windows update coordination directory when applicable"
    );
    let expected_support = shell_support::read_generated(&generated)?;
    ensure!(
        shell_support::read_generated(&install_dir.join("share/kuru").join(version).join(target))?
            == expected_support,
        "direct installation changed its paired shell support"
    );
    // The stable MANPATH file is a Unix direct-install contract. Stock Windows
    // PowerShell installs the exact five files in the versioned support tree.
    #[cfg(unix)]
    ensure!(
        fs::read(install_dir.join("share/man/man1/kuru.1"))?
            == expected_support
                .get("man/kuru.1")
                .context("generated man file")?,
        "direct installation changed its stable man page"
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
    first
        .retire_memory()
        .await
        .context("retire the direct installation's managed memory owner")?;

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
    ensure!(
        shell_support::read_generated(&install_dir.join("share/kuru").join(version).join(target))?
            == expected_support,
        "self-update changed its paired shell support"
    );
    #[cfg(unix)]
    ensure!(
        fs::read(install_dir.join("share/man/man1/kuru.1"))?
            == expected_support
                .get("man/kuru.1")
                .context("generated man file")?,
        "self-update changed its stable man page"
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
    second
        .retire_memory()
        .await
        .context("retire the updated installation's managed memory owner")?;
    ensure!(
        fs::read_dir(&install_dir)?.count() == 2 + usize::from(cfg!(windows)),
        "self-update left an unexpected companion alongside Kuru and shell support"
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
