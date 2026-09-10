#![cfg(unix)]
// Native Windows loaded-image CLI acceptance lives in embedded_runtime.rs and
// windows_cli.rs; these independent script payloads exercise Unix execution.
use std::{path::Path, process::Command};

use sha2::{Digest, Sha256};

fn copy_executable(destination: &Path) {
    // These tests spawn concurrently. Writing an executable in this process
    // lets another thread's child inherit its writable descriptor before exec,
    // even with CLOEXEC, causing Linux ETXTBSY after our copy has returned.
    // Keep all executable writes in a separate process and wait for its exit:
    // the test parent can never pass those descriptors to another child.
    // https://github.com/rust-lang/rust/issues/114554
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "copy_executable_process",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KURU_UPDATE_COPY_DESTINATION", destination)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "copy fixture failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// A test-only subprocess entry; both updater tests exercise it explicitly.
// The copied executable has its own inode, so an updater regression cannot
// modify Cargo's built executable through a hardlink.
#[test]
#[ignore = "subprocess entry exercised by both updater integration tests"]
fn copy_executable_process() {
    let destination = std::env::var_os("KURU_UPDATE_COPY_DESTINATION").unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_kuru"), destination).unwrap();
}

#[test]
fn updater_replaces_the_running_binary_only_after_checksum_validation() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    let release = root.path().join("release");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&release).unwrap();
    let executable = bin.join("kuru");
    copy_executable(&executable);
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        other => panic!("unsupported fixture target: {other:?}"),
    };
    let name = format!("kuru-0.2.0-{target}.tar.gz");
    // Construct the fixture independently of the production packaging code.
    let file = std::fs::File::create(release.join(&name)).unwrap();
    let compressed = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut archive = tar::Builder::new(compressed);
    let payload = b"#!/bin/sh\necho updated-kuru\n";
    let mut member = tar::Header::new_ustar();
    member.set_size(payload.len() as u64);
    member.set_mode(0o755);
    member.set_cksum();
    archive
        .append_data(&mut member, "kuru", &payload[..])
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let digest: String = Sha256::digest(std::fs::read(release.join(&name)).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    std::fs::write(release.join("SHA256SUMS"), format!("{digest}  {name}\n")).unwrap();
    let output = Command::new(&executable)
        .args(["update", "--version", "0.2.0", "--release-base"])
        .arg(&release)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        Command::new(&executable).output().unwrap().stdout,
        b"updated-kuru\n"
    );
    copy_executable(&executable);
    std::fs::write(release.join("SHA256SUMS"), "bad checksum").unwrap();
    let before = std::fs::read(&executable).unwrap();
    let output = Command::new(&executable)
        .args(["update", "--version", "0.2.0", "--release-base"])
        .arg(&release)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&executable).unwrap(), before);
    assert!(
        Command::new(&executable)
            .arg("--version")
            .output()
            .unwrap()
            .stdout
            .starts_with(concat!("kuru ", env!("CARGO_PKG_VERSION")).as_bytes())
    );
}

#[test]
fn source_update_passes_checkout_and_destination_without_shell_interpolation() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin with spaces");
    let source = root.path().join("checkout with spaces");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    let executable = bin.join("kuru");
    copy_executable(&executable);
    std::fs::write(source.join("scripts/install.sh"),"#!/bin/sh\nset -eu\ntest \"$1\" = --source\nprintf '%s' \"$KURU_INSTALL_DIR\" > \"$KURU_INSTALL_DIR/destination-check\"\n").unwrap();
    let output = Command::new(&executable)
        .args(["update", "--source"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(bin.join("destination-check")).unwrap(),
        bin.to_string_lossy()
    );
    assert!(
        !Command::new(&executable)
            .args(["update", "--version", "0.2.0", "--source"])
            .arg(&source)
            .output()
            .unwrap()
            .status
            .success()
    );
}
