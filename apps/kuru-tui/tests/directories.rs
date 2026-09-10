use kuru_delivery::command::BlockingCommand as Command;
use std::path::Path;

fn isolated(root: &Path, project: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru"));
    command
        .env_clear()
        .current_dir(root)
        .arg("-C")
        .arg(project)
        .args(["--provider", "demo", "--no-dream"])
        .env("PATH", "")
        .env("TMPDIR", root)
        .env("TMP", root)
        .env("TEMP", root);
    for key in ["SystemRoot", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

fn text(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn explicit_and_xdg_directories_work_without_an_ambient_home() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project 日本語");
    std::fs::create_dir(&project).unwrap();
    let config_home = root.path().join("config override");
    std::fs::create_dir_all(config_home.join("kuru")).unwrap();
    std::fs::write(config_home.join("kuru/config.toml"), "mode = 'freudian'\n").unwrap();
    let explicit = root.path().join("explicit.toml");
    std::fs::write(&explicit, "mode = 'jungian'\n").unwrap();
    let mut command = isolated(root.path(), &project);
    command
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_DATA_HOME", root.path().join("data override"));
    assert!(text(command.arg("config")).contains("mode = \"freudian\""));
    let mut command = isolated(root.path(), &project);
    command
        .arg("--config")
        .arg(&explicit)
        .arg("--data-dir")
        .arg("relative state")
        .arg("config");
    assert!(text(&mut command).contains("mode = \"jungian\""));
    assert!(
        !root.path().join("relative state").exists(),
        "configuration inspection must not provision memory"
    );
    let failure = isolated(root.path(), &project)
        .arg("config")
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("--data-dir"));
}

#[cfg(windows)]
#[test]
fn native_roaming_and_local_defaults_and_explicit_precedence_survive_real_reopen() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("workspace 日本語");
    std::fs::create_dir(&project).unwrap();
    let roaming = root.path().join("Roaming space");
    let local = root.path().join("Local space");
    std::fs::create_dir_all(roaming.join("kuru")).unwrap();
    let config = format!(
        "mode = 'freudian'\n[memory]\noffline = true\ncache_dir = {}\n",
        toml::Value::String(
            kuru_memory::test_support::cache_dir()
                .to_str()
                .unwrap()
                .into()
        )
    );
    std::fs::write(roaming.join("kuru/config.toml"), &config).unwrap();
    let native = || {
        let mut command = isolated(root.path(), &project);
        // Windows environment keys are case-insensitive.
        command.env("appdata", &roaming).env("localappdata", &local);
        command
    };
    assert!(text(native().arg("config")).contains("mode = \"freudian\""));
    let session: serde_json::Value = serde_json::from_str(&text(native().args([
        "run",
        "Keep this native session",
        "--json",
    ])))
    .unwrap();
    assert!(local.join("kuru/memory").is_dir());
    let sessions: serde_json::Value =
        serde_json::from_str(&text(native().arg("sessions"))).unwrap();
    assert!(
        sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|saved| saved["id"] == session["session"])
    );

    let xdg = root.path().join("XDG choice");
    std::fs::create_dir_all(xdg.join("kuru")).unwrap();
    std::fs::write(
        xdg.join("kuru/config.toml"),
        config.replace("freudian", "jungian"),
    )
    .unwrap();
    assert!(
        text(native().env("XDG_CONFIG_HOME", &xdg).arg("config")).contains("mode = \"jungian\"")
    );
    let explicit = root.path().join("explicit.toml");
    std::fs::write(&explicit, "mode = 'polyvagal'\n").unwrap();
    assert!(
        text(
            native()
                .env("XDG_CONFIG_HOME", &xdg)
                .arg("--config")
                .arg(&explicit)
                .arg("config")
        )
        .contains("mode = \"polyvagal\"")
    );
    let override_data = root.path().join("explicit native data");
    text(
        native()
            .env("XDG_DATA_HOME", root.path().join("unused data"))
            .env("KURU_DATA_DIR", &override_data)
            .args(["run", "Use configured data"]),
    );
    assert!(override_data.join("memory").is_dir());
    assert!(!root.path().join("unused data").exists());

    let profile = root.path().join("Profile fallback");
    std::fs::create_dir_all(profile.join("AppData/Roaming/kuru")).unwrap();
    std::fs::write(profile.join("AppData/Roaming/kuru/config.toml"), config).unwrap();
    assert!(
        text(
            isolated(root.path(), &project)
                .env("USERPROFILE", &profile)
                .arg("config")
        )
        .contains("mode = \"freudian\"")
    );
}
