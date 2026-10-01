//! Opt-in owner open timeline, a release-binary measurement aid.
//!
//! Only a memory service owner whose environment holds
//! `KURU_OPEN_TIMELINE=1` (exactly `1`) installs a timeline, as the first
//! statement of [`crate::service::service_entry`]. Every other process (the
//! client, the env-cleared supervisor, test runners, direct opens) and every
//! ungated owner takes one untaken branch at each stamp site. The owner
//! stamps named milestones as nanosecond offsets from one monotonic anchor
//! into a bounded log, sealed at endpoint publication, and writes it once,
//! after its close has released the owner lock, as a non-durable
//! owner-private file named for its service generation. The record carries
//! only static event names, offsets, one wall-clock anchor, the service
//! generation, the package version and counts: never a path, scope, SQL,
//! identity, credential or content. Recording and writing never fail or
//! delay the open, serve or close. Windows owners are spawned with an
//! explicit environment that does not carry the variable, so the timeline is
//! inert there and no support is claimed.

use std::{
    ffi::OsStr,
    io::Write as _,
    path::Path,
    sync::{Mutex, OnceLock, PoisonError},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{NameRetention, Privacy};
use serde::Serialize;

/// The variable that gates the timeline in the owner process.
pub(crate) const ENV: &str = "KURU_OPEN_TIMELINE";
const CAPACITY: usize = 64;
const MAX_BYTES: usize = 8 * 1024;
const FORMAT: &str = "kuru.open-timeline";
const FORMAT_VERSION: u32 = 1;
const FILE_PREFIX: &str = "open-timeline-";

static INSTALLED: OnceLock<Timeline> = OnceLock::new();

/// Open milestones in canonical order. Each has exactly one stamp site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    OwnerMain,
    OwnerLock,
    CacheVerifyStart,
    CacheVerifyEnd,
    SupervisorReady,
    ProbeVerified,
    MainPool,
    VersionRead,
    ValidateActive,
    CandidateRecovery,
    UsagePool,
    UsageScan1,
    UsageUpgrade,
    UsageValidate,
    UsageScan2,
    StoreReady,
    ListenerBound,
    EndpointPublished,
}

impl Event {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 18] = [
        Self::OwnerMain,
        Self::OwnerLock,
        Self::CacheVerifyStart,
        Self::CacheVerifyEnd,
        Self::SupervisorReady,
        Self::ProbeVerified,
        Self::MainPool,
        Self::VersionRead,
        Self::ValidateActive,
        Self::CandidateRecovery,
        Self::UsagePool,
        Self::UsageScan1,
        Self::UsageUpgrade,
        Self::UsageValidate,
        Self::UsageScan2,
        Self::StoreReady,
        Self::ListenerBound,
        Self::EndpointPublished,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::OwnerMain => "owner-main",
            Self::OwnerLock => "owner-lock",
            Self::CacheVerifyStart => "cache-verify-start",
            Self::CacheVerifyEnd => "cache-verify-end",
            Self::SupervisorReady => "supervisor-ready",
            Self::ProbeVerified => "probe-verified",
            Self::MainPool => "main-pool",
            Self::VersionRead => "version-read",
            Self::ValidateActive => "validate-active",
            Self::CandidateRecovery => "candidate-recovery",
            Self::UsagePool => "usage-pool",
            Self::UsageScan1 => "usage-scan-1",
            Self::UsageUpgrade => "usage-upgrade",
            Self::UsageValidate => "usage-validate",
            Self::UsageScan2 => "usage-scan-2",
            Self::StoreReady => "store-ready",
            Self::ListenerBound => "listener-bound",
            Self::EndpointPublished => "endpoint-published",
        }
    }
}

/// True only for exactly `1`.
fn enabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// Read the gate once and, when it is set, install this process's timeline
/// and stamp its anchor. Called only by the owner's service entry.
pub(crate) fn install_from_env() {
    if enabled(std::env::var_os(ENV).as_deref()) {
        INSTALLED.get_or_init(Timeline::new).stamp(Event::OwnerMain);
    }
}

/// Stamp `event` when this process installed a timeline; otherwise nothing.
pub(crate) fn stamp(event: Event) {
    if let Some(timeline) = INSTALLED.get() {
        timeline.stamp(event);
    }
}

/// Record the first usage-ledger scan's row count (name and number only).
pub(crate) fn usage_rows(rows: u64) {
    if let Some(timeline) = INSTALLED.get() {
        timeline.record_usage_rows(rows);
    }
}

