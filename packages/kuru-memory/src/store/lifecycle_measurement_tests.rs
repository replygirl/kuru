//! Lifecycle ordering measurements (not assertions).
//!
//! These tests are ignored by default: they measure how long the Dolt server
//! keeps counting a candidate branch's sessions after the product retires that
//! branch's pool, and whether the next unguarded checked branch operation would
//! meet Dolt's branch-in-use refusal. Run them explicitly, together, at the
//! package's two test threads:
//!
//! ```text
//! KURU_TEST_LIFECYCLE_MEASURE_DIR=<dir> KURU_TEST_LIFECYCLE_MEASURE_ITERATIONS=<n> \
//!   cargo test -p kuru-memory --lib --all-features lifecycle_measurement_tests \
//!   -- --ignored --test-threads=2
//! ```
//!
//! Each iteration follows the product's promoted-candidate cleanup exactly up
//! to its ungated delete: create a candidate, write to it, rename it to its
//! promoting status through `transition_candidate` (which already waits for the
//! source branch's sessions), then run `candidate_branch_is_clean` on the status
//! branch. That opens a fresh branch pool, queries it and retires it, which is
//! the step immediately before `delete_candidate_ref` in both cleanup paths.
//! Time zero is its return. Each iteration then ends with the product's own
//! `Candidate::abandon`, whose cleanup runs the same shape once more.
//!
//! Rows are written as CSV beneath the measurement directory; nothing is
//! printed, so the tests are safe beside PTY fixtures.

use super::*;
use serde_json::json;
use std::{fmt::Write as _, io::Write as _, path::PathBuf};

const MEASURE_DIR_ENV: &str = "KURU_TEST_LIFECYCLE_MEASURE_DIR";
const ITERATIONS_ENV: &str = "KURU_TEST_LIFECYCLE_MEASURE_ITERATIONS";
const DEFAULT_ITERATIONS: usize = 300;
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const POLL_LIMIT: Duration = Duration::from_secs(2);
const BRANCH_IN_USE: &str =
    "unsafe to delete or rename branches in use in other sessions; use --force to force the change";

fn measure_dir() -> Result<PathBuf> {
    let directory = std::env::var_os(MEASURE_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || std::env::temp_dir().join("kuru-lifecycle-measurements/m1"),
            PathBuf::from,
        );
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn iterations() -> Result<usize> {
    std::env::var(ITERATIONS_ENV).map_or(Ok(DEFAULT_ITERATIONS), |value| {
        value
            .parse()
            .context("KURU_TEST_LIFECYCLE_MEASURE_ITERATIONS must be a count")
    })
}

/// The product's own predicate (`await_branch_sessions_end`).
async fn branch_sessions(store: &MemoryStore, branch: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.processlist WHERE BINARY DB = BINARY ?",
    )
    .bind(format!("kuru/{branch}"))
    .fetch_one(store.pool.as_ref())
    .await?)
}

struct Linger {
    first: i64,
    /// Elapsed since time zero at the completion of the first poll that
    /// observed no session; `None` if sessions outlived the poll limit.
    zero_after: Option<Duration>,
    polls: usize,
}

