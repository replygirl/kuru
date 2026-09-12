//! Native authentication CLI regressions. No real account, credential store or
//! external executable is used; the process starts with an isolated environment.
use kuru_delivery::command::BlockingCommand as Command;
use serde_json::Value;
use std::{fs, path::PathBuf};

struct Environment {
    _temp: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
}

impl Environment {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        kuru_platform::fs::Directory::ensure_private(&root).unwrap();
        let project = root.join("project");
        for path in [
            &project,
            &root.join("home"),
            &root.join("config"),
            &root.join("empty-path"),
            &root.join("tmp"),
        ] {
            fs::create_dir(path).unwrap();
        }
        Self {
            _temp: temp,
            root,
            project,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        command
            .env_clear()
            .current_dir(&self.project)
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("APPDATA", self.root.join("config"))
            .env("LOCALAPPDATA", self.root.join("local"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("PATH", self.root.join("empty-path"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("TEMP", self.root.join("tmp"))
            .env("TMP", self.root.join("tmp"));
        for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
    }

    fn native_data(&self) -> PathBuf {
        if cfg!(windows) {
            self.root.join("local/kuru")
        } else {
            self.root.join("home/.local/share/kuru")
        }
    }
}

#[test]
fn native_status_works_without_codex_and_does_not_create_memory_or_credentials() {
    for kind in ["explicit", "xdg", "native"] {
        let env = Environment::new();
        let mut command = env.command();
        let expected = match kind {
            "explicit" => {
                let data = env.root.join("selected-data");
                command.arg("--data-dir").arg(&data);
                data
            }
            "xdg" => {
                command.env("XDG_DATA_HOME", env.root.join("xdg"));
                env.root.join("xdg/kuru")
            }
            _ => env.native_data(),
        };
        let output = command.arg("auth").output().unwrap();
        assert!(
            output.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(status["authenticated"], false);
        assert_eq!(status["api_key_available"], false);
        assert!(status["account_id"].is_null());
        assert!(!expected.exists(), "auth status created {kind} state");
        assert!(
            fs::read_dir(env.root.join("empty-path"))
                .unwrap()
                .next()
                .is_none()
        );
    }
}

#[test]
fn status_reports_only_api_key_presence_and_never_exposes_the_key() {
    let env = Environment::new();
    let output = env
        .command()
        .args(["--provider", "responses"])
        .env("OPENAI_API_KEY", "synthetic-key-do-not-print")
        .arg("auth")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["api_key_available"], true);
    assert_eq!(status["authenticated"], false);
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("synthetic-key-do-not-print"));
    }
    assert!(!env.native_data().exists());

    let inactive = env
        .command()
        .env("OPENAI_API_KEY", "inactive-key-do-not-print")
        .arg("auth")
        .output()
        .unwrap();
    assert!(inactive.status.success());
    let status: Value = serde_json::from_slice(&inactive.stdout).unwrap();
    assert_eq!(status["api_key_available"], false);
    assert!(
        String::from_utf8_lossy(&inactive.stderr).contains("was not checked"),
        "{}",
        String::from_utf8_lossy(&inactive.stderr)
    );
    for bytes in [&inactive.stdout, &inactive.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("inactive-key-do-not-print"));
    }
    assert!(!env.native_data().exists());
}

#[test]
fn authentication_rejects_workspace_state_and_removed_external_command_settings() {
    let env = Environment::new();
    let forbidden = env.project.join("state");
    let output = env
        .command()
        .arg("--data-dir")
        .arg(&forbidden)
        .arg("auth")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!forbidden.exists());
    let config = env.root.join("old-config.toml");
    fs::write(&config, "codex_command = 'untrusted-external-command'\n").unwrap();
    let output = env
        .command()
        .arg("--config")
        .arg(&config)
        .arg("auth")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("configuration validation error"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("untrusted-external-command"));
    assert!(!env.native_data().exists());
}

#[test]
fn login_help_exposes_browser_and_device_routes_without_starting_authentication() {
    let env = Environment::new();
    let output = env.command().args(["login", "--help"]).output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--device") && help.contains("--no-browser"));
    assert!(!env.native_data().exists());
    assert!(!env.root.join("home/.codex").exists());
}
