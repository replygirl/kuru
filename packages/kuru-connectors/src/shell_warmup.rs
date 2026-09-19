//! Shared stock-PowerShell engine warm-up for Windows test binaries.
//!
//! A cold `powershell.exe` v5.1 engine start (loading
//! `System.Management.Automation.dll`, `types.ps1xml`/`format.ps1xml`, and
//! building the initial runspace) can stall for tens of seconds before any
//! script line executes — work that happens before `-NoProfile`/
//! `-NonInteractive` skip anything. Tests that spawn the ToolHost stock shell
//! for the first time in a process can trip the shell tool's own
//! `timeout_ms` budget on that cold start alone, not on the tested command.
//!
//! This module drives one throwaway command through the real
//! `ToolHost::execute("shell", …)` path — the exact executable resolution,
//! flags, reconstructed `PSModulePath` policy, and module bootstrap a real
//! `tool shell` call uses (see `tools.rs`'s `shell_inner`) — so the warm-up
//! actually pre-touches the code path under test instead of a differently
//! flagged ad hoc `powershell.exe` invocation. It intentionally reuses that
//! path rather than re-deriving its launch flags, so the two cannot drift.
//!
//! Distinct from and unrelated to `kuru-delivery`'s `warm_up_powershell_engine`
//! (used by the install/update bootstrap fixture in
//! `apps/kuru-tui/tests/embedded_runtime.rs`): that helper launches
//! `powershell.exe` directly, with different arguments and without the
//! ToolHost's module-import source wrapping, so it does not warm this path.

#[cfg(windows)]
use crate::ToolHost;
#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use kuru_core::Config;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use tokio::sync::OnceCell;

/// The warm-up's own generous outer bound. Distinct from and not counted
/// against the shell tool's own `timeout_ms` (product default 30_000ms,
/// validated range 1..=120_000ms) — this module changes neither.
#[cfg(windows)]
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(130);

#[cfg(windows)]
static WARMED: OnceCell<Result<(), String>> = OnceCell::const_new();

/// Warm the ToolHost stock PowerShell 5.1 engine once per process before the
/// first timed `tool shell` call. Safe to call from every test that reaches
/// the stock shell; only the first call in a process does any work, and every
/// later call (including from other threads/tasks racing the first) awaits
/// and returns that same outcome. A no-op on non-Windows targets.
pub async fn warm_up_stock_powershell_engine() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let outcome = WARMED.get_or_init(run_once).await;
        return outcome.clone().map_err(|message| anyhow::anyhow!(message));
    }
    #[cfg(not(windows))]
    Ok(())
}

#[cfg(windows)]
async fn run_once() -> Result<(), String> {
    match tokio::time::timeout(WARM_UP_TIMEOUT, attempt()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{error:#}")),
        Err(_) => Err(format!(
            "PowerShell warm-up exceeded its own {WARM_UP_TIMEOUT:?} bound (distinct from and \
             not counted against the shell tool's own timeout); a stall here points at \
             PowerShell engine/host cold start, not the tested command"
        )),
    }
}

#[cfg(windows)]
async fn attempt() -> anyhow::Result<()> {
    let root = tempfile::tempdir().context("create warm-up workspace")?;
    let host = ToolHost::new(
        root.path(),
        &Config {
            allow_shell: true,
            ..Config::default()
        },
    )
    .context("construct warm-up ToolHost")?;
    host.execute(
        "shell",
        serde_json::json!({"command": "$null = 1", "timeout_ms": 120_000}),
    )
    .await
    .context(
        "warm up the ToolHost stock PowerShell 5.1 engine ahead of the timed shell tool call \
         (distinct from and not counted against the shell tool's own timeout); a stall here \
         points at PowerShell engine/host cold start, not the tested command",
    )?;
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::warm_up_stock_powershell_engine;

    #[tokio::test]
    async fn warms_up_once_and_is_safe_to_call_concurrently() {
        let (first, second) = tokio::join!(
            warm_up_stock_powershell_engine(),
            warm_up_stock_powershell_engine()
        );
        first.unwrap();
        second.unwrap();
    }
}
