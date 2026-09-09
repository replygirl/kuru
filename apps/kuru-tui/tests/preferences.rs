use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use kuru_connectors::DemoProvider;
use kuru_core::{Config, MemoryStore, Mode};
use kuru_runtime::Harness;

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
        Self {
            root,
            project,
            data,
        }
    }

    fn command(&self) -> Command {
        self.command_for(&self.project, &self.data, "demo")
    }

    fn command_for(&self, project: &Path, data: &Path, provider: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        command
            .arg("-C")
            .arg(project)
            .arg("--data-dir")
            .arg(data)
            .args(["--provider", provider, "--no-dream"])
            .env("XDG_CONFIG_HOME", self.root.path().join("config"));
        command
    }

    fn run(&self, args: &[&str]) -> String {
        let output = self.command().args(args).output().unwrap();
        success(output)
    }

    fn config(&self, args: &[&str]) -> Config {
        toml::from_str(&success(
            self.command().args(args).arg("config").output().unwrap(),
        ))
        .unwrap()
    }

    fn remember(&self) -> String {
        let memory = MemoryStore::open(&self.data.join("memory.sqlite3")).unwrap();
        let mut harness = Harness::new(
            Config {
                provider: "demo".into(),
                model: "demo".into(),
                ..Config::default()
            },
            &self.project,
            memory,
            Arc::new(DemoProvider),
            None,
        )
        .unwrap();
        harness.set_mode(Mode::Jungian).unwrap();
        harness
            .set_model("saved-model", Some("ultra".into()))
            .unwrap();
        harness.session.id.clone()
    }
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn terminal_selections_survive_restarts_picker_changes_and_failed_database_writes() {
    let sandbox = Sandbox::new();
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/preferences_smoke.py"
        ))
        .arg(env!("CARGO_BIN_EXE_kuru"))
        .arg(&sandbox.project)
        .arg(&sandbox.data)
        .env("XDG_CONFIG_HOME", sandbox.root.path().join("config"))
        .output()
        .unwrap();
    success(output);
    let sessions: Vec<kuru_runtime::Session> =
        serde_json::from_str(&sandbox.run(&["sessions"])).unwrap();
    assert_eq!(sessions.len(), 4);
    assert!(sessions.iter().all(|session| session.turns == 0));
    assert_eq!(sessions[0].mode, Mode::Jungian);
    assert_eq!(sessions[1].mode, Mode::Freudian);
    assert_eq!(sessions[2].mode, Mode::Freudian);
    assert_eq!(sessions[3].mode, Mode::Freudian);
}

#[tokio::test]
async fn invocation_overrides_resume_and_other_projects_do_not_replace_saved_selections() {
    let sandbox = Sandbox::new();
    let saved_session = sandbox.remember();
    std::fs::create_dir(sandbox.project.join(".kuru")).unwrap();
    let shared = sandbox.project.join(".kuru/config.toml");
    let shared_contents = "mode='ifs'\nmodel='team-model'\neffort='low'\n";
    std::fs::write(&shared, shared_contents).unwrap();
    let saved = sandbox.config(&[]);
    assert_eq!(saved.mode, Mode::Jungian);
    assert_eq!(saved.model, "saved-model");
    assert_eq!(saved.effort.as_deref(), Some("ultra"));

    let temporary = sandbox.config(&[
        "--mode",
        "freudian",
        "--model",
        "temporary",
        "--effort",
        "high",
    ]);
    assert_eq!(temporary.mode, Mode::Freudian);
    assert_eq!(temporary.model, "temporary");
    assert_eq!(temporary.effort.as_deref(), Some("high"));
    let changed_model = sandbox.config(&["--model", "temporary"]);
    assert_eq!(changed_model.effort.as_deref(), Some("low"));
    assert_eq!(sandbox.config(&[]), saved);

    let local = sandbox.root.path().join("invocation.toml");
    std::fs::write(
        &local,
        "mode='polyvagal'\nmodel='local-model'\neffort='medium'\n",
    )
    .unwrap();
    let local_config = sandbox.config(&["--config", local.to_str().unwrap()]);
    assert_eq!(local_config.mode, Mode::Polyvagal);
    assert_eq!(local_config.model, "local-model");
    assert_eq!(local_config.effort.as_deref(), Some("medium"));
    sandbox.run(&["--mode", "freudian", "run", "temporary invocation"]);
    sandbox.run(&[
        "--config",
        local.to_str().unwrap(),
        "run",
        "local invocation",
    ]);
    sandbox.run(&[
        "--resume",
        &saved_session,
        "--mode",
        "ifs",
        "run",
        "resume own framework",
    ]);
    assert_eq!(sandbox.config(&[]), saved);
    let sessions: Vec<kuru_runtime::Session> =
        serde_json::from_str(&sandbox.run(&["sessions"])).unwrap();
    assert!(
        sessions.iter().any(
            |session| session.mode == Mode::Freudian && session.label == "temporary invocation"
        )
    );
    assert!(
        sessions
            .iter()
            .any(|session| session.mode == Mode::Polyvagal && session.label == "local invocation")
    );
    assert!(sessions.iter().any(|session| session.id == saved_session
        && session.mode == Mode::Jungian
        && session.turns == 1));
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), shared_contents);

    let other = sandbox.root.path().join("other-project");
    std::fs::create_dir(&other).unwrap();
    let other_config: Config = toml::from_str(&success(
        sandbox
            .command_for(&other, &sandbox.data, "demo")
            .arg("config")
            .output()
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(other_config.mode, Mode::Ifs);
    assert_eq!(other_config.model, "auto");
    assert!(other_config.effort.is_none());
    let api_config: Config = toml::from_str(&success(
        sandbox
            .command_for(&sandbox.project, &sandbox.data, "responses")
            .arg("config")
            .output()
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(api_config.model, "team-model");
    assert_eq!(api_config.effort.as_deref(), Some("low"));

    #[cfg(unix)]
    {
        let alias = sandbox.root.path().join("project-alias");
        std::os::unix::fs::symlink(&sandbox.project, &alias).unwrap();
        let alias_config: Config = toml::from_str(&success(
            sandbox
                .command_for(&alias, &sandbox.data, "demo")
                .arg("config")
                .output()
                .unwrap(),
        ))
        .unwrap();
        assert_eq!(alias_config, saved);
    }
}

#[test]
fn configuration_inspection_does_not_create_a_store_but_rejects_existing_tool_root_storage() {
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.config(&[]).mode, Mode::Ifs);
    assert!(!sandbox.data.exists());
    let nested = sandbox.project.join("state");
    MemoryStore::open(&nested.join("memory.sqlite3")).unwrap();
    let output = sandbox
        .command_for(&sandbox.project, &nested, "demo")
        .arg("config")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the tool workspace"));
}

#[tokio::test]
async fn an_explicit_framework_override_is_validated_against_its_own_part_budget() {
    let sandbox = Sandbox::new();
    sandbox.remember();
    let local = sandbox.root.path().join("small-pool.toml");
    std::fs::write(&local, "max_parts=3\n").unwrap();
    let config = sandbox.config(&["--config", local.to_str().unwrap(), "--mode", "freudian"]);
    assert_eq!(config.mode, Mode::Freudian);
    assert_eq!(config.max_parts, 3);
    sandbox.run(&[
        "--config",
        local.to_str().unwrap(),
        "--mode",
        "freudian",
        "run",
        "Use the smaller pool",
    ]);
    assert_eq!(sandbox.config(&[]).mode, Mode::Jungian);
}
