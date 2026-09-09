use std::process::Command;

#[test]
fn updater_replaces_the_running_binary_only_after_checksum_validation() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    let release = root.path().join("release");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(&release).unwrap();
    let executable = bin.join("kuru");
    std::fs::copy(env!("CARGO_BIN_EXE_kuru"), &executable).unwrap();
    let make=Command::new("python3").arg("-c").arg(r#"
import hashlib,io,pathlib,platform,sys,tarfile
directory=pathlib.Path(sys.argv[1])
targets={('Darwin','arm64'):'aarch64-apple-darwin',('Darwin','x86_64'):'x86_64-apple-darwin',('Linux','aarch64'):'aarch64-unknown-linux-gnu',('Linux','x86_64'):'x86_64-unknown-linux-gnu'}
name='kuru-0.2.0-'+targets[(platform.system(),platform.machine())]+'.tar.gz'
with tarfile.open(directory/name,'w:gz') as archive:
    payload=b'#!/bin/sh\necho updated-kuru\n'
    member=tarfile.TarInfo('kuru');member.size=len(payload);member.mode=0o755
    archive.addfile(member,io.BytesIO(payload))
(directory/'SHA256SUMS').write_text(hashlib.sha256((directory/name).read_bytes()).hexdigest()+'  '+name+'\n')
"#).arg(&release).output().unwrap();
    assert!(make.status.success());
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
    let before = std::fs::metadata(&executable).unwrap().len();
    let output = Command::new(&executable)
        .args(["update", "--version", "0.2.0", "--release-base"])
        .arg(&release)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::metadata(&executable).unwrap().len(), before);
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