/// This process's timeline, when the gate installed one.
pub(crate) fn installed() -> Option<&'static Timeline> {
    INSTALLED.get()
}

pub(crate) struct Timeline {
    anchor: Instant,
    anchor_unix_ns: u64,
    log: Mutex<Log>,
}

struct Log {
    entries: Vec<(Event, u64)>,
    limit: usize,
    usage_rows: Option<u64>,
    dropped: u32,
    late: u32,
    sealed: bool,
}

impl Timeline {
    pub(crate) fn new() -> Self {
        Self::with_limit(CAPACITY)
    }

    #[cfg(test)]
    pub(crate) fn with_capacity(limit: usize) -> Self {
        Self::with_limit(limit)
    }

    fn with_limit(limit: usize) -> Self {
        let anchor = Instant::now();
        let anchor_unix_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
            });
        Self {
            anchor,
            anchor_unix_ns,
            log: Mutex::new(Log {
                entries: Vec::with_capacity(limit),
                limit,
                usage_rows: None,
                dropped: 0,
                late: 0,
                sealed: false,
            }),
        }
    }

    /// Append `event` at the current offset. The clock is read while the log
    /// is held, so stored offsets never decrease. A full log counts `dropped`
    /// and never grows; a sealed one counts `late`.
    pub(crate) fn stamp(&self, event: Event) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        let ns = u64::try_from(self.anchor.elapsed().as_nanos()).unwrap_or(u64::MAX);
        if log.sealed {
            log.late = log.late.saturating_add(1);
            return;
        }
        if log.entries.len() < log.limit {
            log.entries.push((event, ns));
        } else {
            log.dropped = log.dropped.saturating_add(1);
        }
        if event == Event::EndpointPublished {
            log.sealed = true;
        }
    }

    /// The first count wins.
    fn record_usage_rows(&self, rows: u64) {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.usage_rows.get_or_insert(rows);
    }

    fn encode(&self, generation: &str) -> Result<Vec<u8>> {
        let (events, usage_rows, dropped, late) = {
            let log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
            let events = log
                .entries
                .iter()
                .map(|&(event, ns)| Stamp {
                    event: event.name(),
                    ns,
                })
                .collect::<Vec<_>>();
            (events, log.usage_rows, log.dropped, log.late)
        };
        let bytes = serde_json::to_vec(&Record {
            format: FORMAT,
            format_version: FORMAT_VERSION,
            kuru_version: env!("CARGO_PKG_VERSION"),
            service_generation: generation,
            anchor_unix_ns: self.anchor_unix_ns,
            events,
            counts: Counts { usage_rows },
            dropped,
            late,
        })?;
        ensure!(bytes.len() <= MAX_BYTES, "open timeline exceeds its size");
        Ok(bytes)
    }
}

#[derive(Serialize)]
struct Record<'a> {
    format: &'static str,
    format_version: u32,
    kuru_version: &'static str,
    service_generation: &'a str,
    anchor_unix_ns: u64,
    events: Vec<Stamp>,
    counts: Counts,
    dropped: u32,
    late: u32,
}

#[derive(Serialize)]
struct Stamp {
    event: &'static str,
    ns: u64,
}

#[derive(Serialize)]
struct Counts {
    usage_rows: Option<u64>,
}

/// The file name for one service generation.
pub(crate) fn file_name(generation: &str) -> String {
    format!("{FILE_PREFIX}{generation}.json")
}

/// True for a name this module writes.
#[cfg(test)]
pub(crate) fn is_timeline_name(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(FILE_PREFIX) && name.ends_with(".json"))
}

