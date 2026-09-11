#![cfg(all(windows, feature = "tooling"))]

use anyhow::{Context, Result, ensure};
use kuru_delivery::{archive::digest, command::BlockingCommand as Command};
use kuru_platform::{
    fs::{Directory, NameRetention, Privacy, regular_file_info, require_private},
    windows::{
        pipe::Pipe,
        process::{NativeChild, NativeSpawnSpec, Stdio, system_directory},
    },
};
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(30);

#[path = "support/update_trace.rs"]
mod update_trace;

fn trace_records(bytes: &[u8]) -> Result<Vec<update_trace::Record<'_>>> {
    update_trace::parse(bytes).map_err(|cause| {
        anyhow::anyhow!(
            "unexpected helper stderr ({cause}): {}",
            String::from_utf8_lossy(bytes)
        )
    })
}

#[tokio::test]
async fn trusted_helper_stderr_preserves_eof_error_and_drains_past_prefix_limit() -> Result<()> {
    let root = tempfile::tempdir()?;
    for bytes in [1024, 256 * 1024] {
        let mut spec = NativeSpawnSpec::new(
            PathBuf::from(env!("CARGO_BIN_EXE_kuru-delivery-fixture")),
            root.path().to_owned(),
        );
        spec.args = vec!["helper-stderr".into(), bytes.to_string().into()];
        spec.stderr = Stdio::Pipe;
        spec.stdout = Stdio::Pipe;
        let mut child = spec.spawn().await?;
        let mut output = child.take_stdout().context("native completion pipe")?;
        let captured = tokio::time::timeout(
            TIMEOUT,
            kuru_delivery::update::test_support::capture_stderr(&mut child),
        )
        .await;
        if captured.is_err() {
            child.terminate()?;
        }
        let status = child.wait(TIMEOUT).await?;
        let captured = captured.context("native stderr drain timed out")??;
        let mut completion = Vec::new();
        let read = tokio::time::timeout(
            TIMEOUT,
            (&mut output).take(128).read_to_end(&mut completion),
        )
        .await;
        output.close(TIMEOUT).await?;
        read.context("native completion read timed out")??;
        ensure!(
            completion == b"stderr fully written\n",
            "stderr closed before the fixture finished its full write: {completion:?}"
        );
        ensure!(!status.success(), "fixture error unexpectedly succeeded");
        ensure!(captured.starts_with("native helper diagnostic prefix\n"));
        if bytes < 64 * 1024 {
            ensure!(
                captured.contains("native helper diagnostic final error"),
                "missing EOF error: {captured}"
            );
        } else {
            ensure!(captured.len() == 64 * 1024, "diagnostic prefix cap changed");
        }
    }
    Ok(())
}

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    current: PathBuf,
    candidate: PathBuf,
    cache: PathBuf,
    marker: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("ordinary mise bin λ");
        fs::create_dir(&directory).unwrap();
        let current = directory.join("kuru.exe");
        let original = fs::read(env!("CARGO_BIN_EXE_kuru-delivery-fixture")).unwrap();
        fs::write(&current, &original).unwrap();
        let candidate = root.path().join("candidate.exe");
        let mut replacement = original.clone();
        replacement.extend_from_slice(b"KURU_CANDIDATE_MARKER");
        fs::write(&candidate, &replacement).unwrap();
        let cache = root.path().join("private helper cache");
        let marker = root.path().join("candidate-executed");
        Self {
            _root: root,
            directory,
            current,
            candidate,
            cache,
            marker,
            original,
            replacement,
        }
    }

    fn spawn(&self) -> NativeSpawnSpec {
        let mut spec = NativeSpawnSpec::new(self.current.clone(), self.directory.clone());
        spec.args = vec![
            "update".into(),
            self.candidate.clone().into(),
            self.cache.clone().into(),
        ];
        spec.environment = vec![("KURU_EXECUTION_MARKER".into(), self.marker.clone().into())];
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            spec.environment.push(("LLVM_PROFILE_FILE".into(), profile));
        }
        spec.stdin = Stdio::Pipe;
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        spec
    }

    fn receipt(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.directory.join(".kuru-update/receipt.json")).unwrap())
            .unwrap()
    }
}

