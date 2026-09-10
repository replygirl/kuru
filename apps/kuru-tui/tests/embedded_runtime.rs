#![cfg(unix)]
//! Compiler-free installed-runtime acceptance. This fixture sets offline=true
//! and removes PATH tools; it does not install an OS egress firewall. Runtime
//! download removal is also checked in the owning memory package/source review.

use anyhow::{Context, Result, ensure};
use kuru_core::MemoryConfig;
use kuru_delivery::archive;
use kuru_memory::{MemoryStore, OpenOptions};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const OUTPUT_LIMIT: u64 = 1024 * 1024;
// Cold creation starts staging and active servers, each with the configured
// 30-second bound, then includes their handshakes and bounded shutdowns.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(100);
const MANIFEST: &str = include_str!("../../../packages/kuru-memory/support/dolt-assets.json");

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
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_CACHE_HOME", self.home.join("cache"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("TMPDIR", &self.temporary)
            .env("PATH", &self.empty_path)
            .current_dir(&self.project)
            .arg("-C")
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.data)
            .args(["--provider", "demo", "--mode", "freudian", "--no-dream"]);
        // Preserve only the coverage destination, never ambient provider state.
        if let Some(path) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", path);
        }
        command
    }
    async fn run(&self, args: &[&str]) -> Result<Value> {
        let output = execute(self.command().args(args)).await?;
        ensure!(
            output.status.success(),
            "installed command {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).context("installed command did not return JSON")
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
                .any(|message| message.role == "user" && message.content == marker),
            "original user message was not persisted"
        );
        ensure!(
            transcript.iter().any(|message| message.role == "user"
                && message.content == "Continue the saved conversation"),
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
            ("dolt", "executable_bytes", "executable_sha256"),
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
    let archive_path = archive::package(&binary, target, version, &releases)?;
    let name = archive_path
        .file_name()
        .and_then(OsStr::to_str)
        .context("archive name")?;
    fs::copy(
        releases.join(format!("{name}.sha256")),
        releases.join("SHA256SUMS"),
    )?;
    let installed = archive::install(
        releases.to_str().context("local release path")?,
        version,
        &install_dir,
        Some(target),
    )
    .await?;
    let original_digest = digest(&binary)?;
    ensure!(
        digest(&installed)? == original_digest,
        "direct installation changed the packaged executable"
    );
    ensure!(
        fs::read_dir(&install_dir)?.count() == 1,
        "installation must require only the Kuru executable"
    );
    let first = Installation::new(root, "direct", &project, &installed)?;
    let reported = execute(first.command().arg("--version")).await?;
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
        .await?;

    let previous_inode = fs::metadata(&installed)?.ino();
    let output = execute(
        first
            .command()
            .args(["update", "--version", version, "--release-base"])
            .arg(&releases),
    )
    .await?;
    ensure!(
        output.status.success(),
        "native self-update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        fs::metadata(&installed)?.ino() != previous_inode,
        "self-update did not replace the installed executable"
    );
    ensure!(
        digest(&installed)? == original_digest,
        "self-update lost or changed the bundled executable"
    );
    let second = Installation::new(root, "updated", &project, &installed)?;
    second
        .conversation(
            "Remember the offline self-update marker: indigo-942",
            asset,
            dolt_version,
        )
        .await?;
    ensure!(
        fs::read_dir(&install_dir)?.count() == 1,
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
    let root = tempfile::Builder::new()
        .prefix("kuru-embedded-acceptance-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    if let Err(error) = packaged_roundtrip(root.path()).await {
        // Keep private diagnostics on failure; a timeout must never remove a
        // directory while an owned SQL supervisor may still be completing EOF.
        let path = root.keep();
        panic!(
            "{error:#}\nprivate embedded-runtime fixture retained at {}",
            path.display()
        );
    }
}
