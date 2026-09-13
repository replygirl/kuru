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
        assert_eq!(result.as_object().unwrap().len(), 8);
        for field in [
            "session",
            "speaker",
            "text",
            "relationship",
            "input_tokens",
            "output_tokens",
            "limited",
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
    assert_eq!(std::fs::read(path).unwrap(), original);
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
        String::from_utf8_lossy(&output.stderr).contains("configuration validation error"),
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
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("preferences are omitted"));
    toml::from_str::<toml::Value>(&String::from_utf8(output.stdout).unwrap()).unwrap();
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
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("preferences are omitted"));
    let output = env
        .command()
        .arg("--config")
        .arg(&invalid_config)
        .args(["run", "invalid effective topology"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    stopped();
    env.success(&["run", "The next command still works"]);
    stopped();
}