async fn record(pipe: &mut Pipe) -> Result<serde_json::Value> {
    tokio::time::timeout(TIMEOUT, async {
        let mut bytes = Vec::new();
        loop {
            let byte = pipe
                .read_u8()
                .await
                .context("checkpoint pipe closed before its marker")?;
            if byte == b'\n' {
                break;
            }
            ensure!(bytes.len() < 65536, "checkpoint output exceeded limit");
            bytes.push(byte);
        }
        serde_json::from_slice(&bytes).context("fixture checkpoint is not JSON")
    })
    .await
    .context("fixture checkpoint deadline expired")?
}

async fn error_output(mut pipe: Pipe) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let read = (&mut pipe).take(65537).read_to_end(&mut bytes).await;
    let close = pipe.close(TIMEOUT).await;
    read?;
    close?;
    ensure!(bytes.len() <= 65536, "fixture stderr exceeded limit");
    Ok(bytes)
}

fn file_identity(path: &Path) -> Result<[u8; 24]> {
    Ok(regular_file_info(&fs::File::open(path)?)?
        .identity
        .to_bytes())
}

fn receipt_image(receipt: &serde_json::Value, key: &str) -> Result<[u8; 24]> {
    serde_json::from_value(receipt[key]["identity"].clone()).context("receipt image identity")
}

fn read_receipt(fixture: &Fixture) -> Result<serde_json::Value> {
    Ok(serde_json::from_slice(&fs::read(
        fixture.directory.join(".kuru-update/receipt.json"),
    )?)?)
}

fn recover(fixture: &Fixture) -> Result<()> {
    let system = system_directory()?;
    let powershell = system.join("WindowsPowerShell/v1.0/powershell.exe");
    ensure!(
        powershell.is_file(),
        "stock Windows PowerShell 5.1 is required"
    );
    let environment = fixture._root.path().join("recovery environment");
    fs::create_dir(&environment)?;
    let empty_path = environment.join("empty PATH");
    fs::create_dir(&empty_path)?;
    let mut command = Command::new(powershell);
    command
        .current_dir(&environment)
        .env_clear()
        .env(
            "SystemRoot",
            system.parent().context("Windows system root")?,
        )
        .env("PROCESSOR_ARCHITECTURE", "AMD64")
        .env("USERPROFILE", &environment)
        .env("APPDATA", &environment)
        .env("LOCALAPPDATA", &environment)
        .env("HOME", &environment)
        .env("TMP", &environment)
        .env("TEMP", &environment)
        .env("PATH", &empty_path)
        .env("KURU_EXECUTION_MARKER", &fixture.marker)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("support/install.ps1"))
        .arg("-Recover")
        .arg("-InstallDir")
        .arg(&fixture.directory);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let output = command.output()?;
    ensure!(
        output.status.success(),
        "actual PowerShell recovery failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        fs::read_dir(empty_path)?.next().is_none(),
        "recovery introduced a PATH dependency"
    );
    Ok(())
}

fn reconciled(
    fixture: &Fixture,
    before: &serde_json::Value,
    installed_identity: [u8; 24],
    new: bool,
) -> Result<()> {
    let receipt = read_receipt(fixture)?;
    ensure!(
        receipt["operation"] == before["operation"],
        "recovery replaced the original transaction"
    );
    ensure!(
        receipt["phase"] == if new { "complete" } else { "rolled_back" },
        "unexpected terminal phase: {receipt}"
    );
    ensure!(
        file_identity(&fixture.current)? == installed_identity,
        "recovery changed the accepted installed identity"
    );
    let expected_bytes = if new {
        fixture.replacement.as_slice()
    } else {
        fixture.original.as_slice()
    };
    ensure!(
        fs::read(&fixture.current)?.as_slice() == expected_bytes,
        "recovery selected incorrect bytes"
    );
    let state_path = fixture.directory.join(".kuru-update");
    for (parent, key) in [
        (&fixture.directory, "displaced"),
        (&state_path, "candidate"),
        (&state_path, "backup"),
    ] {
        ensure!(
            !parent
                .join(receipt[key].as_str().context("receipt name")?)
                .try_exists()?,
            "recorded {key} remained after recovery"
        );
    }
    let helper = Path::new(receipt["helper"].as_str().context("helper path")?);
    ensure!(
        fs::read(helper)? == fixture.original,
        "retained helper differs from trusted original"
    );
    require_private(&fs::File::open(helper)?)?;
    let state = Directory::open(
        &fixture.directory.join(".kuru-update"),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )?;
    let lease = state.lock_file(OsStr::new("install.lock"))?;
    lease
        .try_lock()
        .context("recovery leaked its install lease")?;
    ensure!(
        !fixture.marker.exists(),
        "candidate executed during update or recovery"
    );
    ensure!(
        fs::read(fixture.directory.join("unrelated.txt"))?
            == b"retain unrelated installation content",
        "unrelated install content changed"
    );
    Ok(())
}

