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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_update_uses_native_mise_arguments_and_preserves_read_only_output()
-> anyhow::Result<()> {
    kuru_memory::test_support::closing(async {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = tempfile::tempdir()?;
        let bin = root.path().join("bin with spaces");
        let source = root.path().join("checkout with spaces; literal $()");
        let tools = root.path().join("tools");
        std::fs::create_dir(&bin)?;
        std::fs::create_dir(&source)?;
        std::fs::create_dir(&tools)?;
        let executable = bin.join("kuru");
        copy_executable(&executable);
        let before = image_facts(&executable)?;
        let arguments = root.path().join("build-arguments");
        let marker = root.path().join("must-not-execute");
        let host = kuru_delivery::archive::host_target()?;
        let target = root.path().join("build output");
        let artifact = target.join(host).join("release/kuru");
        std::fs::create_dir_all(artifact.parent().unwrap())?;
        let candidate = br#"#!/bin/sh
printf 'executed' > "$KURU_EXECUTION_MARKER"
"#;
        std::fs::write(&artifact, candidate)?;
        std::fs::set_permissions(&artifact, std::fs::Permissions::from_mode(0o555))?;
        std::fs::hard_link(&artifact, root.path().join("other-cargo-output"))?;
        let metadata = std::fs::metadata(&artifact)?;
        std::fs::write(
            tools.join("mise"),
            br#"#!/bin/sh
printf '%s\0' "$@" >> "$KURU_ARGUMENTS"
"#,
        )?;
        std::fs::set_permissions(tools.join("mise"), std::fs::Permissions::from_mode(0o755))?;
        let mut invalid = Command::new(&executable);
        invalid
            .args(["update", "--version", "0.2.0", "--source"])
            .arg(&source);
        assert!(!owned_update_output(invalid).await?.status.success());
        assert_eq!(image_facts(&executable)?, before);
        let mut command = Command::new(&executable);
        command
            .args(["update", "--source"])
            .arg(&source)
            .env("PATH", &tools)
            .env("CARGO_TARGET_DIR", &target)
            .env_remove("CARGO_BUILD_TARGET")
            .env("KURU_ARGUMENTS", &arguments)
            .env("KURU_EXECUTION_MARKER", &marker);
        let output = owned_update_output(command).await?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recorded = std::fs::read(&arguments)?;
        let argv: Vec<&[u8]> = recorded
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .collect();
        let exact_source = source.canonicalize()?;
        assert_eq!(
            argv,
            vec![
                b"-C".as_slice(),
                exact_source.as_os_str().as_encoded_bytes(),
                b"install",
                b"rust",
                b"-C",
                exact_source.as_os_str().as_encoded_bytes(),
                b"run",
                b"//apps/kuru-tui:build:release",
                b"--",
                b"--target",
                host.as_bytes(),
            ]
        );
        assert_eq!(std::fs::read(&executable)?, candidate);
        let after = std::fs::metadata(&artifact)?;
        assert_eq!(
            (
                metadata.dev(),
                metadata.ino(),
                metadata.nlink(),
                metadata.mode()
            ),
            (after.dev(), after.ino(), after.nlink(), after.mode())
        );
        assert_eq!(std::fs::read(&artifact)?, candidate);
        assert!(!marker.exists());
        let mut entries = std::fs::read_dir(bin.join(".kuru-update"))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        assert_eq!(entries, vec![std::ffi::OsString::from("install.lock")]);
        Ok(())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_update_recovery_is_config_free_and_keeps_stdout_unchanged() -> anyhow::Result<()>
{
    kuru_memory::test_support::closing(async {
        use kuru_platform::fs::{Directory, NameRetention, Privacy, regular_file_info};
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        let parent = root.path().join("installation");
        let home = root.path().join("empty home");
        std::fs::create_dir(&parent)?;
        std::fs::create_dir(&home)?;
        let executable = parent.join("kuru");
        copy_executable(&executable);
        let configuration = || {
            let mut command = Command::new(&executable);
            command.arg("config").current_dir(root.path()).env_clear()
                .env("PATH", "/usr/bin:/bin").env("HOME", &home)
                .env("XDG_CONFIG_HOME", home.join("config"))
                .env("XDG_DATA_HOME", home.join("data"));
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            command
        };
        let baseline = owned_update_output(configuration()).await?;
        assert!(baseline.status.success(), "{}", String::from_utf8_lossy(&baseline.stderr));
        assert!(!parent.join(".kuru-update").exists());
        let directory = Directory::open(&parent, Privacy::Inherited, NameRetention::Movable)?;
        let original = directory.read(std::ffi::OsStr::new("kuru"))?;
        let original_info = regular_file_info(&original)?;
        let original_hash = image_facts(&executable)?.3.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let operation = uuid::Uuid::new_v4();
        let state = parent.join(".kuru-update");
        std::fs::create_dir(&state)?;
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
        let receipt = serde_json::json!({
            "schema_version":1,"platform":"unix","operation":operation.to_string(),
            "parent":directory.path(),"parent_identity":directory.identity().to_bytes(),
            "installed":"kuru","candidate":format!("candidate-{operation}"),"backup":format!("backup-{operation}"),
            "original":{"identity":original_info.identity.to_bytes(),"sha256":original_hash,"bytes":original_info.len},
            "original_meta":{"version":null,"target":kuru_delivery::archive::host_target()?},
            "replacement":null,"replacement_meta":{"version":null,"target":kuru_delivery::archive::host_target()?},
            "backup_image":null,"restored":null,"support_hashes":null,"phase":"preparing"
        });
        std::fs::write(state.join("receipt.json"), serde_json::to_vec(&receipt)?)?;
        std::fs::set_permissions(state.join("receipt.json"), std::fs::Permissions::from_mode(0o600))?;
        std::fs::write(state.join(format!("candidate-{operation}")), b"partial")?;
        std::fs::write(state.join(format!("backup-{operation}")), b"partial")?;
        std::fs::set_permissions(state.join(format!("backup-{operation}")), std::fs::Permissions::from_mode(0o600))?;
        let recovered = owned_update_output(configuration()).await?;
        assert!(recovered.status.success(), "{}", String::from_utf8_lossy(&recovered.stderr));
        assert_eq!(recovered.stdout, baseline.stdout);
        assert!(String::from_utf8_lossy(&recovered.stderr).contains("settled without replacement"));
        assert_eq!(regular_file_info(&original)?.identity, original_info.identity);
        assert_eq!(image_facts(&executable)?.3.iter().map(|byte| format!("{byte:02x}")).collect::<String>(), original_hash);
        assert_eq!(std::fs::read_dir(&state)?.count(), 1);
        assert!(state.join("install.lock").is_file());
        assert!(!home.join("data").exists());
        // Corrupt retained evidence reports a fixed stderr refusal, preserving
        // the ordinary pure-inspection output and the exact unknown bytes.
        std::fs::write(state.join("receipt.next"), b"{unknown\x1b[31m")?;
        std::fs::set_permissions(state.join("receipt.next"), std::fs::Permissions::from_mode(0o600))?;
        let refused = owned_update_output(configuration()).await?;
        assert!(refused.status.success());
        assert_eq!(refused.stdout, baseline.stdout);
        assert!(!refused.stderr.contains(&0x1b));
        assert_eq!(std::fs::read(state.join("receipt.next"))?, b"{unknown\x1b[31m");
        Ok(())
    }).await
}

/// Retained root and concurrently drained bounded pipes for changed updater
/// interruption fixtures. Only the fixed launcher writes the root PID.
struct InterruptedUpdate {
    owner: kuru_platform::unix::OwnedProcessGroup,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    stderr: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    #[cfg(all(target_os = "linux", feature = "test-support"))]
    ready: Option<tokio::sync::oneshot::Receiver<()>>,
}
impl InterruptedUpdate {
    fn start(command: Command, hold_input: bool) -> anyhow::Result<Self> {
        use kuru_platform::unix::{OwnedProcessGroup, StdioPlan, StdioSlot};
        use std::io::Read;
        let mut owner = OwnedProcessGroup::spawn(
            command,
            StdioPlan::new(
                if hold_input {
                    StdioSlot::Pipe
                } else {
                    StdioSlot::Null
                },
                StdioSlot::Pipe,
                StdioSlot::Pipe,
            ),
        )?;
        let stdin = if hold_input {
            Some(owner.take_stdin()?)
        } else {
            None
        };
        let (ready_sender, ready) = tokio::sync::oneshot::channel();
        let (out_sender, stdout) = std::sync::mpsc::channel();
        let mut out = owner.take_stdout()?;
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = out
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .and_then(|_| {
                    if bytes.len() <= 1024 * 1024 {
                        Ok(bytes)
                    } else {
                        Err(std::io::Error::other("update stdout exceeds bound"))
                    }
                });
            let _ = out_sender.send(result);
        });
        let (err_sender, stderr) = std::sync::mpsc::channel();
        let mut err = owner.take_stderr()?;
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut ready_sender = Some(ready_sender);
            let result = (|| {
                let mut byte = [0];
                while err.read(&mut byte)? != 0 {
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"headless stdin signal ready\n")
                        && let Some(sender) = ready_sender.take()
                    {
                        let _ = sender.send(());
                    }
                    if bytes.len() > 1024 * 1024 {
                        return Err(std::io::Error::other("update stderr exceeds bound"));
                    }
                }
                Ok(bytes)
            })();
            let _ = err_sender.send(result);
        });
        #[cfg(not(all(target_os = "linux", feature = "test-support")))]
        drop(ready);
        Ok(Self {
            owner,
            stdin,
            stdout,
            stderr,
            #[cfg(all(target_os = "linux", feature = "test-support"))]
            ready: Some(ready),
        })
    }
    async fn finish(mut self, outcome: anyhow::Result<()>) -> anyhow::Result<std::process::Output> {
        use kuru_platform::unix::{GroupPresence, Reap, RootState};
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(60);
        let normal: anyhow::Result<()> = async {
            outcome?;
            loop {
                match self.owner.root_state() {
                    RootState::Exited => return Ok(()),
                    RootState::Running | RootState::Interrupted => {}
                    state => anyhow::bail!("update root ownership: {state:?}"),
                }
                anyhow::ensure!(Instant::now() < deadline, "update root did not exit");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        .await;
        self.owner.terminate_before_reap();
        let cleanup_deadline = Instant::now() + Duration::from_secs(10);
        self.owner
            .wait_pre_reap(Duration::from_millis(10), cleanup_deadline)
            .await;
        let reaped = self.owner.reap_if_exited();
        let mut listing = self.owner.permission_listing(cleanup_deadline);
        let presence = listing.resolve(self.owner.presence_after_reap()).await;
        drop(self.stdin.take());
        let stdout = self.stdout.recv_timeout(Duration::from_secs(10));
        let stderr = self.stderr.recv_timeout(Duration::from_secs(10));
        normal?;
        let Reap::Reaped(status) = reaped else {
            anyhow::bail!("update root not reaped: {reaped:?}");
        };
        anyhow::ensure!(
            matches!(presence, GroupPresence::Absent | GroupPresence::Recycled),
            "update group not absent: {presence:?}"
        );
        Ok(std::process::Output {
            status,
            stdout: stdout??,
            stderr: stderr??,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_update_sigint_awaits_the_same_build_and_descendant_before_return()
-> anyhow::Result<()> {
    kuru_memory::test_support::closing(async {
        use kuru_platform::unix::RootState;
        use std::{
            os::unix::fs::PermissionsExt,
            time::{Duration, Instant},
        };
        let fixture = tempfile::tempdir()?;
        let root = fixture.path();
        let bin = root.join("installation");
        let checkout = root.join("checkout");
        let tools = root.join("tools");
        for directory in [&bin, &checkout, &tools] {
            std::fs::create_dir(directory)?;
        }
        let executable = bin.join("kuru");
        copy_executable(&executable);
        let original = image_facts(&executable)?;
        let ready = root.join("build.ready");
        let root_pid = root.join("kuru.pid");
        let child_pid = root.join("build.pid");
        let descendant_pid = root.join("descendant.pid");
        std::fs::write(
            tools.join("mise"),
            br#"#!/bin/sh
/bin/sh -c 'printf "%s" "$$" > "$KURU_DESCENDANT_PID"; exec /bin/sleep 3600' &
printf '%s' "$$" > "$KURU_BUILD_PID"
while [ ! -s "$KURU_DESCENDANT_PID" ]; do :; done
printf 'actual build and descendant started' > "$KURU_BUILD_READY"
wait
"#,
        )?;
        std::fs::set_permissions(tools.join("mise"), std::fs::Permissions::from_mode(0o755))?;
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "printf '%s' \"$$\" > \"$1\"; shift; exec \"$@\"",
                "source-interrupt-fixture",
            ])
            .arg(&root_pid)
            .arg(&executable)
            .args(["update", "--source"])
            .arg(&checkout)
            .env("PATH", &tools)
            .env("KURU_BUILD_READY", &ready)
            .env("KURU_BUILD_PID", &child_pid)
            .env("KURU_DESCENDANT_PID", &descendant_pid);
        let mut child = InterruptedUpdate::start(command, false)?;
        let body = async {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !ready.exists() {
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "source build never reached actual launch"
                );
                anyhow::ensure!(
                    matches!(child.owner.root_state(), RootState::Running),
                    "updater exited before actual build launch"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let pid: i32 = std::fs::read_to_string(&root_pid)?.parse()?;
            anyhow::ensure!(pid > 1 && matches!(child.owner.root_state(), RootState::Running));
            // Retained, unreaped fixed launcher root; no await before signal.
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGINT,
            )?;
            Ok(())
        }
        .await;
        let output = child.finish(body).await?;
        anyhow::ensure!(
            output.status.code() == Some(130),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(image_facts(&executable)?, original);
        assert!(!bin.join(".kuru-update").exists());
        for path in [&child_pid, &descendant_pid] {
            let pid: i32 = std::fs::read_to_string(path)?.parse()?;
            // Observation only, never signal from a released numeric identity.
            assert_eq!(
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
                Err(nix::errno::Errno::ESRCH),
                "owned build process remains observable"
            );
        }
        Ok(())
    })
    .await
}

#[cfg(all(target_os = "linux", feature = "test-support"))]
#[path = "support/memory.rs"]
mod linux_memory;

#[cfg(all(target_os = "linux", feature = "test-support"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaced_linux_running_image_opens_its_own_managed_memory_without_candidate_execution()
-> anyhow::Result<()> {
    kuru_memory::test_support::closing(async {
        use std::{io::Write, os::unix::fs::PermissionsExt, time::Duration};
        let root = kuru_memory::test_support::tempdir()?;
        let data = root.path().join("data");
        let cleanup = linux_memory::ServiceCleanup::new(root, &data.join("kuru"));
        let outcome = async {
            let root = cleanup.path();
            let configuration = linux_memory::configuration_warmed(root).await?;
            let installation = root.join("installation");
            let home = root.join("home");
            let project = root.join("project");
            for directory in [&installation, &home, &project] {
                std::fs::create_dir(directory)?;
            }
            let executable = installation.join("kuru");
            copy_executable(&executable);
            let marker = root.join("replacement-executed");
            let replacement = installation.join("requested-replacement");
            std::fs::write(
                &replacement,
                b"#!/bin/sh\nprintf incompatible > \"$KURU_EXECUTION_MARKER\"\nexit 99\n",
            )?;
            std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o755))?;
            let replacement_bytes = std::fs::read(&replacement)?;
            let mut command = Command::new(&executable);
            command
                .args(["--provider", "demo", "--no-dream", "run", "--json"])
                .current_dir(&project)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", &configuration)
                .env("XDG_DATA_HOME", &data)
                .env(
                    kuru_memory::test_support::OWNER_DIAGNOSTIC_ENV,
                    cleanup.owner_diagnostic_path(),
                )
                .env("KURU_EXECUTION_MARKER", &marker)
                .env("KURU_HEADLESS_STDIN_READY_FIXTURE", "1");
            if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            if let Some(cache) = std::env::var_os("KURU_DOLT_CACHE") {
                command.env("KURU_DOLT_CACHE", cache);
            }
            let mut child = InterruptedUpdate::start(command, true)?;
            let body = async {
                tokio::time::timeout(Duration::from_secs(30), child.ready.take().unwrap())
                    .await??;
                // Actual calling image is now mapped and awaiting input; pathname
                // replacement occurs before its memory/service/supervisor open.
                std::fs::rename(&replacement, &executable)?;
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(b"mapped image ordinary conversation")?;
                drop(child.stdin.take());
                Ok(())
            }
            .await;
            let output = child.finish(body).await?;
            anyhow::ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            anyhow::ensure!(
                value
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            );
            assert_eq!(std::fs::read(&executable)?, replacement_bytes);
            assert!(!marker.exists());
            assert!(data.exists());
            Ok(())
        }
        .await;
        cleanup.release(outcome)
    })
    .await
}
