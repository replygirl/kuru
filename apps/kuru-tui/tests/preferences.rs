use kuru_delivery::command::BlockingCommand as Command;
use kuru_memory::MemoryStore;
use std::{
    path::{Path, PathBuf},
    process::Output,
    sync::Arc,
};

use kuru_connectors::DemoProvider;
use kuru_core::{Config, Mode, ProjectPreferences};
use kuru_runtime::Harness;

#[path = "support/memory.rs"]
mod memory;

struct Sandbox {
    root: memory::ServiceCleanup,
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
            root: memory::ServiceCleanup::new(root, &data),
            project,
            data,
        }
    }

    fn command(&self) -> Command {
        self.command_for(&self.project, &self.data, "demo")
    }

    fn command_for(&self, project: &Path, data: &Path, provider: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        #[cfg(windows)]
        command.fixture_allow_independent_service();
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

    async fn preferences(&self) -> ProjectPreferences {
        let mut options = kuru_memory::test_support::open_options(
            self.data.clone(),
            kuru_runtime::project_scope(&self.project).unwrap(),
        )
        .unwrap();
        options.read_only = true;
        let memory = MemoryStore::open(options).await.unwrap();
        let preferences = Harness::load_preferences(&memory, &self.project)
            .await
            .unwrap();
        memory.close().await.unwrap();
        preferences
    }

    async fn remember(&self) -> String {
        let options = kuru_memory::test_support::open_options(
            self.data.clone(),
            kuru_runtime::project_scope(&self.project).unwrap(),
        )
        .unwrap();
        let memory = MemoryStore::open(options).await.unwrap();
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
        .await
        .unwrap();
        harness.set_mode(Mode::Jungian).await.unwrap();
        harness
            .set_model("saved-model", Some("ultra".into()))
            .await
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

#[tokio::test]
async fn invocation_overrides_resume_and_other_projects_do_not_replace_saved_selections() {
    let sandbox = Sandbox::new();
    let saved_session = sandbox.remember().await;
    std::fs::create_dir(sandbox.project.join(".kuru")).unwrap();
    let shared = sandbox.project.join(".kuru/config.toml");
    let shared_contents = "mode='ifs'\nmodel='team-model'\neffort='low'\n";
    std::fs::write(&shared, shared_contents).unwrap();
    let configured = sandbox.config(&[]);
    assert_eq!(configured.mode, Mode::Ifs);
    assert_eq!(configured.model, "team-model");
    assert_eq!(configured.effort.as_deref(), Some("low"));
    let saved = sandbox.preferences().await;
    assert_eq!(saved.mode, Some(Mode::Jungian));
    assert_eq!(saved.providers["demo"].model, "saved-model");
    assert_eq!(saved.providers["demo"].effort.as_deref(), Some("ultra"));

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
    assert_eq!(sandbox.preferences().await, saved);

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
    sandbox.run(&["run", "saved preference invocation"]);
    sandbox.run(&[
        "--resume",
        &saved_session,
        "--mode",
        "ifs",
        "run",
        "resume own framework",
    ]);
    assert_eq!(sandbox.preferences().await, saved);
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
    assert!(
        sessions.iter().any(|session| session.mode == Mode::Jungian
            && session.label == "saved preference invocation")
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
        assert_eq!(alias_config, configured);
    }
}

#[tokio::test]
async fn configuration_inspection_ignores_tool_root_storage_but_activation_rejects_it() {
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.config(&[]).mode, Mode::Ifs);
    assert!(!sandbox.data.exists());
    let nested = sandbox.project.join("state");
    kuru_platform::fs::Directory::ensure_private(&nested).unwrap();
    std::fs::write(nested.join("memory.sqlite3"), b"existing legacy store").unwrap();
    let output = sandbox
        .command_for(&sandbox.project, &nested, "demo")
        .arg("config")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    toml::from_slice::<toml::Value>(&output.stdout).unwrap();
    let output = sandbox
        .command_for(&sandbox.project, &nested, "demo")
        .arg("sessions")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("outside the tool workspace"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read(nested.join("memory.sqlite3")).unwrap(),
        b"existing legacy store"
    );
}

#[tokio::test]
async fn an_explicit_framework_override_is_validated_against_its_own_part_budget() {
    let sandbox = Sandbox::new();
    sandbox.remember().await;
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
    assert_eq!(sandbox.preferences().await.mode, Some(Mode::Jungian));
}
