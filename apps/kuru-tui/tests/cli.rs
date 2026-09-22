use kuru_delivery::command::BlockingCommand as Command;
use serde_json::Value;
use std::{path::PathBuf, process::Output};

#[path = "support/memory.rs"]
mod memory;

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
    root: tempfile::TempDir,
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

fn warm_memory_progress() -> &'static str {
    concat!(
        "Memory: waiting for project ownership…\n",
        "Memory: verifying cached runtime…\n",
        "Memory: checking runtime version…\n",
        "Memory: preparing database…\n",
        "Memory: opening database…\n",
        "Memory: ready.\n",
    )
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
    assert_eq!(
        String::from_utf8_lossy(&export.stderr),
        warm_memory_progress()
    );
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
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        warm_memory_progress()
    );
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
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        warm_memory_progress()
    );
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
    let cold = env.run(&["run", "cold memory", "--json"]);
    let cold_elapsed = cold_started.elapsed();
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let cold_json: Value = serde_json::from_slice(&cold.stdout).unwrap();
    assert!(cold_json["text"].as_str().unwrap().contains("demo"));
    let cold_stderr = String::from_utf8_lossy(&cold.stderr);
    assert!(
        cold_stderr.starts_with(concat!(
            "Memory: waiting for project ownership…\n",
            "Memory: waiting for verified runtime cache…\n",
            "Memory: extracting embedded runtime…\n",
            "Memory: checking runtime version…\n",
            "Memory: preparing database…\n",
            "Memory: opening database…\n",
            "Memory: ready.\n",
        )),
        "{cold_stderr}"
    );
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
    assert_eq!(
        String::from_utf8_lossy(&warm.stderr),
        warm_memory_progress()
    );
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

    let reopened = MemoryStore::open(options.clone()).await.unwrap();
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

    let output = env.run(&["sessions"]);
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
