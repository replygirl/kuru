use std::{fs, io::Write, path::Path, process::Output};

use anyhow::{Result, ensure};
use kuru_connectors::AuthManager;
use kuru_delivery::command::BlockingCommand as Command;
use kuru_memory::MemoryStore;
use serde_json::Value;

fn launch(project: &Path, data: &Path, config: &Path, args: &[&str]) -> Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
    command
        .arg("-C")
        .arg(project)
        .arg("--data-dir")
        .arg(data)
        .arg("doctor")
        .args(args)
        .env("XDG_CONFIG_HOME", config)
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .env_remove("KURU_DOCTOR_FAKE_KEY")
        .env_remove("KURU_DOCTOR_REPO_KEY")
        .env_remove("KURU_DOCTOR_COLD_KEY");
    Ok(command.output()?)
}

fn report(output: &Output, exit: i32) -> Result<Value> {
    ensure!(
        output.status.code() == Some(exit),
        "doctor exit mismatch: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(report["schema_version"] == 1);
    ensure!(report["exit_code"] == exit);
    Ok(report)
}

fn check<'a>(report: &'a Value, id: &str) -> Result<&'a Value> {
    report["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|check| check["id"] == id))
        .ok_or_else(|| anyhow::anyhow!("missing doctor check {id}"))
}

