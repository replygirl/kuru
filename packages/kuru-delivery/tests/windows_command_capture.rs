#![cfg(all(windows, feature = "tooling"))]

use anyhow::{Result, ensure};
use kuru_delivery::command::{Command, output};
use std::{fs::OpenOptions, time::Duration};

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
    let held = OpenOptions::new().read(true).write(true).open(lease)?;
    held.try_lock()?;
    Ok(())
}
