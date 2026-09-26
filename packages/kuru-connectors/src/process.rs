//! Windows consumers share the platform's single atomic process boundary.

use std::{collections::BTreeMap, ffi::OsString, path::Path, time::Duration};

use anyhow::{Context, Result};
use kuru_platform::windows::process::{self, NativeChild, NativeSpawnSpec, Stdio};

pub(crate) fn configured(
    program: &str,
    args: &[String],
    overrides: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<NativeSpawnSpec> {
    let environment = process::merge_environment(
        std::env::vars_os(),
        overrides
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    )?;
    process::configured_command(
        program.as_ref(),
        &args.iter().map(OsString::from).collect::<Vec<_>>(),
        cwd,
        environment,
    )
    .context("cannot resolve configured Windows command")
}

pub(crate) fn configured_finite(
    program: &str,
    args: &[String],
    environment: Vec<(OsString, OsString)>,
    cwd: &Path,
) -> Result<NativeSpawnSpec> {
    process::configured_command(
        program.as_ref(),
        &args.iter().map(OsString::from).collect::<Vec<_>>(),
        cwd,
        environment,
    )
    .context("cannot resolve configured Windows hook command")
}

pub(crate) async fn piped(
    program: &str,
    args: &[String],
    overrides: &BTreeMap<String, String>,
    cwd: &Path,
    capture_error: bool,
) -> Result<NativeChild> {
    let mut spec = configured(program, args, overrides, cwd)?;
    spec.stdin = Stdio::Pipe;
    spec.stdout = Stdio::Pipe;
    spec.stderr = if capture_error {
        Stdio::Pipe
    } else {
        Stdio::Null
    };
    spec.spawn()
        .await
        .context("cannot start configured Windows command")
}

pub(crate) async fn stop(child: &mut NativeChild) -> Result<()> {
    child.terminate()?;
    child
        .wait(Duration::from_secs(5))
        .await
        .context("Windows subprocess tree did not terminate")?;
    Ok(())
}
