use kuru_delivery::command::BlockingCommand as Command;
use serde_json::Value;
use std::{path::PathBuf, process::Output};

#[path = "support/memory.rs"]
mod memory;

struct Sandbox {
    root: tempfile::TempDir,
    project: PathBuf,
    data: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let data = root.path().join("data");
        std::fs::create_dir(&project).unwrap();
        memory::configuration(root.path()).unwrap();
        Self {
            root,
            project,
            data,
        }
    }
    fn command(&self) -> Command {
        self.command_for("demo")
    }
    fn command_for(&self, provider: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
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
fn cli_file_crud_and_shell_require_real_capabilities() {
    let env = Sandbox::new();
    let tools: Value = serde_json::from_str(&env.success(&["tools"])).unwrap();
    let shell_args = if cfg!(windows) {
        r#"{"command":"[Console]::Write('shell-ok')"}"#
    } else {
        r#"{"command":"printf shell-ok"}"#
    };
    assert!(
        tools
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
    assert!(
        env.success(&["--allow-shell", "tool", "shell", "--args", shell_args])
            .contains("shell-ok")
    );
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

#[cfg(unix)]
#[test]
fn supported_auth_commands_forward_to_native_codex_without_handling_tokens() {
    use std::os::unix::fs::PermissionsExt;
    let env = Sandbox::new();
    let script = env.root.path().join("fake-codex");
    let log = env.root.path().join("auth-actions");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$AUTH_TEST_LOG\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = env.root.path().join("auth.toml");
    std::fs::write(
        &config,
        format!("codex_command = {:?}\n", script.display().to_string()),
    )
    .unwrap();
    for args in [
        vec!["login"],
        vec!["login", "--device"],
        vec!["auth"],
        vec!["logout"],
    ] {
        let output = env
            .command()
            .arg("--config")
            .arg(&config)
            .env("AUTH_TEST_LOG", &log)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "login\nlogin --device-auth\nlogin status\nlogout\n"
    );
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

#[tokio::test]
async fn sequential_commands_reap_owned_memory_before_returning_on_success_or_error() {
    use kuru_memory::MemoryStore;
    use serde_json::json;

    let env = Sandbox::new();
    let scope = kuru_runtime::project_scope(&env.project).unwrap();
    let store_path = env
        .data
        .join("memory")
        .join(scope.strip_prefix("project/").unwrap());
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
    stopped();
    for args in [
        vec!["config"],
        vec!["sessions"],
        vec!["models"],
        vec!["tools"],
        vec!["memory", "status"],
        vec!["memory", "history", "--limit", "1"],
    ] {
        env.success(&args);
        stopped();
    }
    for (args, diagnostic) in [
        (
            vec!["memory", "history", "--limit", "0"],
            "between 1 and 1000",
        ),
        (
            vec!["tool", "file_read", "--args", "malformed"],
            "expected value",
        ),
    ] {
        let output = env.run(&args);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        stopped();
    }
    let output = env
        .command_for("unknown-provider")
        .arg("models")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("provider must be"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stopped();
    let options = kuru_memory::test_support::open_options(env.data.clone(), scope.clone()).unwrap();
    let memory = MemoryStore::open(options.clone()).await.unwrap();
    memory
        .put(
            &format!("{scope}/preferences"),
            &json!({"mode":"not-a-framework"}),
        )
        .await
        .unwrap();
    memory.close().await.unwrap();
    let output = env.run(&["config"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("preferences"));
    stopped();
    let memory = MemoryStore::open(options).await.unwrap();
    memory
        .put(&format!("{scope}/preferences"), &json!({}))
        .await
        .unwrap();
    memory.close().await.unwrap();
    let invalid_config = env.root.path().join("invalid-selection.toml");
    std::fs::write(&invalid_config, "max_parts = 1\n").unwrap();
    let output = env
        .command()
        .arg("--config")
        .arg(&invalid_config)
        .arg("config")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("parts"));
    stopped();
    env.success(&["run", "The next command still works"]);
    stopped();
}