async fn stop(child: &mut NativeChild, input: &mut Pipe, output: &mut Pipe) -> Result<()> {
    child.terminate()?;
    child
        .wait(TIMEOUT)
        .await
        .context("fixture Job did not become quiescent")?;
    input.close(TIMEOUT).await?;
    output.close(TIMEOUT).await?;
    Ok(())
}

async fn interrupted(checkpoint: &str, phase: &str, new: bool) -> Result<()> {
    let fixture = Fixture::new();
    fs::write(
        fixture.directory.join("unrelated.txt"),
        b"retain unrelated installation content",
    )?;
    let original_identity = file_identity(&fixture.current)?;
    let mut spec = fixture.spawn();
    spec.args[0] = "update-observed".into();
    spec.args.push(checkpoint.into());
    let mut child = spec.spawn().await?;
    let mut input = child.take_stdin().context("fixture stdin")?;
    let mut output = child.take_stdout().context("fixture stdout")?;
    let error = tokio::spawn(error_output(child.take_stderr().context("fixture stderr")?));
    let mut stopped = false;
    let result = async {
        let mut parent_acknowledged = false;
        loop {
            let marker = record(&mut output).await?;
            if marker["parent_acknowledged"] == true {
                parent_acknowledged = true;
                continue;
            }
            if marker.get("installed").is_some() {
                continue;
            }
            ensure!(
                marker["checkpoint"] == checkpoint && marker["phase"] == phase,
                "unexpected checkpoint {marker}"
            );
            ensure!(
                marker["helper_pid"].as_u64().context("helper PID")? != u64::from(child.id()),
                "checkpoint came from parent, not real helper"
            );
            let receipt = read_receipt(&fixture)?;
            ensure!(
                marker["operation"] == receipt["operation"],
                "checkpoint does not identify persisted transaction"
            );
            break;
        }
        ensure!(
            child.try_wait()?.is_none(),
            "checkpoint did not hold the real owned Job"
        );
        ensure!(
            parent_acknowledged == (checkpoint == "old_removed_before_complete"),
            "ACK occurred at the wrong publication boundary"
        );
        let receipt = read_receipt(&fixture)?;
        ensure!(
            receipt["phase"] == phase,
            "durable receipt moved past selected checkpoint"
        );
        ensure!(
            receipt_image(&receipt, "original")? == original_identity,
            "receipt lost original identity"
        );
        let expected = if new {
            receipt_image(&receipt, "replacement")?
        } else {
            original_identity
        };
        if checkpoint == "old_moved_before_receipt" {
            ensure!(
                !fixture.current.try_exists()?,
                "first move did not remove installed name"
            );
        } else {
            ensure!(
                file_identity(&fixture.current)? == expected,
                "installed identity does not match reached boundary"
            );
        }
        let old = fixture
            .directory
            .join(receipt["displaced"].as_str().context("old name")?);
        if matches!(checkpoint, "prepared" | "old_removed_before_complete") {
            ensure!(
                !old.try_exists()?,
                "unexpected displaced image at checkpoint"
            );
        } else {
            ensure!(
                file_identity(&old)? == original_identity && fs::read(&old)? == fixture.original,
                "displaced original changed"
            );
        }
        ensure!(
            !fixture.marker.exists(),
            "candidate executed before checkpoint"
        );
        stop(&mut child, &mut input, &mut output).await?;
        stopped = true;
        recover(&fixture)?;
        reconciled(&fixture, &receipt, expected, new)
    }
    .await;
    let cleanup = if stopped {
        Ok(())
    } else {
        stop(&mut child, &mut input, &mut output).await
    };
    let error = tokio::time::timeout(TIMEOUT, error).await;
    if let Err(failure) = result.and(cleanup) {
        let retained = fixture._root.keep();
        anyhow::bail!(
            "{checkpoint}: {failure:#}; stderr={error:?}; retained {}",
            retained.display()
        );
    }
    let bytes = error.context("stderr task timed out")???;
    trace_records(&bytes)?;
    Ok(())
}

