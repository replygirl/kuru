//! Completed standalone invocations share checked storage without sharing drivers.

use anyhow::{Context as _, Result, ensure};
use kuru_delivery::command::{Command, bounded_output};
use kuru_memory::service::{EndpointRecord, ServiceCall, ServiceValue, attach_or_start};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{ffi::OsStr, io::Read as _, path::Path, time::Duration};

#[path = "support/memory.rs"]
mod memory;

async fn invoke(
    project: &Path,
    data: &Path,
    cleanup: &memory::ServiceCleanup,
    args: &[&str],
) -> Result<Value> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
    command.env_clear().current_dir(project);
    // Keep native launch requirements and instrumentation, while excluding
    // ambient managed configuration, provider credentials and fixture hooks.
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "SystemDrive",
        "LLVM_PROFILE_FILE",
        "KURU_COVERAGE_SPAWN_LEDGER",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HOME", cleanup.path().join("home"))
        .env("USERPROFILE", cleanup.path().join("home"))
        .env("APPDATA", cleanup.path().join("home/Roaming"))
        .env("LOCALAPPDATA", cleanup.path().join("home/Local"))
        .env("TMPDIR", cleanup.path().join("temporary"))
        .env("TEMP", cleanup.path().join("temporary"))
        .env("TMP", cleanup.path().join("temporary"));
    #[cfg(unix)]
    command.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    command.fixture_allow_independent_service();
    command
        .arg("-C")
        .arg(project)
        .arg("--data-dir")
        .arg(data)
        .args(["--provider", "demo", "--no-dream"])
        .env("XDG_CONFIG_HOME", cleanup.path().join("config"))
        .env(
            kuru_memory::test_support::OWNER_DIAGNOSTIC_ENV,
            cleanup.owner_diagnostic_path(),
        )
        .args(args);
    let output = bounded_output(&mut command, Duration::from_secs(120), 1024 * 1024).await?;
    ensure!(
        output.status.success(),
        "standalone {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).context("parse completed standalone output")
}

/// Read only the private engine's bounded non-secret endpoint identity. The
/// service generation is authenticated separately through an actual RPC.
fn engine_identity(data: &Path, scope: &str) -> Result<(String, u16)> {
    let digest = scope
        .strip_prefix("project/")
        .context("canonical project scope")?;
    let directory = Directory::open(
        &data.join("memory").join(digest),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    let file = directory.read(OsStr::new("endpoint.json"))?;
    ensure!(
        file.metadata()?.len() <= 16 * 1024,
        "engine endpoint exceeds fixture bound"
    );
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024,
        "engine endpoint grew beyond fixture bound"
    );
    let endpoint: Value = serde_json::from_slice(&bytes)?;
    let instance = endpoint["instance"]
        .as_str()
        .context("engine endpoint instance")?
        .to_owned();
    let port = u16::try_from(endpoint["port"].as_u64().context("engine endpoint port")?)?;
    ensure!(port != 0, "engine endpoint has no bound port");
    Ok((instance, port))
}

