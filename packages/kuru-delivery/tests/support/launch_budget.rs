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
//!   Windows updating parent's serial waits on its trusted helper,
//!   [`update_handoff`], composed from the product's
//!   `kuru_delivery::update_budget` constants.
//! - Native mise launches (`mise_acceptance.rs`) take mise's stalled-request
//!   budget, [`mise_stalled_request`], plus the memory package's backstop for
//!   one Kuru lifecycle that creates its store.
//!
//! Trade at the job deadline: a launch bounded by [`until_job_deadline`] that
//! stalls ends at the same deadline the coverage orchestrator enforces on the
//! test process. Both convert it from whole seconds against the truncated
//! current second, each ending up to a second after it, so which fires first
//! is not determined, and the launch's own stall diagnostics are not assured.
//! When the orchestrator fires first it terminates the test process tree; its
//! stall report names the unfinished test and carries that test's output so
//! far, but its process sample covers only the test process root (Unix root
//! state, Windows CPU times and working set), not the wrapper subtree that
//! `bounded_output` would have described. The stall also takes the partition's
//! remaining time, so that attempt leaves the rest of the binary's tests
//! unrun. Both follow from having no launch budget to take a shorter bound
//! from; a recorded wrapper budget would replace them.
//!
//! Follow-on: the `install` and `verify-staged` jobs, and the release
//! workflow's ordinary `tests` job, record no job start. Recording
//! `KURU_COVERAGE_JOB_STARTED` and `KURU_COVERAGE_JOB_MINUTES` on the steps
//! that run these binaries would let the previous updater's launches with no
//! product budget (`--version`, completions, man-page generation and the Unix
//! update) and the release job's wrapper launches take [`until_job_deadline`]
//! instead of the handoff budget or the local window.

#![allow(dead_code)] // Each includer uses a subset.

use kuru_delivery::update_budget::{CLEANUP, PUBLICATION, STARTUP};
use std::{sync::LazyLock, time::Duration};

/// The Windows updating parent's waits on its trusted helper, in series, each
/// with the wait it bounds as it reads in `src/update.rs` with whitespace
/// removed (the pin test requires each there exactly once). On a failed
/// handoff the parent has waited for the connection, the request and the
/// publication acknowledgment (its frame deadline is clamped inside it), and
/// the pipe close; it then waits for the helper's exit, and after an observed
/// exit drains the helper's diagnostics to EOF before stopping the drain, whose
/// pipe close cancels the in-flight read under its own bound. A successful
/// handoff skips the exit wait and the drain. Verification before the handoff
/// and the synchronous check of the acknowledged image have no bound of their
/// own. The series and its constants are unchanged since v0.9.0.
pub const UPDATE_HANDOFF_WAITS: [(&str, Duration); 7] = [
    ("listener.accept(&child,STARTUP)", STARTUP),
    (
        "tokio::time::timeout(STARTUP,async{pipe.write_all(",
        STARTUP,
    ),
    (
        "receive_publication(&mutpipe,PUBLICATION,STARTUP)",
        PUBLICATION,
    ),
    ("pipe.close(STARTUP).await?;", STARTUP),
    (
        "Some(child.wait(STARTUP+CLEANUP).await)",
        STARTUP.saturating_add(CLEANUP),
    ),
    ("tokio::time::timeout(CLEANUP,&muttask)", CLEANUP),
    ("letclosed=pipe.close(CLEANUP).await;", CLEANUP),
];

/// The longest the Windows updating parent waits on its trusted helper: the
/// sum of [`UPDATE_HANDOFF_WAITS`].
pub const fn update_handoff() -> Duration {
    let mut total = Duration::ZERO;
    let mut index = 0;
    while index < UPDATE_HANDOFF_WAITS.len() {
        total = total.saturating_add(UPDATE_HANDOFF_WAITS[index].1);
        index += 1;
    }
    total
}

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
