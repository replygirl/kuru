#![cfg(all(windows, feature = "tooling"))]

use anyhow::{Context, Result, ensure};
use kuru_delivery::command::{Command, output};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::Path,
    time::Duration,
};
use tokio::time::{Instant, sleep_until};

async fn acquire_released_lease(file: &File, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::Error(error)) => return Err(error),
            Err(TryLockError::WouldBlock) => (),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "descendant lease remained locked after owned cleanup",
            ));
        }
        sleep_until(deadline.min(Instant::now() + Duration::from_millis(10))).await;
    }
}

#[tokio::test]
async fn lease_observation_rejects_held_lock_and_accepts_explicit_release() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("held-control.lock");
    let owner = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    owner.try_lock()?;
    let observer = OpenOptions::new().read(true).write(true).open(path)?;
    let error = acquire_released_lease(&observer, Duration::from_millis(30))
        .await
        .expect_err("a retained real lock must not pass the cleanup observation");
    ensure!(error.kind() == io::ErrorKind::TimedOut, "{error}");
    owner.unlock()?;
    acquire_released_lease(&observer, Duration::ZERO).await?;
    Ok(())
}

#[tokio::test]
async fn exited_root_output_survives_descendant_quiescence_failure_and_owned_cleanup() -> Result<()>
{
    let root = tempfile::tempdir()?;
    let lease = root.path().join("live-descendant.lock");
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    command
        .env_clear()
        .current_dir(root.path())
        .arg("command-output-before-tree-wait")
        .arg(&lease);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let error = output(&mut command, Duration::from_secs(30))
        .await
        .expect_err("a live descendant cannot satisfy command quiescence");
    let diagnostic = error.to_string();
    for required in [
        "wait for native process tree quiescence",
        "native process tree did not become quiescent",
        "stdout_eof=true stderr_eof=true",
        "cleanup=Ok(()) stdout_close=Ok(()) stderr_close=Ok(())",
        "native captured stdout prefix",
        "native fixture failure before tree wait",
        "native root failed after both streams",
    ] {
        ensure!(
            diagnostic.contains(required),
            "missing {required}: {diagnostic}"
        );
    }
    ensure!(
        !diagnostic.contains("excluded stdout tail"),
        "diagnostic prefix cap was not enforced"
    );
    ensure!(
        diagnostic.len() < 66 * 1024,
        "bounded output diagnostic grew unexpectedly"
    );
    // The descendant acquired this real lock before the root emitted output;
    // only terminating the enclosing owned tree can release its pending lease.
    // Windows may defer unlocking after termination (LockFileEx's documented
    // contract). Observe real release on one retained file, within a deadline.
    let held = OpenOptions::new().read(true).write(true).open(lease)?;
    acquire_released_lease(&held, Duration::from_secs(5))
        .await
        .with_context(|| format!("observe descendant lease release; {diagnostic}"))?;
    Ok(())
}

/// Every listed Job member names its full image path, and one of them is the
/// fixture executable itself.
fn ensure_full_image_paths(diagnostic: &str) -> Result<()> {
    let fixture = fs::canonicalize(env!("CARGO_BIN_EXE_kuru-delivery-fixture"))?;
    let mut found = false;
    for listed in diagnostic.split(" image=").skip(1) {
        let image = [" cpu=", " sample_error="]
            .iter()
            .filter_map(|field| listed.find(field))
            .min()
            .map_or(listed, |end| &listed[..end]);
        ensure!(
            Path::new(image).is_absolute(),
            "member image {image:?} is not a full path: {diagnostic}"
        );
        found |= fs::canonicalize(image).is_ok_and(|image| image == fixture);
    }
    ensure!(
        found,
        "no member image is the fixture executable: {diagnostic}"
    );
    Ok(())
}

fn held_tree_command(root: &std::path::Path, mode: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery-fixture"));
    command
        .env_clear()
        .current_dir(root)
        .arg(mode)
        .arg(root.join("live-descendant.lock"));
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

#[tokio::test]
async fn quiescence_failure_lists_the_live_descendant_before_owned_cleanup() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut command = held_tree_command(root.path(), "command-output-before-tree-wait");
    let error = output(&mut command, Duration::from_secs(30))
        .await
        .expect_err("a live descendant cannot satisfy command quiescence");
    let diagnostic = error.to_string();
    // The descendant held its lease before the root failed, so it is a live
    // Job member when the tree is observed, ahead of termination.
    for required in [
        "wait for native process tree quiescence",
        &format!("command={:?}", env!("CARGO_BIN_EXE_kuru-delivery-fixture")),
        r#"arguments=["command-output-before-tree-wait""#,
        &format!("directory={:?}", root.path()),
        "tree before cleanup: root pid=",
        "state=exited(1)",
        "job active=",
        "members=[pid=",
        "\\kuru-delivery-fixture.exe cpu=",
    ] {
        ensure!(
            diagnostic.contains(required),
            "missing {required}: {diagnostic}"
        );
    }
    ensure_full_image_paths(&diagnostic)
}

#[tokio::test]
async fn timeout_lists_the_running_root_before_owned_cleanup() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut command = held_tree_command(root.path(), "command-held-descendant");
    let error = output(&mut command, Duration::from_secs(2))
        .await
        .expect_err("a blocked root must reach the timeout arm");
    let diagnostic = error.to_string();
    for required in [
        "read native stdout/stderr after ",
        "tool timed out",
        r#"arguments=["command-held-descendant""#,
        "tree before cleanup: root pid=",
        "state=running cpu=",
        "\\kuru-delivery-fixture.exe cpu=",
    ] {
        ensure!(
            diagnostic.contains(required),
            "missing {required}: {diagnostic}"
        );
    }
    ensure!(
        !diagnostic.contains("job_error=") && !diagnostic.contains("image_error="),
        "tree observation failed: {diagnostic}"
    );
    // A timed-out fixture names each Job member by its full image path.
    ensure_full_image_paths(&diagnostic)
}