#[tokio::test]
async fn killed_helper_after_prepared_receipt_recovers_original_identity() -> Result<()> {
    interrupted("prepared", "prepared", false).await
}

#[tokio::test]
async fn candidate_sharing_failure_restores_loaded_original_before_explicit_recovery() -> Result<()>
{
    use std::os::windows::fs::OpenOptionsExt;

    let fixture = Fixture::new();
    fs::write(
        fixture.directory.join("unrelated.txt"),
        b"retain unrelated installation content",
    )?;
    let original_identity = file_identity(&fixture.current)?;
    let mut spec = fixture.spawn();
    spec.args[0] = "update-observed".into();
    spec.args.push("old_moved_before_receipt_resume".into());
    let mut child = spec.spawn().await?;
    let mut input = child.take_stdin().context("fixture stdin")?;
    let mut output = child.take_stdout().context("fixture stdout")?;
    let errors = tokio::spawn(error_output(child.take_stderr().context("fixture stderr")?));
    let mut blocker = None;
    let result = async {
        let marker = record(&mut output).await?;
        ensure!(
            marker["checkpoint"] == "old_moved_before_receipt" && marker["phase"] == "prepared",
            "unexpected checkpoint {marker}"
        );
        ensure!(
            marker["helper_pid"].as_u64().context("helper PID")? != u64::from(child.id()),
            "checkpoint must originate in the actual helper"
        );
        let before = read_receipt(&fixture)?;
        ensure!(
            before["operation"] == marker["operation"],
            "wrong persisted transaction"
        );
        ensure!(
            !fixture.current.try_exists()?,
            "old image was not moved before the conflict"
        );
        let candidate = fixture
            .directory
            .join(".kuru-update")
            .join(before["candidate"].as_str().context("candidate name")?);
        // Permit reads/writes, but deny DELETE sharing: the real second move
        // fails while the helper can still validate all recovery bytes.
        blocker = Some(
            fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(&candidate)?,
        );
        ensure!(
            file_identity(&candidate)? == receipt_image(&before, "replacement")?,
            "wrong candidate identity"
        );
        let probe = candidate.with_file_name("sharing-conflict-probe.exe");
        let conflict =
            fs::rename(&candidate, &probe).expect_err("retained native handle must deny the move");
        ensure!(
            conflict.raw_os_error() == Some(32),
            "expected native sharing violation, got {conflict}"
        );
        ensure!(
            !probe.try_exists()? && candidate.is_file(),
            "failed probe changed candidate namespace"
        );
        input.write_all(b"r").await?;
        input.flush().await?;
        let status = child.wait(TIMEOUT).await?;
        ensure!(
            !status.success(),
            "conflicting publication unexpectedly succeeded"
        );

        // This is the critical automatic rollback observation. No recovery
        // command has run; the original loaded object's name/bytes are restored.
        ensure!(
            file_identity(&fixture.current)? == original_identity
                && fs::read(&fixture.current)? == fixture.original,
            "helper failed to restore original identity and bytes automatically"
        );
        let pending = read_receipt(&fixture)?;
        ensure!(
            pending["operation"] == before["operation"] && pending["phase"] == "old_moved",
            "failed cleanup was not retained explicitly: {pending}"
        );
        ensure!(
            candidate.is_file() && !fixture.marker.exists(),
            "candidate was lost or executed"
        );
        ensure!(
            fs::read(fixture.directory.join("unrelated.txt"))?
                == b"retain unrelated installation content",
            "unrelated bytes changed"
        );
        drop(blocker.take());
        recover(&fixture)?;
        reconciled(&fixture, &before, original_identity, false)
    }
    .await;
    let cleanup = stop(&mut child, &mut input, &mut output).await;
    drop(blocker);
    let errors = tokio::time::timeout(TIMEOUT, errors).await;
    if let Err(failure) = result.and(cleanup) {
        let retained = fixture._root.keep();
        anyhow::bail!(
            "handled sharing conflict: {failure:#}; stderr={errors:?}; retained {}",
            retained.display()
        );
    }
    let bytes = errors.context("stderr task timed out")???;
    ensure!(
        String::from_utf8_lossy(&bytes).contains("publication failed; recovery receipt retained"),
        "wrong failure cause: {}",
        String::from_utf8_lossy(&bytes)
    );
    Ok(())
}

