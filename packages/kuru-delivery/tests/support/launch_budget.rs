//! Derivation record for the delivery fixtures' launch bounds.
//!
//! Every bounded launch here already ends on its own event:
//! `command::output` and `command::bounded_output` return once the command has
//! exited and both pipes reached EOF, and the native bootstrap acknowledgment
//! wait returns at its newline. A bound only decides when a launch that has
//! not reached its event is reported as stalled, so each one is taken from the
//! budget that governs the launch, never chosen for runner speed:
//!
//! - Coverage-dispatched launches with no product or vendor budget of their own
//!   (`powershell_diagnostics.rs` wrapper launches: mise, `cmd.exe` and
//!   PowerShell 7 startup; `bootstrap_windows.rs`: stock PowerShell startup and
//!   `Add-Type` compilation, around `install.ps1`'s own 60 s stream, download
//!   and DEFLATE deadlines and its 30 s recovery wait, which end the script
//!   through its exit) wait for their event until [`until_job_deadline`].
//! - The previous release's updater (`previous_updater.rs`, also the candidate
//!   support generator in `previous_release_update.rs`) runs in the `install`
//!   and `verify-staged` jobs, which record no job start, so it takes the
//!   product's composed handoff budget, `kuru_delivery::update_budget::handoff`.
//! - Native mise launches (`mise_acceptance.rs`) take mise's stalled-request
//!   budget, [`mise_stalled_request`], plus the memory package's backstop for
//!   one Kuru lifecycle that creates its store.

#![allow(dead_code)] // Each includer uses a subset.

use std::{sync::LazyLock, time::Duration};

/// The mise release whose documented HTTP settings these constants cite; the
/// pin test requires it to be the release the workflows install.
pub const MISE_VERSION: &str = "2026.9.18";
/// mise `http_timeout` default ("30s", `settings.toml` at v2026.9.18): the
/// connect and each idle read of one HTTP attempt.
pub const MISE_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// mise `http_retries` default (3) at v2026.9.18: transient failures, read
/// timeouts included, are retried this many times.
pub const MISE_HTTP_RETRIES: usize = 3;
/// mise's documented retry backoff at v2026.9.18 ("~200ms / ~1s / ~4s /
/// ~15s", with jitter the documentation does not bound).
pub const MISE_RETRY_BACKOFF: [Duration; 4] = [
    Duration::from_millis(200),
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(15),
];

/// One mise request that stalls on every attempt: each attempt's
/// `http_timeout` and the backoff before each retry. The fixture's isolated
/// environment overrides none of these settings, and every other request is
/// answered by the local fixture server. `fetch_remote_versions_timeout`
/// (20 s) is shorter than `http_timeout`, so it adds nothing.
pub fn mise_stalled_request() -> Duration {
    let attempts = u32::try_from(MISE_HTTP_RETRIES + 1).expect("mise attempts fit u32");
    MISE_HTTP_TIMEOUT.saturating_mul(attempts)
        + MISE_RETRY_BACKOFF[..MISE_HTTP_RETRIES]
            .iter()
            .sum::<Duration>()
}

/// The coverage job this process runs under, as its workflow records it: the
/// job start (`KURU_COVERAGE_JOB_STARTED`) and the job limit
/// (`KURU_COVERAGE_JOB_MINUTES`), or neither.
fn recorded_job() -> Option<(u64, u64)> {
    let read = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{name} is not whole seconds or minutes: {value}"))
        })
    };
    match (
        read("KURU_COVERAGE_JOB_STARTED"),
        read("KURU_COVERAGE_JOB_MINUTES"),
    ) {
        (Some(started), Some(minutes)) => Some((started, minutes)),
        (None, None) => None,
        (started, minutes) => panic!(
            "a coverage job records both its start and its limit, found start {started:?} \
             and limit {minutes:?}"
        ),
    }
}

/// Every `KURU_COVERAGE_JOB_MINUTES` the workflows give their coverage jobs.
pub fn workflow_job_minutes() -> Vec<u64> {
    let workflows =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let mut limits = Vec::new();
    for workflow in ["ci.yml", "native-tests.yml"] {
        let text = std::fs::read_to_string(workflows.join(workflow))
            .unwrap_or_else(|error| panic!("read {workflow}: {error}"));
        limits.extend(
            text.lines()
                .filter_map(|line| line.trim().strip_prefix("KURU_COVERAGE_JOB_MINUTES: \""))
                .map(|value| {
                    value
                        .strip_suffix('"')
                        .and_then(|minutes| minutes.parse::<u64>().ok())
                        .unwrap_or_else(|| panic!("{workflow} job limit is not minutes: {value}"))
                }),
        );
    }
    assert!(!limits.is_empty(), "no coverage job limit found");
    limits
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock precedes the Unix epoch")
        .as_secs()
}

/// A run that records no job (a local run, or the release workflow's ordinary
/// `mise run test` pass) gets a window starting at this process's first
/// bounded launch, with the shortest limit any coverage job has. In the
/// release `tests` job that window is not tied to the job's own start; see
/// the derivation record's follow-on.
static LOCAL_JOB: LazyLock<(u64, u64)> = LazyLock::new(|| {
    let minutes = workflow_job_minutes().into_iter().min().unwrap();
    (unix_now(), minutes)
});

/// Time left before the inner test deadline of the job `(started, minutes)`
/// at `now`, as `kuru_delivery::coverage::shard_deadline` places it inside the
/// job limit; a passed deadline is an error naming it.
pub fn remaining_at(job: (u64, u64), now: u64) -> Result<Duration, String> {
    let (started, minutes) = job;
    let deadline = kuru_delivery::coverage::shard_deadline(started, minutes)
        .map_err(|error| format!("{error:#}"))?;
    deadline
        .checked_sub(now)
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .ok_or_else(|| {
            format!("the coverage job's inner test deadline {deadline} passed (now {now})")
        })
}

/// Bound for a launch with no product or vendor budget: wait for its event
/// until the running coverage job's inner test deadline. This is the deadline
/// the coverage orchestrator itself enforces on the test process, inside the
/// job's evidence reserve, so a stall is reported in the job by whichever of
/// the two observes it first. A run that records no job gets the window a
/// coverage job of the shortest limit gives a test that starts it. Coverage
/// jobs set the limit only on their test steps while the start persists for
/// the job, and only those steps run these binaries, so a run that records
/// one without the other is refused.
pub fn until_job_deadline() -> Duration {
    let job = recorded_job().unwrap_or_else(|| *LOCAL_JOB);
    remaining_at(job, unix_now()).unwrap_or_else(|error| panic!("{error}"))
}