#[test]
fn doctor_reports_fixed_local_states_without_creating_data_or_leaking_values() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let project = sandbox.path().join("private-project");
    let data = sandbox.path().join("private-data");
    let config_home = sandbox.path().join("config-home");
    fs::create_dir(&project)?;

    let fresh = launch(&project, &data, &config_home, &["--json"])?;
    let fresh = report(&fresh, 3)?;
    ensure!(check(&fresh, "chatgpt_subscription")?["state"] == "problem");
    ensure!(check(&fresh, "chatgpt_subscription")?["action"] == "sign_in");
    ensure!(check(&fresh, "chatgpt_subscription")?["condition"] == "signed_out");
    ensure!(check(&fresh, "responses_route")?["condition"] == "route_not_selected");
    ensure!(check(&fresh, "memory")?["condition"] == "not_initialized");
    ensure!(check(&fresh, "embedded_engine")?["condition"] == "payload_consistent");
    ensure!(!data.exists(), "doctor created absent data state");

    assert_private_human_report(
        &launch(&project, &data, &config_home, &[])?,
        3,
        &[
            "ChatGPT subscription: Problem — no Kuru sign-in is stored",
            "Run `kuru login`",
        ],
    )?;
    ensure!(!data.exists(), "human doctor created absent data state");

    fs::create_dir_all(config_home.join("kuru"))?;
    fs::write(
        config_home.join("kuru/config.toml"),
        "provider = 'responses'\napi_key_env = 'KURU_DOCTOR_FAKE_KEY'\n",
    )?;
    let configured = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .arg("-C")
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args(["doctor", "--json"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("KURU_DOCTOR_FAKE_KEY", "DOCTOR_SECRET_SENTINEL")
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .output()?;
    let configured = report(&configured, 0)?;
    ensure!(check(&configured, "responses_route")?["condition"] == "route_variable_present");
    ensure!(
        !String::from_utf8(serde_json::to_vec(&configured)?)?.contains("DOCTOR_SECRET_SENTINEL")
    );
    ensure!(!data.exists(), "doctor created absent data state");

    let missing_key = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .arg("-C")
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args(["doctor", "--json"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env_remove("KURU_DOCTOR_FAKE_KEY")
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .output()?;
    let missing_key = report(&missing_key, 3)?;
    ensure!(check(&missing_key, "responses_route")?["state"] == "problem");
    ensure!(check(&missing_key, "responses_route")?["action"] == "configure_responses");

    let human = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .arg("-C")
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args(["doctor"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("KURU_DOCTOR_FAKE_KEY", "DOCTOR_SECRET_SENTINEL")
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .output()?;
    ensure!(human.status.success());
    let human = String::from_utf8(human.stdout)?;
    ensure!(human.contains("ChatGPT subscription: Not configured or not selected"));
    ensure!(human.contains("Responses API route: OK"));
    ensure!(human.contains("API access was not tested"));
    ensure!(human.contains("environment variable is non-empty"));
    ensure!(!human.contains("DOCTOR_SECRET_SENTINEL"));
    ensure!(!human.contains("private-project"));
    Ok(())
}

#[test]
fn repository_responses_route_is_unverified_until_workspace_approval() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let project = sandbox.path().join("private-project");
    let data = sandbox.path().join("private-data");
    let config_home = sandbox.path().join("config-home");
    fs::create_dir(&project)?;
    fs::create_dir(project.join(".kuru"))?;
    fs::write(
        project.join(".kuru/config.toml"),
        "provider = 'responses'\napi_key_env = 'KURU_DOCTOR_REPO_KEY'\n",
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .arg("-C")
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args(["doctor", "--json"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("KURU_DOCTOR_REPO_KEY", "UNAPPROVED_KEY_SENTINEL")
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .output()?;
    let untrusted_report = report(&output, 2)?;
    ensure!(check(&untrusted_report, "workspace_trust")?["condition"] == "unapproved");
    ensure!(check(&untrusted_report, "responses_route")?["condition"] == "route_unverified");
    ensure!(check(&untrusted_report, "responses_route")?["state"] == "unverified");
    ensure!(check(&untrusted_report, "chatgpt_subscription")?["condition"] == "signed_out");
    let output = String::from_utf8(output.stdout)?;
    ensure!(!output.contains("UNAPPROVED_KEY_SENTINEL"));
    ensure!(!data.exists(), "untrusted doctor created data state");

    let approved_once = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(["--trust-workspace-once", "-C"])
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args(["doctor", "--json"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("KURU_DOCTOR_REPO_KEY", "UNAPPROVED_KEY_SENTINEL")
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .output()?;
    let approved_once = report(&approved_once, 0)?;
    ensure!(check(&approved_once, "workspace_trust")?["condition"] == "unapproved");
    ensure!(check(&approved_once, "responses_route")?["condition"] == "route_variable_present");
    ensure!(
        !String::from_utf8(serde_json::to_vec(&approved_once)?)?
            .contains("UNAPPROVED_KEY_SENTINEL")
    );
    ensure!(!data.exists(), "one-invocation doctor created data state");
    Ok(())
}

#[test]
fn invalid_configuration_and_corrupt_activation_are_redacted_and_unchanged() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let project = sandbox.path().join("private-project");
    let data = sandbox.path().join("private-data");
    let config_home = sandbox.path().join("config-home");
    fs::create_dir(&project)?;
    fs::create_dir(project.join(".kuru"))?;
    fs::write(
        project.join(".kuru/config.toml"),
        "unknown_doctor_key = 'CONFIG_SECRET_SENTINEL'\n",
    )?;
    let invalid = launch(&project, &data, &config_home, &["--json"])?;
    let invalid_report = report(&invalid, 3)?;
    ensure!(check(&invalid_report, "configuration")?["condition"] == "configuration_invalid");
    ensure!(check(&invalid_report, "workspace_trust")?["state"] == "unverified");
    for output in [&invalid.stdout, &invalid.stderr] {
        let output = String::from_utf8_lossy(output);
        ensure!(!output.contains("CONFIG_SECRET_SENTINEL"));
        ensure!(!output.contains("private-project"));
        ensure!(!output.contains("private-data"));
    }
    ensure!(!data.exists(), "invalid config doctor created data state");

    assert_private_human_report(
        &launch(&project, &data, &config_home, &[])?,
        3,
        &[
            "Configuration: Problem — configuration is invalid",
            "Correct the local configuration",
        ],
    )?;

    fs::remove_file(project.join(".kuru/config.toml"))?;
    fs::create_dir(project.join(".kuru/config.toml"))?;
    let unreadable = launch(&project, &data, &config_home, &["--json"])?;
    let unreadable_report = report(&unreadable, 2)?;
    ensure!(check(&unreadable_report, "configuration")?["condition"] == "configuration_unreadable");
    ensure!(check(&unreadable_report, "configuration")?["state"] == "unverified");
    for output in [&unreadable.stdout, &unreadable.stderr] {
        let output = String::from_utf8_lossy(output);
        ensure!(!output.contains("private-project"));
        ensure!(!output.contains("private-data"));
    }
    ensure!(
        !data.exists(),
        "unreadable config doctor created data state"
    );

    assert_private_human_report(
        &launch(&project, &data, &config_home, &[])?,
        2,
        &[
            "Configuration: Unverified — configuration could not be inspected reliably",
            "Retry the local check",
        ],
    )?;

    fs::remove_dir(project.join(".kuru/config.toml"))?;
    fs::write(project.join(".kuru/config.toml"), "")?;
    fs::create_dir_all(config_home.join("kuru"))?;
    fs::write(
        config_home.join("kuru/config.toml"),
        "provider = 'responses'\napi_key_env = 'KURU_DOCTOR_COLD_KEY'\n",
    )?;
    let scope = kuru_runtime::project_scope(&project.canonicalize()?)?;
    let hash = scope.strip_prefix("project/").expect("project scope");
    let memory = data.join("memory").join(hash);
    let data_dir = kuru_platform::fs::Directory::ensure_private(&data)?;
    drop(data_dir);
    let memory_parent = kuru_platform::fs::Directory::ensure_private(&data.join("memory"))?;
    drop(memory_parent);
    let directory = kuru_platform::fs::Directory::ensure_private(&memory)?;
    let mut ready = directory.create_new(std::ffi::OsStr::new("ready.json"))?;
    let activation = serde_json::json!({
        "format": 1,
        "project_scope": scope,
        "initial_revision": "00000000000000000000000000000000",
        "migration": null,
    });
    ready.write_all(&serde_json::to_vec(&activation)?)?;
    ready.sync_all()?;
    drop(ready);
    drop(directory);
    let before = fs::read(memory.join("ready.json"))?;

    let cold = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .arg("-C")
        .arg(&project)
        .arg("--data-dir")
        .arg(&data)
        .args(["doctor", "--json"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("KURU_DOCTOR_COLD_KEY", "DOCTOR_COLD_KEY_SENTINEL")
        .env_remove("KURU_MANAGED_CONFIG")
        .env_remove("OPENAI_API_KEY")
        .output()?;
    let cold_report = report(&cold, 2)?;
    ensure!(check(&cold_report, "memory")?["state"] == "unverified");
    ensure!(check(&cold_report, "memory")?["condition"] == "cold_health_unchecked");
    ensure!(fs::read(memory.join("ready.json"))? == before);
    ensure!(!data.join("memory/services").exists());
    ensure!(!data.join("tools").exists());

    assert_private_human_report(
        &launch(&project, &data, &config_home, &[])?,
        3,
        &[
            "Project memory: Unverified — project metadata is valid; cold database health was not checked",
            "Open the project normally",
        ],
    )?;
    ensure!(fs::read(memory.join("ready.json"))? == before);

    let directory = kuru_platform::fs::Directory::open(
        &memory,
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Pinned,
    )?;
    let mut marker = directory.read_write(std::ffi::OsStr::new("ready.json"))?;
    marker.set_len(0)?;
    marker.write_all(b"{CORRUPT_MEMORY_SENTINEL")?;
    marker.sync_all()?;
    drop(marker);
    let before = fs::read(memory.join("ready.json"))?;

    let corrupt = launch(&project, &data, &config_home, &["--json"])?;
    let corrupt_report = report(&corrupt, 3)?;
    ensure!(check(&corrupt_report, "memory")?["condition"] == "activation_invalid");
    ensure!(fs::read(memory.join("ready.json"))? == before);
    ensure!(!data.join("tools").exists());
    ensure!(!data.join("memory/services").exists());
    for output in [&corrupt.stdout, &corrupt.stderr] {
        ensure!(!String::from_utf8_lossy(output).contains("CORRUPT_MEMORY_SENTINEL"));
    }
    assert_private_human_report(
        &launch(&project, &data, &config_home, &[])?,
        3,
        &[
            "Project memory: Problem — project activation metadata is invalid",
            "See the troubleshooting guide",
        ],
    )?;
    ensure!(fs::read(memory.join("ready.json"))? == before);
    Ok(())
}

#[test]
fn invocation_failure_uses_fixed_redacted_report() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let missing_project = sandbox.path().join("missing-project");
    let data = sandbox.path().join("private-data");
    let config_home = sandbox.path().join("config-home");
    let output = launch(&missing_project, &data, &config_home, &["--json"])?;
    let report = report(&output, 1)?;
    ensure!(check(&report, "invocation")?["condition"] == "invocation_invalid");
    ensure!(!String::from_utf8(output.stdout)?.contains("missing-project"));
    assert_private_human_report(
        &launch(&missing_project, &data, &config_home, &[])?,
        1,
        &[
            "Invocation: Problem — the workspace or data directory could not be resolved",
            "Check `-C`, `--data-dir`",
        ],
    )?;
    ensure!(
        !data.exists(),
        "human invocation failure created data state"
    );
    Ok(())
}

#[tokio::test]
async fn doctor_reports_synthetic_kuru_subscription_without_exposing_identity() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let sandbox = tempfile::tempdir()?;
        let project = sandbox.path().join("private-project");
        let data = sandbox.path().join("private-data");
        let config_home = sandbox.path().join("config-home");
        fs::create_dir(&project)?;
        fs::create_dir_all(config_home.join("kuru"))?;
        fs::write(config_home.join("kuru/config.toml"), "provider = 'codex'\n")?;

        AuthManager::seed_test_subscription_session(data.clone(), project.clone()).await?;
        let auth = AuthManager::new(data.clone(), project.clone(), None)?;
        let before = auth.status().await?;
        ensure!(before.authenticated);

        let output = launch(&project, &data, &config_home, &["--json"])?;
        let report = report(&output, 0)?;
        ensure!(check(&report, "chatgpt_subscription")?["state"] == "healthy");
        ensure!(check(&report, "chatgpt_subscription")?["condition"] == "credentials_present");
        ensure!(check(&report, "chatgpt_subscription")?["action"] == "none");
        let serialized = String::from_utf8(output.stdout)?;
        ensure!(!serialized.contains("synthetic-canary-account"));
        ensure!(!serialized.contains("synthetic-canary-access-token"));

        let after = auth.status().await?;
        ensure!(after.authenticated == before.authenticated);
        ensure!(after.expires_at == before.expires_at);
        ensure!(after.account_id == before.account_id);
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doctor_closes_only_its_read_only_view_of_a_live_memory_owner() -> Result<()> {
    kuru_memory::test_support::closing(async {
        let root = kuru_memory::test_support::tempdir()?;
        let project = root.path().join("project");
        let data = root.path().join("data");
        let config_home = root.path().join("config-home");
        fs::create_dir(&project)?;
        let project = project.canonicalize()?;
        let scope = kuru_runtime::project_scope(&project)?;
        let options = kuru_memory::test_support::warmed_open_options(data.clone(), scope).await?;
        let owner = MemoryStore::open_managed_observed(
            options.clone(),
            project.clone(),
            std::path::PathBuf::from(env!("CARGO_BIN_EXE_kuru")),
        )
        .1
        .await?;

        let child_project = project.clone();
        let child_data = data.clone();
        let child_config = config_home.clone();
        let output = tokio::task::spawn_blocking(move || {
            launch(&child_project, &child_data, &child_config, &["--json"])
        })
        .await??;
        let report = report(&output, 3)?;
        ensure!(check(&report, "memory")?["state"] == "healthy");
        ensure!(check(&report, "memory")?["condition"] == "live_owner_checked");

        // Closing the doctor's short-lived attachment must not retire a
        // service that still has its original owner client.
        owner.status().await?;
        owner.close().await?;
        kuru_memory::test_support::await_managed_quiescence(&options).await?;
        Ok(())
    })
    .await
}

fn assert_private_human_report(output: &Output, exit: i32, expected: &[&str]) -> Result<()> {
    ensure!(output.status.code() == Some(exit), "{output:?}");
    let human = String::from_utf8_lossy(&output.stdout);
    ensure!(human.contains("Kuru doctor (schema 1)"));
    ensure!(human.contains(&format!("Exit code {exit}:")));
    for fragment in expected {
        ensure!(human.contains(fragment), "missing {fragment:?}: {human}");
    }
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for private in [
            "private-project",
            "private-data",
            "missing-project",
            "config-home",
            "CONFIG_SECRET_SENTINEL",
            "CORRUPT_MEMORY_SENTINEL",
            "DOCTOR_COLD_KEY_SENTINEL",
            "APPROVAL_KEY_SENTINEL",
            "CORRUPT_APPROVAL_SENTINEL",
        ] {
            ensure!(!text.contains(private), "doctor disclosed {private}");
        }
    }
    Ok(())
}

#[test]
fn doctor_rechecks_stored_approval_without_granting_changed_or_corrupt_authority() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let project = sandbox.path().join("private-project");
    let data = sandbox.path().join("private-data");
    let config_home = sandbox.path().join("config-home");
    fs::create_dir(&project)?;
    fs::create_dir(project.join(".kuru"))?;
    let config = project.join(".kuru/config.toml");
    fs::write(
        &config,
        "provider = 'responses'\napi_key_env = 'KURU_DOCTOR_REPO_KEY'\n",
    )?;
    let run = |args: &[&str]| -> Result<Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_kuru"))
            .arg("-C")
            .arg(&project)
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("KURU_DOCTOR_REPO_KEY", "APPROVAL_KEY_SENTINEL")
            .env_remove("KURU_MANAGED_CONFIG")
            .env_remove("OPENAI_API_KEY")
            .output()?)
    };
    let approved = run(&["trust", "approve", "--yes"])?;
    ensure!(approved.status.success(), "{approved:?}");
    let records: Vec<_> = fs::read_dir(data.join("trust/workspaces"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|value| value == "json"))
        .collect();
    ensure!(
        records.len() == 1,
        "approval must create exactly one private record"
    );
    let directory = kuru_platform::fs::Directory::open(
        records[0].parent().expect("approval parent"),
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Movable,
    )?;
    let name = records[0].file_name().expect("approval record name");
    let before = fs::read(&records[0])?;
    let approved_report = report(&run(&["doctor", "--json"])?, 0)?;
    ensure!(!serde_json::to_string(&approved_report)?.contains("APPROVAL_KEY_SENTINEL"));
    ensure!(check(&approved_report, "workspace_trust")?["condition"] == "approved");
    ensure!(check(&approved_report, "responses_route")?["condition"] == "route_variable_present");
    assert_private_human_report(
        &run(&["doctor"])?,
        0,
        &["Workspace trust: OK — the captured workspace manifest is approved"],
    )?;
    ensure!(fs::read(&records[0])? == before);

    fs::write(
        &config,
        "provider = 'responses'\napi_key_env = 'KURU_DOCTOR_REPO_KEY'\nallow_shell = true\n",
    )?;
    let stale = report(&run(&["doctor", "--json"])?, 2)?;
    ensure!(!serde_json::to_string(&stale)?.contains("APPROVAL_KEY_SENTINEL"));
    ensure!(check(&stale, "workspace_trust")?["condition"] == "approval_changed");
    ensure!(check(&stale, "responses_route")?["condition"] == "route_unverified");
    assert_private_human_report(
        &run(&["doctor"])?,
        2,
        &[
            "stored approval does not match the captured manifest",
            "Review the exact workspace manifest",
        ],
    )?;
    ensure!(
        fs::read(&records[0])? == before,
        "doctor rewrote stale approval"
    );

    directory.remove_file(name, directory.read(name)?)?;
    let mut record = directory.create_new(name)?;
    record.write_all(b"{CORRUPT_APPROVAL_SENTINEL")?;
    kuru_platform::fs::seal_private(&record, false)?;
    record.sync_all()?;
    drop(record);
    let corrupt_before = fs::read(&records[0])?;
    let invalid_output = run(&["doctor", "--json"])?;
    let invalid = report(&invalid_output, 3)?;
    ensure!(check(&invalid, "workspace_trust")?["condition"] == "approval_invalid");
    ensure!(check(&invalid, "workspace_trust")?["state"] == "problem");
    ensure!(check(&invalid, "responses_route")?["condition"] == "route_unverified");
    for bytes in [&invalid_output.stdout, &invalid_output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        ensure!(!text.contains("APPROVAL_KEY_SENTINEL"));
        ensure!(!text.contains("CORRUPT_APPROVAL_SENTINEL"));
    }
    assert_private_human_report(
        &run(&["doctor"])?,
        3,
        &[
            "Workspace trust: Problem — stored approval is invalid",
            "Review the exact workspace manifest",
        ],
    )?;
    ensure!(
        fs::read(&records[0])? == corrupt_before,
        "doctor repaired corrupt evidence"
    );
    ensure!(!data.join("memory").exists());
    ensure!(!data.join("auth").exists());
    ensure!(!data.join("tools").exists());
    Ok(())
}