#[tokio::test]
async fn completed_standalone_calls_reuse_service_and_engine_then_retire_and_restart() -> Result<()>
{
    kuru_memory::test_support::closing(async {
        let cache = kuru_memory::test_support::warmed_cache_dir().await?;
        let root = kuru_memory::test_support::tempdir()?;
        std::fs::create_dir(root.path().join("home"))?;
        std::fs::create_dir(root.path().join("temporary"))?;
        memory::configuration_with(root.path(), &cache)?;
        let config_path = root.path().join("config/kuru/config.toml");
        let mut config: toml::Table = toml::from_str(&std::fs::read_to_string(&config_path)?)?;
        // Exercise the actual product default, not a test-build or ambient policy.
        config
            .get_mut("memory")
            .and_then(toml::Value::as_table_mut)
            .context("fixture memory table")?
            .remove("service_idle_timeout_secs");
        std::fs::write(&config_path, toml::to_string(&config)?)?;
        let project = root.path().join("project");
        std::fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let data = root.path().join("data");
        let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
        let scope = format!(
            "project/{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let mut options = kuru_memory::OpenOptions::new(data.clone(), scope.clone());
        options.config.cache_dir = Some(cache);
        options.config.offline = true;
        let idle = Duration::from_secs(options.config.service_idle_timeout_secs);
        ensure!(
            idle == Duration::from_secs(30),
            "standalone fixture did not select default retention"
        );
        let cleanup = memory::ServiceCleanup::new(root, &data);
        let outcome: Result<()> = async {
            let mut expected = None;
            let mut sessions = Vec::new();
            let mut last_detach = tokio::time::Instant::now();
            for prompt in ["retained first", "retained second", "retained third"] {
                // This subprocess has completed and been reaped before inspection
                // and before the next separate invocation starts.
                let answer = invoke(&project, &data, &cleanup, &["run", prompt, "--json"]).await?;
                ensure!(
                    answer["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("demo")),
                    "standalone response did not settle"
                );
                let session = answer["session"]
                    .as_str()
                    .context("settled session")?
                    .to_owned();
                ensure!(
                    !sessions.contains(&session),
                    "separate invocation reused a driver session"
                );
                sessions.push(session);
                let record = EndpointRecord::read(&data, &scope)?
                    .context("completed command did not retain service discovery")?;
                let mut attachment =
                    attach_or_start(&options, &project, Path::new(env!("CARGO_BIN_EXE_kuru")))
                        .await?;
                ensure!(
                    attachment.generation() == record.authority.service_generation,
                    "inspection started a successor instead of attaching to retained service"
                );
                ensure!(
                    matches!(
                        attachment.call(ServiceCall::Revision).await?,
                        ServiceValue::Revision(_)
                    ),
                    "retained service did not authenticate and answer"
                );
                let engine = engine_identity(&data, &scope)?;
                ensure!(
                    engine.0 == record.authority.store_instance,
                    "retained engine differs from authenticated store"
                );
                let identity = (attachment.generation().to_owned(), engine);
                if let Some(expected) = &expected {
                    ensure!(
                        &identity == expected,
                        "completed standalone calls restarted service or engine"
                    );
                } else {
                    expected = Some(identity);
                }
                last_detach = tokio::time::Instant::now();
                attachment.close();
                // No inspection or driver attachment remains across the gap.
            }
            tokio::time::timeout(
                idle + Duration::from_secs(60),
                kuru_memory::test_support::await_owner_release(&options),
            )
            .await
            .context("retained service did not release its owner after idle expiry")??;
            ensure!(
                last_detach.elapsed() >= idle,
                "service retired before its configured idle interval"
            );
            ensure!(
                EndpointRecord::read(&data, &scope)?.is_none(),
                "released owner retained discovery"
            );
            let engine_directory = data
                .join("memory")
                .join(scope.strip_prefix("project/").unwrap());
            ensure!(
                !engine_directory.join("endpoint.json").try_exists()?,
                "owner released before engine endpoint retirement"
            );
            let restarted = invoke(
                &project,
                &data,
                &cleanup,
                &["run", "after retirement", "--json"],
            )
            .await?;
            ensure!(
                restarted["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("demo")),
                "successor did not complete"
            );
            let successor =
                EndpointRecord::read(&data, &scope)?.context("successor endpoint missing")?;
            let expected = expected.context("initial service identity")?;
            ensure!(
                successor.authority.service_generation != expected.0,
                "expired generation was reused"
            );
            eprintln!("retained standalone storage: 3 completed calls; service={}; engine_instance={} engine_port={}; idle_release={:?}; successor={}",
                expected.0, expected.1.0, expected.1.1, last_detach.elapsed(), successor.authority.service_generation);
            let catalog = invoke(&project, &data, &cleanup, &["sessions"]).await?;
            let catalog = catalog.as_array().context("public session catalog")?;
            for session in sessions {
                ensure!(
                    catalog.iter().any(|row| row["session_id"] == session),
                    "retirement lost a completed session"
                );
            }
            Ok(())
        }
        .await;
        cleanup.release(outcome)
    })
    .await
}
