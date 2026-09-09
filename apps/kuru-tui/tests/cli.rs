use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Command, Output},
};

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
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
        command
            .arg("-C")
            .arg(&self.project)
            .arg("--data-dir")
            .arg(&self.data)
            .args(["--provider", "demo", "--no-dream"])
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
    assert!(env.success(&["--version"]).starts_with("kuru 0."));
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
        !env.run(&[
            "tool",
            "shell",
            "--args",
            r#"{"command":"printf shell-ok"}"#
        ])
        .status
        .success()
    );
    assert!(
        env.success(&[
            "--allow-shell",
            "tool",
            "shell",
            "--args",
            r#"{"command":"printf shell-ok"}"#
        ])
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
