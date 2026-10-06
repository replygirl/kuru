#![cfg(unix)]
// Native Windows loaded-image CLI acceptance lives in embedded_runtime.rs and
// windows_cli.rs; these independent script payloads exercise Unix execution.
use std::{path::Path, process::Command};

use sha2::{Digest, Sha256};

/// New ownership fixtures use the platform spawn lock and retain the child
/// through pipe draining, group cleanup and reap on every outcome.
async fn owned_update_output(command: Command) -> anyhow::Result<std::process::Output> {
    use anyhow::{Context, ensure};
    use kuru_platform::unix::{OwnedProcessGroup, Reap, RootState, StdioPlan, StdioSlot};
    use std::{
        io::Read,
        time::{Duration, Instant},
    };

    let mut child = OwnedProcessGroup::spawn(
        command,
        StdioPlan::new(StdioSlot::Null, StdioSlot::Pipe, StdioSlot::Pipe),
    )?;
    let capture = |mut pipe: Box<dyn Read + Send>| {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut bytes = Vec::new();
                let mut chunk = [0; 8192];
                let mut overflow = false;
                loop {
                    let count = pipe.read(&mut chunk)?;
                    if count == 0 {
                        break;
                    }
                    let available = (1024 * 1024usize).saturating_sub(bytes.len());
                    bytes.extend_from_slice(&chunk[..count.min(available)]);
                    overflow |= count > available;
                }
                ensure!(!overflow, "update fixture output exceeded its bound");
                Ok::<_, anyhow::Error>(bytes)
            })();
            let _ = sender.send(result);
        });
        receiver
    };
    let stdout = capture(Box::new(child.take_stdout()?));
    let stderr = capture(Box::new(child.take_stderr()?));
    let deadline = Instant::now() + Duration::from_secs(60);
    let outcome = async {
        loop {
            match child.root_state() {
                RootState::Exited => return Ok::<_, anyhow::Error>(()),
                RootState::Running | RootState::Interrupted => {}
                state => anyhow::bail!("update fixture root state: {state:?}"),
            }
            ensure!(Instant::now() < deadline, "update fixture did not settle");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    .await;
    child.terminate_before_reap();
    child
        .wait_pre_reap(
            Duration::from_millis(10),
            Instant::now() + Duration::from_secs(10),
        )
        .await;
    let status = child.reap_if_exited();
    let stdout = stdout
        .recv_timeout(Duration::from_secs(10))
        .context("update stdout did not drain")?;
    let stderr = stderr
        .recv_timeout(Duration::from_secs(10))
        .context("update stderr did not drain")?;
    outcome?;
    let Reap::Reaped(status) = status else {
        anyhow::bail!("update fixture was not reaped: {status:?}")
    };
    Ok(std::process::Output {
        status,
        stdout: stdout.context("update stdout")?,
        stderr: stderr.context("update stderr")?,
    })
}

fn image_facts(path: &Path) -> anyhow::Result<(u64, u64, u64, Vec<u8>)> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let mut digest = Sha256::new();
    let mut chunk = [0; 8192];
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        digest.update(&chunk[..count]);
    }
    Ok((
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        digest.finalize().to_vec(),
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manager_owned_update_refuses_release_source_and_alias_without_effects()
-> anyhow::Result<()> {
    kuru_memory::test_support::closing(async {
        use anyhow::ensure;
        let root = tempfile::tempdir()?;
        let home = root.path().join("home");
        std::fs::create_dir(&home)?;
        let source = root.path().join("checkout");
        std::fs::create_dir_all(source.join("scripts"))?;
        let sentinel = root.path().join("build-attempted");
        std::fs::write(
            source.join("scripts/install.sh"),
            b"#!/bin/sh\nprintf attempted > \"$KURU_OWNERSHIP_BUILD_SENTINEL\"\n",
        )?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let release = format!("https://{}", listener.local_addr()?);
        for (manager, base, suffix, hint) in [
            (
                "mise",
                "mise data",
                "installs/github-replygirl-kuru/v/bin",
                "mise upgrade github:replygirl/kuru",
            ),
            ("Homebrew", "cellar", "kuru/v/bin", "brew upgrade kuru"),
        ] {
            let manager_root = root.path().join(base);
            let bin = manager_root.join(suffix);
            std::fs::create_dir_all(&bin)?;
            let executable = bin.join("kuru");
            let mut copy = Command::new(std::env::current_exe()?);
            copy.args([
                "--exact",
                "copy_executable_process",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("KURU_UPDATE_COPY_DESTINATION", &executable);
            let output = owned_update_output(copy).await?;
            ensure!(
                output.status.success(),
                "copy failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let before = image_facts(&executable)?;
            let alias = root.path().join(format!("{manager}-alias"));
            std::os::unix::fs::symlink(&executable, &alias)?;
            for (invoked, source_mode) in
                [(&executable, false), (&executable, true), (&alias, false)]
            {
                let mut command = Command::new(invoked);
                command
                    .env_clear()
                    .env("HOME", &home)
                    .env("PATH", "/usr/bin:/bin")
                    .env(
                        "MISE_DATA_DIR",
                        if manager == "mise" {
                            &manager_root
                        } else {
                            &home
                        },
                    )
                    .env(
                        "HOMEBREW_CELLAR",
                        if manager == "Homebrew" {
                            &manager_root
                        } else {
                            &home
                        },
                    )
                    .env("KURU_OWNERSHIP_BUILD_SENTINEL", &sentinel)
                    .current_dir(&home)
                    .arg("update");
                if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                    command.env("LLVM_PROFILE_FILE", profile);
                }
                if source_mode {
                    command.arg("--source").arg(&source);
                } else {
                    command.args(["--version", "0.2.0", "--release-base", &release]);
                }
                let output = owned_update_output(command).await?;
                ensure!(!output.status.success());
                ensure!(output.stdout.is_empty());
                let error = String::from_utf8_lossy(&output.stderr);
                ensure!(
                    error.contains(&format!("managed by {manager}")) && error.contains(hint),
                    "{error}"
                );
                ensure!(image_facts(&executable)? == before);
                ensure!(!sentinel.exists());
                ensure!(std::fs::read_dir(&bin)?.count() == 1);
                ensure!(std::fs::read_dir(&home)?.count() == 0);
                match listener.accept() {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    other => {
                        anyhow::bail!("manager refusal attempted a network connection: {other:?}")
                    }
                }
            }
        }
        Ok(())
    })
    .await
}

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
    // This is an explicitly unmarked legacy core. Its full three-member
    // inventory must pass structural checks before the checksum control runs.
    for (name, bytes, mode) in [
        ("kuru", &payload[..], 0o755),
        ("LICENSE", &b"fixture license\n"[..], 0o644),
        ("README.md", &b"unmarked legacy fixture\n"[..], 0o644),
    ] {
        let mut member = tar::Header::new_ustar();
        member.set_size(bytes.len() as u64);
        member.set_mode(mode);
        member.set_cksum();
        archive.append_data(&mut member, name, bytes).unwrap();
    }
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
    std::fs::write(
        release.join("SHA256SUMS"),
        format!("{}  {name}\n", "0".repeat(64)),
    )
    .unwrap();
    let before = std::fs::read(&executable).unwrap();
    let output = Command::new(&executable)
        .args(["update", "--version", "0.2.0", "--release-base"])
        .arg(&release)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("checksum mismatch"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
