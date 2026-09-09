use std::process::Command;

use sha2::{Digest, Sha256};

#[test]
fn updater_replaces_the_running_binary_only_after_checksum_validation() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    let release = root.path().join("release");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&release).unwrap();
    let executable = bin.join("kuru");
    std::fs::copy(env!("CARGO_BIN_EXE_kuru"), &executable).unwrap();
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
    std::fs::copy(env!("CARGO_BIN_EXE_kuru"), &executable).unwrap();
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
            .starts_with(b"kuru 0.1.0")
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
    std::fs::copy(env!("CARGO_BIN_EXE_kuru"), &executable).unwrap();
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