/// Write `timeline` once into the existing owner-private `directory`,
/// through the checked handle: `O_CREAT | O_EXCL`, owner-only, never
/// following a link, replacing a file or creating a directory. Deliberately
/// not durable: no sync and no rename, since a lost diagnostic costs nothing
/// and an fsync would lengthen every gated owner's exit.
pub(crate) fn write(timeline: &Timeline, directory: &Path, generation: &str) -> Result<()> {
    uuid::Uuid::parse_str(generation).context("open timeline generation is not a UUID")?;
    let bytes = timeline.encode(generation)?;
    let parent =
        crate::files::open_directory(directory, Privacy::OwnerOnly, NameRetention::Movable)?;
    let mut file = parent.create_new(OsStr::new(&file_name(generation)))?;
    file.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, PoisonError};

    fn snapshot(timeline: &Timeline) -> (Vec<(Event, u64)>, usize, u32, u32, Option<u64>) {
        let log = timeline.log.lock().unwrap_or_else(PoisonError::into_inner);
        (
            log.entries.clone(),
            log.entries.capacity(),
            log.dropped,
            log.late,
            log.usage_rows,
        )
    }

    fn parsed(bytes: &[u8]) -> serde_json::Value {
        serde_json::from_slice(bytes).expect("an encoded timeline is JSON")
    }

    // Test 1: non-decreasing and bounded across threads and tasks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_stamps_stay_ordered_and_never_grow_the_log() {
        let timeline = Arc::new(Timeline::with_capacity(20_000));
        let capacity = snapshot(&timeline).1;
        let stamp_n = |timeline: Arc<Timeline>, count: usize| {
            move || {
                for _ in 0..count {
                    timeline.stamp(Event::MainPool);
                }
            }
        };
        let threads = (0..4)
            .map(|_| std::thread::spawn(stamp_n(timeline.clone(), 1_100)))
            .collect::<Vec<_>>();
        let tasks = (0..4)
            .map(|_| {
                let stamping = stamp_n(timeline.clone(), 1_100);
                tokio::spawn(async move { stamping() })
            })
            .collect::<Vec<_>>();
        let blocking = tokio::task::spawn_blocking(stamp_n(timeline.clone(), 1_200));
        for thread in threads {
            thread.join().expect("stamping thread");
        }
        for task in tasks {
            task.await.expect("stamping task");
        }
        blocking.await.expect("blocking stamper");
        let (entries, after, dropped, late, _) = snapshot(&timeline);
        assert_eq!(entries.len() + dropped as usize, 10_000);
        assert_eq!(late, 0);
        assert_eq!(after, capacity, "the log reallocated");
        assert!(
            entries.windows(2).all(|pair| pair[0].1 <= pair[1].1),
            "stored offsets decreased"
        );
    }

    #[test]
    fn a_full_log_counts_drops_without_growing() {
        let timeline = Timeline::new();
        let capacity = snapshot(&timeline).1;
        for _ in 0..200 {
            timeline.stamp(Event::VersionRead);
        }
        let (entries, after, dropped, late, _) = snapshot(&timeline);
        assert_eq!(entries.len(), CAPACITY);
        assert_eq!(dropped, 136);
        assert_eq!(late, 0);
        assert_eq!(after, capacity);
    }

    // Test 2: endpoint publication seals the log.
    #[test]
    fn stamps_after_endpoint_publication_are_counted_late() {
        let timeline = Timeline::new();
        timeline.stamp(Event::OwnerMain);
        timeline.stamp(Event::EndpointPublished);
        timeline.stamp(Event::SupervisorReady);
        timeline.stamp(Event::EndpointPublished);
        let (entries, _, dropped, late, _) = snapshot(&timeline);
        assert_eq!(
            entries.iter().map(|&(event, _)| event).collect::<Vec<_>>(),
            [Event::OwnerMain, Event::EndpointPublished]
        );
        assert_eq!((dropped, late), (0, 2));
    }

    // Test 3: only exactly `1` enables the timeline.
    #[test]
    fn only_exactly_one_enables_the_timeline() {
        assert!(enabled(Some(OsStr::new("1"))));
        for value in ["", "0", "true", " 1", "1 ", "11", "yes"] {
            assert!(!enabled(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(!enabled(None));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            assert!(!enabled(Some(OsStr::from_bytes(b"1\xff"))));
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt as _;
            let value = std::ffi::OsString::from_wide(&[u16::from(b'1'), 0xd800]);
            assert!(!enabled(Some(&value)));
        }
    }

    #[test]
    fn the_first_usage_row_count_wins() {
        let timeline = Timeline::new();
        assert_eq!(snapshot(&timeline).4, None);
        timeline.record_usage_rows(7);
        timeline.record_usage_rows(9);
        assert_eq!(snapshot(&timeline).4, Some(7));
    }

    // Test 4: the record's schema, names and size.
    #[test]
    fn the_record_has_the_specified_fields_and_names() {
        let generation = uuid::Uuid::new_v4().to_string();
        let timeline = Timeline::new();
        for event in Event::ALL {
            timeline.stamp(event);
        }
        let value = parsed(&timeline.encode(&generation).unwrap());
        let object = value.as_object().unwrap();
        let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "anchor_unix_ns",
                "counts",
                "dropped",
                "events",
                "format",
                "format_version",
                "kuru_version",
                "late",
                "service_generation",
            ]
        );
        assert_eq!(value["format"], "kuru.open-timeline");
        assert_eq!(value["format_version"], 1);
        assert_eq!(value["kuru_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["service_generation"], generation.as_str());
        assert!(value["anchor_unix_ns"].is_u64());
        assert!(value["counts"]["usage_rows"].is_null());
        assert_eq!(value["counts"].as_object().unwrap().len(), 1);
        assert_eq!(
            (value["dropped"].as_u64(), value["late"].as_u64()),
            (Some(0), Some(0))
        );
        let events = value["events"].as_array().unwrap();
        let names = events
            .iter()
            .map(|entry| entry["event"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "owner-main",
                "owner-lock",
                "cache-verify-start",
                "cache-verify-end",
                "supervisor-ready",
                "probe-verified",
                "main-pool",
                "version-read",
                "validate-active",
                "candidate-recovery",
                "usage-pool",
                "usage-scan-1",
                "usage-upgrade",
                "usage-validate",
                "usage-scan-2",
                "store-ready",
                "listener-bound",
                "endpoint-published",
            ]
        );
        for entry in events {
            assert_eq!(entry.as_object().unwrap().len(), 2);
            assert!(entry["ns"].is_u64(), "{entry}");
        }
        timeline.record_usage_rows(42);
        assert_eq!(
            parsed(&timeline.encode(&generation).unwrap())["counts"]["usage_rows"],
            42
        );
    }

    #[test]
    fn a_full_log_encodes_within_its_bound() {
        let timeline = Timeline::new();
        for _ in 0..(CAPACITY + 10) {
            // The longest name, at the widest offset.
            timeline.stamp(Event::CandidateRecovery);
        }
        {
            let mut log = timeline.log.lock().unwrap();
            for entry in &mut log.entries {
                entry.1 = u64::MAX;
            }
            log.usage_rows = Some(u64::MAX);
            log.dropped = u32::MAX;
            log.late = u32::MAX;
        }
        let bytes = timeline.encode(&uuid::Uuid::new_v4().to_string()).unwrap();
        assert!(bytes.len() <= MAX_BYTES, "{} bytes", bytes.len());
        assert_eq!(parsed(&bytes)["events"].as_array().unwrap().len(), CAPACITY);
    }

    #[test]
    fn an_oversized_record_is_refused() {
        let timeline = Timeline::with_capacity(400);
        for _ in 0..400 {
            timeline.stamp(Event::CandidateRecovery);
        }
        assert!(timeline.encode(&uuid::Uuid::new_v4().to_string()).is_err());
    }

    // Test 5: the checked, owner-only, create-only write.
    #[test]
    fn the_write_creates_one_private_file_and_never_replaces_one() {
        let root = tempfile::tempdir().unwrap();
        let services = root.path().join("services");
        crate::files::ensure_private_directory(&services).unwrap();
        let generation = uuid::Uuid::new_v4().to_string();
        let timeline = Timeline::new();
        timeline.stamp(Event::OwnerMain);
        write(&timeline, &services, &generation).unwrap();
        let path = services.join(file_name(&generation));
        assert!(is_timeline_name(path.file_name().unwrap()));
        let first = std::fs::read(&path).unwrap();
        assert_eq!(first, timeline.encode(&generation).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        timeline.stamp(Event::OwnerLock);
        assert!(write(&timeline, &services, &generation).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), first);
    }

    #[test]
    fn the_write_fails_without_its_directory_or_with_its_name_taken() {
        let root = tempfile::tempdir().unwrap();
        let services = root.path().join("services");
        let generation = uuid::Uuid::new_v4().to_string();
        let timeline = Timeline::new();
        assert!(write(&timeline, &services, &generation).is_err());
        assert!(!services.exists(), "the write created its directory");
        crate::files::ensure_private_directory(&services).unwrap();
        let taken = services.join(file_name(&generation));
        std::fs::create_dir(&taken).unwrap();
        assert!(write(&timeline, &services, &generation).is_err());
        assert!(taken.is_dir());
        assert!(std::fs::read_dir(&taken).unwrap().next().is_none());
        for generation in ["../escape", "not-a-uuid", ""] {
            assert!(
                write(&timeline, &services, generation).is_err(),
                "{generation:?}"
            );
        }
        assert!(!is_timeline_name(OsStr::new("endpoint.json")));
    }

    #[test]
    fn an_uninstalled_process_records_nothing() {
        // The runner never calls `install_from_env`, so every site is inert.
        assert!(installed().is_none());
        stamp(Event::OwnerMain);
        usage_rows(1);
        assert!(installed().is_none());
    }
}
