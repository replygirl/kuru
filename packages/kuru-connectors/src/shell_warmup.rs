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

/// Runs `future` to completion on a fresh, dedicated OS thread that owns its
/// own `current_thread` Tokio runtime, then joins that thread and returns the
/// future's output.
///
/// Safe to call whether or not the calling thread is already inside a Tokio
/// runtime, and regardless of that ambient runtime's flavor. Building and
/// entering a *second* runtime directly on a thread that is already driving
/// one panics with "Cannot start a runtime from within a runtime"; the usual
/// escape hatch, `tokio::task::block_in_place` + `Handle::block_on`, only
/// works when the ambient runtime is the `multi_thread` flavor, not
/// `current_thread` (the flavor `#[tokio::test]` defaults to, and the flavor
/// several call sites build for their own sync-test warm-up). Running on an
/// entirely separate OS thread sidesteps both constraints: Tokio's
/// "already inside a runtime" check is thread-local, so a brand-new thread
/// has no ambient runtime to collide with.
///
/// Compiled on Windows (its real caller, [`ensure_stock_powershell_warm`])
/// and under `cfg(test)` on every platform, so its runtime-nesting logic
/// stays covered by a non-Windows regression test even though it would
/// otherwise be dead code on non-Windows release builds.
#[cfg(any(windows, test))]
fn block_on_dedicated_thread<F>(future: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build dedicated warm-up runtime")
            .block_on(future)
    })
    .join()
    .expect("dedicated warm-up thread panicked")
}

/// Synchronous, call-site-safe wrapper around
/// [`warm_up_stock_powershell_engine`] for test helpers that cannot
/// `.await` directly (plain `fn` tests, and `Sandbox::new()` constructors
/// shared by sync and async tests). Safe to call from a plain thread or from
/// inside an already-running Tokio runtime of any flavor; see
/// [`block_on_dedicated_thread`] for why that is sound. Every call still
/// shares `warm_up_stock_powershell_engine`'s process-wide once-per-process
/// semantics.
#[cfg(windows)]
pub fn ensure_stock_powershell_warm() {
    block_on_dedicated_thread(warm_up_stock_powershell_engine()).expect("stock PowerShell warm-up");
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

// `block_on_dedicated_thread` contains no Windows-specific logic — it is the
// generic fix for nesting a blocking `block_on` inside a call that may
// already be running on a Tokio runtime. Exercise it on every platform so
// the core bug (see module docs on the Windows-only callers above) has a
// non-Windows regression test.
#[cfg(test)]
mod dedicated_thread_tests {
    use super::block_on_dedicated_thread;

    #[test]
    fn runs_a_plain_future_when_not_already_inside_a_runtime() {
        let value = block_on_dedicated_thread(async { 6 * 7 });
        assert_eq!(value, 42);
    }

    // Mirrors the real bug this module fixes: calling `Runtime::block_on`
    // directly on a thread that is already inside a `current_thread`-flavor
    // runtime (the flavor `#[tokio::test]` defaults to) panics with "Cannot
    // start a runtime from within a runtime", and `block_in_place` cannot be
    // used on that flavor either. Running the future on a dedicated thread
    // must succeed here without panicking.
    #[tokio::test]
    async fn runs_from_inside_an_existing_current_thread_runtime_without_panicking() {
        let value = block_on_dedicated_thread(async { "warmed" });
        assert_eq!(value, "warmed");
    }
}
