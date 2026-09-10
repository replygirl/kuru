use std::{fs::OpenOptions, process::Command};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[path = "support/memory.rs"]
mod memory;

fn arguments(project: &std::path::Path, data: &std::path::Path) -> Vec<String> {
    let configuration = memory::configuration(data.parent().unwrap()).unwrap();
    vec![
        "--config".into(),
        configuration.join("kuru/config.toml").display().to_string(),
        "-C".into(),
        project.display().to_string(),
        "--data-dir".into(),
        data.display().to_string(),
        "--provider".into(),
        "demo".into(),
        "--no-dream".into(),
    ]
}

#[test]
fn writer_lease_rejects_a_second_process_and_releases_on_close() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    let data = root.path().join("data");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(data.join("locks")).unwrap();
    let project = project.canonicalize().unwrap();
    let digest = Sha256::digest(project.as_os_str().as_encoded_bytes());
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(data.join("locks").join(format!("{hash}.lock")))
        .unwrap();
    lease.try_lock().unwrap();
    let mut args = arguments(&project, &data);
    args.extend(["run".into(), "hello".into(), "--json".into()]);
    let blocked = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(&args)
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("active Kuru writer"));
    // Even a read command needs the writer lease before importing a legacy file.
    // An invalid marker proves the lease error precedes any attempt to parse it.
    let legacy = data.join("memory.sqlite3");
    std::fs::write(&legacy, b"legacy bytes must remain untouched").unwrap();
    let blocked_migration = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(arguments(&project, &data))
        .arg("sessions")
        .output()
        .unwrap();
    assert!(!blocked_migration.status.success());
    assert!(String::from_utf8_lossy(&blocked_migration.stderr).contains("active Kuru writer"));
    assert_eq!(
        std::fs::read(&legacy).unwrap(),
        b"legacy bytes must remain untouched"
    );
    assert_eq!(std::fs::read_dir(&data).unwrap().count(), 2);
    std::fs::remove_file(legacy).unwrap();
    drop(lease);
    let resumed = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(&args)
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert!(!result["text"].as_str().unwrap().is_empty());
}

#[test]
fn listing_sessions_does_not_create_new_sessions() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    let data = root.path().join("data");
    std::fs::create_dir_all(&project).unwrap();
    let args = arguments(&project, &data);
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_kuru"))
            .args(&args)
            .arg("sessions")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!([])
        );
    }
    let run = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(&args)
        .args(["run", "a saved session"])
        .output()
        .unwrap();
    assert!(run.status.success());
    let first = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(&args)
        .arg("sessions")
        .output()
        .unwrap();
    let second = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(&args)
        .arg("sessions")
        .output()
        .unwrap();
    assert!(first.status.success() && second.status.success());
    let sessions: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn config_does_not_print_mcp_environment_credentials() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(project.join(".kuru")).unwrap();
    std::fs::write(project.join(".kuru/config.toml"),
        "[mcp.fixture]\ncommand = 'fixture-server'\n[mcp.fixture.env]\nSYNTHETIC_TEST_TOKEN = 'do-not-print-this-fixture'\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(arguments(&project, &root.path().join("data")))
        .arg("config")
        .output()
        .unwrap();
    assert!(output.status.success());
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.contains("SYNTHETIC_TEST_TOKEN") && printed.contains("[redacted]"));
    assert!(!printed.contains("do-not-print-this-fixture"));
}

#[cfg(unix)]
#[test]
fn symlink_lock_directory_cannot_redirect_project_locks() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    let data = root.path().join("data");
    let outside = root.path().join("outside");
    for path in [&project, &data, &outside] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::os::unix::fs::symlink(&outside, data.join("locks")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kuru"))
        .args(arguments(&project, &data))
        .args(["run", "hello"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("regular directory"));
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}