async fn linger(store: &MemoryStore, branch: &str, zero: Instant) -> Result<Linger> {
    let mut first = None;
    let mut polls = 0;
    loop {
        let active = branch_sessions(store, branch).await?;
        polls += 1;
        first.get_or_insert(active);
        if active == 0 {
            return Ok(Linger {
                first: first.unwrap_or_default(),
                zero_after: Some(zero.elapsed()),
                polls,
            });
        }
        if zero.elapsed() >= POLL_LIMIT {
            return Ok(Linger {
                first: first.unwrap_or_default(),
                zero_after: None,
                polls,
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn sql_class(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(sqlx::Error::Database(database)) = cause.downcast_ref::<sqlx::Error>()
            && let Some(mysql) = database.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
        {
            let message = database.message();
            let reason = if message == BRANCH_IN_USE {
                "branch_in_use"
            } else if message.contains("not fully merged") {
                "not_merged"
            } else {
                "other"
            };
            return format!(
                "{}/{}/{reason}",
                mysql.code().unwrap_or("none"),
                mysql.number()
            );
        }
    }
    "non_sql".into()
}

/// Checked (non-force) delete, exactly as `delete_candidate_ref` issues it.
/// Dolt checks active sessions before merge state, so an unmerged status
/// branch reports either branch-in-use or not-merged and is never removed.
async fn checked_delete(connection: &mut sqlx::MySqlConnection, branch: &str) -> String {
    let result = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query("CALL DOLT_BRANCH(?, ?)")
            .bind("-d")
            .bind(branch)
            .fetch_all(connection),
    )
    .await;
    match result {
        Err(_) => "deadline".into(),
        Ok(Ok(_)) => "deleted".into(),
        Ok(Err(error)) => sql_class(&anyhow::Error::from(error)),
    }
}

fn outcome(result: &Result<()>) -> String {
    match result {
        Ok(()) => "ok".into(),
        Err(error) => candidate_failure_record(error).map_or_else(
            || {
                format!("{error:#}")
                    .chars()
                    .map(|c| if c == ',' || c == '\n' { ';' } else { c })
                    .take(160)
                    .collect()
            },
            |record| format!("{record} sql={}", sql_class(error)),
        ),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Variant {
    /// Poll the server's session set from time zero.
    Linger,
    /// Issue the checked delete at time zero on a pre-acquired connection.
    DeleteTight,
    /// Issue it after `delete_candidate_ref`'s own preceding steps.
    DeleteProduct,
}

impl Variant {
    const fn label(self) -> &'static str {
        match self {
            Self::Linger => "linger",
            Self::DeleteTight => "delete_tight",
            Self::DeleteProduct => "delete_product",
        }
    }
}

struct Row {
    variant: Variant,
    iteration: usize,
    inspection: String,
    probe: String,
    linger: Option<Linger>,
    abandon: String,
}

async fn iteration(store: &MemoryStore, variant: Variant, index: usize) -> Result<Row> {
    let candidate = store.begin_candidate("lifecycle measurement").await?;
    candidate.view().put("measurement", &json!(index)).await?;
    let names = CandidateNames::from_open(&candidate.view.branch)?;
    let target = candidate.view().revision().await?;
    transition_candidate(store, &names.open, &names.promoting, &target).await?;
    let mut held = if variant == Variant::DeleteTight {
        Some(owned_connection(&store.pool).await?.0)
    } else {
        None
    };
    let inspection = candidate_branch_is_clean(store, &names.promoting).await;
    let zero = Instant::now();
    let (probe, linger) = match variant {
        Variant::Linger => (
            "none".to_owned(),
            Some(linger(store, &names.promoting, zero).await?),
        ),
        Variant::DeleteTight => {
            let connection = held.as_mut().expect("pre-acquired connection");
            let probe = checked_delete(connection, &names.promoting).await;
            drop(held);
            (probe, Some(linger(store, &names.promoting, zero).await?))
        }
        Variant::DeleteProduct => {
            candidate_heads(&store.pool, &names).await?;
            let (mut connection, _) = owned_connection(&store.pool).await?;
            let probe = checked_delete(&mut connection, &names.promoting).await;
            drop(connection);
            (probe, Some(linger(store, &names.promoting, zero).await?))
        }
    };
    let inspection = match inspection {
        Ok(clean) => format!("clean={clean}"),
        Err(error) => outcome(&Err(error)),
    };
    let abandon = outcome(&candidate.abandon().await);
    Ok(Row {
        variant,
        iteration: index,
        inspection,
        probe,
        linger,
        abandon,
    })
}

fn percentile(sorted: &[u128], fraction: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() as f64) * fraction).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn summary(rows: &[Row], errors: &[String]) -> String {
    let mut text = String::new();
    for variant in [
        Variant::Linger,
        Variant::DeleteTight,
        Variant::DeleteProduct,
    ] {
        let rows = rows
            .iter()
            .filter(|row| row.variant == variant)
            .collect::<Vec<_>>();
        if rows.is_empty() {
            continue;
        }
        let lingering = rows
            .iter()
            .filter_map(|row| row.linger.as_ref())
            .filter(|linger| linger.first > 0)
            .collect::<Vec<_>>();
        let mut micros = lingering
            .iter()
            .filter_map(|linger| linger.zero_after.map(|elapsed| elapsed.as_micros()))
            .collect::<Vec<_>>();
        micros.sort_unstable();
        let unresolved = lingering.iter().filter(|l| l.zero_after.is_none()).count();
        let in_use = rows
            .iter()
            .filter(|row| row.probe.ends_with("/branch_in_use"))
            .count();
        let abandon_failed = rows.iter().filter(|row| row.abandon != "ok").count();
        let mut probes = BTreeMap::<&str, usize>::new();
        for row in &rows {
            *probes.entry(row.probe.as_str()).or_default() += 1;
        }
        let _ = writeln!(
            text,
            "variant={} iterations={} linger_gt0={} linger_unresolved_after_2s={} \
             linger_us_p50={} p95={} max={} probe_branch_in_use={} abandon_failed={} probes={:?}",
            variant.label(),
            rows.len(),
            lingering.len(),
            unresolved,
            percentile(&micros, 0.50),
            percentile(&micros, 0.95),
            micros.last().copied().unwrap_or(0),
            in_use,
            abandon_failed,
            probes,
        );
        let mut histogram = BTreeMap::<u128, usize>::new();
        for value in &micros {
            // Power-of-two microsecond buckets.
            let bucket = value.next_power_of_two().max(1);
            *histogram.entry(bucket).or_default() += 1;
        }
        let _ = writeln!(text, "  linger_histogram_us_le={histogram:?}");
        for row in rows.iter().filter(|row| row.abandon != "ok") {
            let _ = writeln!(text, "  abandon_failure[{}]={}", row.iteration, row.abandon);
        }
    }
    for error in errors {
        let _ = writeln!(text, "iteration_error={error}");
    }
    text
}

async fn measure(variants: &[Variant], name: &str) -> Result<()> {
    let directory = measure_dir()?;
    let count = iterations()?;
    let stamp = crate::test_support::lifecycle_trace::nanos();
    let mut csv = std::fs::File::create(directory.join(format!("{name}-{stamp}.csv")))?;
    writeln!(
        csv,
        "variant,iteration,inspection,probe,first_sessions,zero_after_us,polls,abandon"
    )?;
    let mut store = MemoryStore::temporary().await?;
    let mut rows = Vec::with_capacity(count);
    let mut errors = Vec::new();
    for index in 0..count {
        let variant = variants[index % variants.len()];
        match iteration(&store, variant, index).await {
            Ok(row) => {
                let linger = row.linger.as_ref();
                writeln!(
                    csv,
                    "{},{},{},{},{},{},{},{}",
                    row.variant.label(),
                    row.iteration,
                    row.inspection,
                    row.probe,
                    linger.map_or(-1, |l| l.first),
                    linger
                        .and_then(|l| l.zero_after)
                        .map_or(-1, |d| i128::try_from(d.as_micros()).unwrap_or(-1)),
                    linger.map_or(0, |l| l.polls),
                    row.abandon,
                )?;
                rows.push(row);
            }
            Err(error) => {
                // A failed iteration may leave a fenced store; start afresh.
                errors.push(format!("{index}:{}", outcome(&Err(error))));
                let previous = std::mem::replace(&mut store, MemoryStore::temporary().await?);
                let _ = previous.close().await;
            }
        }
    }
    store.close().await?;
    std::fs::write(
        directory.join(format!("{name}-{stamp}-summary.txt")),
        summary(&rows, &errors),
    )?;
    Ok(())
}

#[tokio::test]
#[ignore = "measurement: run explicitly with --ignored (see module docs)"]
async fn measure_status_branch_session_linger_after_pool_retirement() -> Result<()> {
    measure(&[Variant::Linger], "linger").await
}

#[tokio::test]
#[ignore = "measurement: run explicitly with --ignored (see module docs)"]
async fn measure_checked_delete_immediately_after_pool_retirement() -> Result<()> {
    measure(&[Variant::DeleteTight, Variant::DeleteProduct], "delete").await
}

/// A server session on `branch` that the product's pool bookkeeping no longer
/// owns, standing in for one the server has not yet removed. It closes itself
/// after `linger`; `None` closes it before returning (the control).
async fn lingering_session(
    store: &MemoryStore,
    branch: &str,
    linger: Option<Duration>,
) -> Result<tokio::task::JoinHandle<Result<()>>> {
    let pool = store.shared.server.pool(branch).await?;
    let mut connection = pool.acquire().await?;
    sqlx::query("SELECT 1").execute(&mut *connection).await?;
    let raw = connection.detach();
    drop(pool);
    let Some(linger) = linger else {
        sqlx::Connection::close(raw).await?;
        await_branch_sessions_end(store, branch, QUERY_TIMEOUT).await?;
        return Ok(tokio::spawn(async { Ok(()) }));
    };
    Ok(tokio::spawn(async move {
        tokio::time::sleep(linger).await;
        sqlx::Connection::close(raw).await?;
        Ok(())
    }))
}

/// Deterministic consequence of one lingering session (not a rate): the same
/// session duration against the ungated checked delete in
/// `delete_candidate_ref` and against the gated `transition_candidate` rename.
#[tokio::test]
#[ignore = "measurement: run explicitly with --ignored (see module docs)"]
async fn measure_lingering_session_consequence_for_delete_and_rename() -> Result<()> {
    let directory = measure_dir()?;
    let stamp = crate::test_support::lifecycle_trace::nanos();
    let mut csv = std::fs::File::create(directory.join(format!("consequence-{stamp}.csv")))?;
    writeln!(
        csv,
        "linger_ms,iteration,delete,delete_ms,rename,rename_ms,abandon"
    )?;
    let store = MemoryStore::temporary().await?;
    let lingers = [None, Some(20), Some(100), Some(1000)];
    for index in 0..(lingers.len() * 5) {
        let linger = lingers[index % lingers.len()].map(Duration::from_millis);
        let candidate = store.begin_candidate("lifecycle consequence").await?;
        candidate.view().put("measurement", &json!(index)).await?;
        let names = CandidateNames::from_open(&candidate.view.branch)?;
        let target = candidate.view().revision().await?;
        transition_candidate(&store, &names.open, &names.promoting, &target).await?;

        let session = lingering_session(&store, &names.promoting, linger).await?;
        let started = Instant::now();
        let delete = delete_candidate_ref(&store, &names.promoting, &target, false, QUERY_TIMEOUT)
            .await
            .context(CandidateFailureStage::Cleanup);
        let delete_ms = started.elapsed().as_millis();
        session.await??;
        await_branch_sessions_end(&store, &names.promoting, QUERY_TIMEOUT).await?;

        let session = lingering_session(&store, &names.promoting, linger).await?;
        let started = Instant::now();
        let rename =
            transition_candidate(&store, &names.promoting, &names.abandoned, &target).await;
        let rename_ms = started.elapsed().as_millis();
        session.await??;

        let abandon = outcome(&candidate.abandon().await);
        writeln!(
            csv,
            "{},{index},{},{delete_ms},{},{rename_ms},{abandon}",
            linger.map_or(-1, |value| i64::try_from(value.as_millis()).unwrap_or(-1)),
            outcome(&delete),
            outcome(&rename),
        )?;
    }
    store.close().await
}