#[tokio::test]
async fn killed_helper_after_old_move_before_receipt_recovers_original_identity() -> Result<()> {
    interrupted("old_moved_before_receipt", "prepared", false).await
}

#[tokio::test]
async fn killed_helper_after_candidate_move_before_receipt_keeps_new_identity() -> Result<()> {
    interrupted("candidate_moved_before_receipt", "old_moved", true).await
}

#[tokio::test]
async fn killed_helper_after_published_receipt_before_ack_keeps_new_identity() -> Result<()> {
    interrupted("candidate_published", "published", true).await
}

#[tokio::test]
async fn killed_helper_after_old_cleanup_before_complete_finishes_recovery() -> Result<()> {
    interrupted("old_removed_before_complete", "published", true).await
}

#[tokio::test]
async fn post_ack_parent_loss_leaves_actual_helper_to_finish_owned_cleanup() -> Result<()> {
    let fixture = Fixture::new();
    fs::write(
        fixture.directory.join("unrelated.txt"),
        b"retain unrelated installation content",
    )?;
    let mut spec = fixture.spawn();
    spec.args[0] = "update-parent-loss".into();
    let mut child = spec.spawn().await?;
    let mut input = child.take_stdin().context("fixture stdin")?;
    let mut output = child.take_stdout().context("fixture stdout")?;
    let error = tokio::spawn(error_output(child.take_stderr().context("fixture stderr")?));
    let result = async {
        let installed = record(&mut output).await?;
        ensure!(
            installed["installed"].is_string() && installed["cleanup_pending"] == true,
            "parent did not report verified publication"
        );
        ensure!(
            record(&mut output).await?["parent_acknowledged"] == true,
            "parent loss preceded acknowledgment"
        );
        let status = child.wait(TIMEOUT).await?;
        ensure!(
            status.code() == Some(42),
            "expected deliberate parent loss, got {status}"
        );
        let receipt = read_receipt(&fixture)?;
        reconciled(
            &fixture,
            &receipt,
            receipt_image(&receipt, "replacement")?,
            true,
        )
    }
    .await;
    let cleanup = stop(&mut child, &mut input, &mut output).await;
    let error = tokio::time::timeout(TIMEOUT, error).await;
    if let Err(failure) = result.and(cleanup) {
        let retained = fixture._root.keep();
        anyhow::bail!(
            "post-ACK parent loss: {failure:#}; stderr={error:?}; retained {}",
            retained.display()
        );
    }
    let bytes = error.context("stderr task timed out")???;
    trace_records(&bytes)?;
    Ok(())
}

async fn line(pipe: &mut kuru_platform::windows::pipe::Pipe) -> String {
    tokio::time::timeout(TIMEOUT, async {
        let mut bytes = Vec::new();
        loop {
            let byte = pipe.read_u8().await?;
            if byte == b'\n' {
                break;
            }
            if bytes.len() >= 65536 {
                return Err(io::Error::other("fixture output overflow"));
            }
            bytes.push(byte);
        }
        String::from_utf8(bytes).map_err(io::Error::other)
    })
    .await
    .unwrap()
    .unwrap()
}

