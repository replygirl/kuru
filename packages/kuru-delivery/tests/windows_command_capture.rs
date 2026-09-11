#![cfg(all(windows, feature = "tooling"))]

use anyhow::{Context, Result, ensure};
use kuru_delivery::command::{Command, output};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
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