fn assert_private_receipt(fixture: &Fixture) {
    let directory = Directory::open(
        &fixture.directory.join(".kuru-update"),
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )
    .unwrap();
    for name in ["receipt.json", "install.lock"] {
        require_private(&directory.read(OsStr::new(name)).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn loaded_image_acknowledges_exact_new_bytes_while_old_process_is_still_alive() {
    let fixture = Fixture::new();
    let original_file = fs::File::open(&fixture.current).unwrap();
    let original_identity = regular_file_info(&original_file).unwrap().identity;
    drop(original_file);
    let mut child = fixture.spawn().spawn().await.unwrap();
    let errors = tokio::spawn(error_output(child.take_stderr().unwrap()));
    let mut input = child.take_stdin().unwrap();
    let mut output = child.take_stdout().unwrap();
    let ack: serde_json::Value = serde_json::from_str(&line(&mut output).await).unwrap();
    assert_eq!(ack["installed"], fixture.current.to_string_lossy().as_ref());
    assert_eq!(ack["cleanup_pending"], true);
    assert!(
        child.try_wait().unwrap().is_none(),
        "old process exited before observer checked publication"
    );
    assert_eq!(fs::read(&fixture.current).unwrap(), fixture.replacement);
    assert!(
        !fixture.marker.exists(),
        "candidate ran during verification or helper execution"
    );
    let receipt = fixture.receipt();
    let displaced = fixture
        .directory
        .join(receipt["displaced"].as_str().unwrap());
    assert_eq!(
        regular_file_info(&fs::File::open(&displaced).unwrap())
            .unwrap()
            .identity,
        original_identity
    );
    assert_eq!(fs::read(&displaced).unwrap(), fixture.original);
    assert_private_receipt(&fixture);
    let helper = Path::new(receipt["helper"].as_str().unwrap());
    assert_eq!(fs::read(helper).unwrap(), fixture.original);
    assert!(
        helper
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&digest(&fixture.original))
    );
    require_private(&fs::File::open(helper).unwrap()).unwrap();
    input.write_all(b"x").await.unwrap();
    input.flush().await.unwrap();
    input.close(TIMEOUT).await.unwrap();
    assert!(child.wait(TIMEOUT).await.unwrap().success());
    output.close(TIMEOUT).await.unwrap();
    let trace = tokio::time::timeout(TIMEOUT, errors)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let trace = trace_records(&trace).unwrap();
    let mut records = trace.iter();
    let mut previous = 0;
    for phase in [
        "started",
        "receive_request",
        "acquire_installation_lock",
        "installation_lock_acquired",
        "hash_loaded_helper",
        "loaded_helper_verified",
        "verify_original",
        "copy_candidate",
        "copy_rollback",
        "prepared_receipt_saved",
        "publish",
        "old_moved_before_receipt",
        "candidate_moved_before_receipt",
        "send_verified_publication_acknowledgment",
        "verified_publication_acknowledgment_sent",
        "wait_parent_exit",
    ] {
        let elapsed = records
            .find(|record| record.phase == phase)
            .unwrap_or_else(|| panic!("missing ordered phase {phase}: {trace:?}"))
            .elapsed_ms;
        assert!(
            elapsed >= previous,
            "helper elapsed records moved backward: {trace:?}"
        );
        previous = elapsed;
    }
    assert!(
        trace.iter().any(|record| matches!(
            record.phase,
            "complete" | "parent_still_alive_cleanup_pending"
        )),
        "helper did not report its actual bounded cleanup outcome: {trace:?}"
    );
    // The cached current-version image is deliberately persistent. A fresh
    // public invocation runs the new image only after verified acknowledgment.
    assert!(helper.is_file());
    let result = Command::new(&fixture.current)
        .arg("--version")
        .env("KURU_EXECUTION_MARKER", &fixture.marker)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(fixture.marker.is_file());
}

#[tokio::test]
async fn corrupt_existing_trusted_helper_is_never_executed_and_preserves_current_image() {
    let fixture = Fixture::new();
    let cache = Directory::ensure_private(&fixture.cache).unwrap();
    let name = format!("x86_64-pc-windows-msvc-{}.exe", digest(&fixture.original));
    let mut wrong = cache.create_new(OsStr::new(&name)).unwrap();
    std::io::Write::write_all(&mut wrong, b"not the trusted current image").unwrap();
    drop(wrong);
    let mut child = fixture.spawn().spawn().await.unwrap();
    let mut error = child.take_stderr().unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(TIMEOUT, error.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(!child.wait(TIMEOUT).await.unwrap().success());
    error.close(TIMEOUT).await.unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("cached trusted helper is corrupt")
    );
    assert_eq!(fs::read(&fixture.current).unwrap(), fixture.original);
    assert!(!fixture.marker.exists());
    assert!(!fixture.directory.join(".kuru-update").exists());
}
