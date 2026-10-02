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
//! identity, credential or content. Recording and writing never fail,
//! cancel or reorder the open, serve or close.
//!
//! A gated owner started with a starter token also streams each stamp, as
//! it is taken, to a create-only owner-private file named from that token's
//! activity tag: one unsynced line of at most 62 bytes per stamp, carrying
//! only the event name, its offset and its wall-clock time. The owner removes
//! that file under owner authority right after endpoint publication, before
//! it serves, or when its open fails; it never appends to or removes a file
//! it did not create. A gated starter reads only its own owner's stream,
//! once, at its readiness deadline. A Windows starter forwards the gate to
//! its owner only when it is exactly `1`.

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Write as _,
    path::Path,
    sync::{Mutex, OnceLock, PoisonError},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use serde::Serialize;

/// The variable that gates the timeline in the owner process, and the
/// deadline read in a starter.
pub(crate) const ENV: &str = "KURU_OPEN_TIMELINE";
const CAPACITY: usize = 64;
/// The bound of the close-time record and of a starter's stream read.
pub(crate) const MAX_BYTES: usize = 8 * 1024;
/// The longest stream line: the longest event name and two `u64::MAX`
/// integers, three separators included.
#[cfg(test)]
const MAX_LINE_BYTES: usize = 62;
const FORMAT: &str = "kuru.open-timeline";
const FORMAT_VERSION: u32 = 1;
const FILE_PREFIX: &str = "open-timeline-";
/// Deliberately not [`FILE_PREFIX`]: nothing that lists close-time records
/// ever sees a stream.
const STREAM_PREFIX: &str = "open-stream-";

static INSTALLED: OnceLock<Timeline> = OnceLock::new();

/// Open milestones in canonical order. Each has exactly one stamp site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    OwnerMain,
    OwnerLock,
    StartupLock,
    ExtractStart,
    ExtractEnd,
    CacheVerifyStart,
    CacheVerifyEnd,
    CreateStart,
    SupervisorSpawned,
    /// Stamped only by a Windows supervisor start.
    #[cfg_attr(not(windows), allow(dead_code))]
    ChannelAccepted,
    SupervisorReady,
    ProbeVerified,
    TemplateCopied,
    ColdCreated,
    Activated,
    MainPool,
    VersionRead,
    MigrateStart,
    MigrateEnd,
    ValidateActive,
    CandidateRecovery,
    UsagePool,
    UsageBound,
    UsageScan1,
    UsageUpgrade,
    UsageValidate,
    UsageScan2,
    UsageRecord,
    StoreReady,
    ListenerBound,
    EndpointPublished,
}

impl Event {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 31] = [
        Self::OwnerMain,
        Self::OwnerLock,
        Self::StartupLock,
        Self::ExtractStart,
        Self::ExtractEnd,
        Self::CacheVerifyStart,
        Self::CacheVerifyEnd,
        Self::CreateStart,
        Self::SupervisorSpawned,
        Self::ChannelAccepted,
        Self::SupervisorReady,
        Self::ProbeVerified,
        Self::TemplateCopied,
        Self::ColdCreated,
        Self::Activated,
        Self::MainPool,
        Self::VersionRead,
        Self::MigrateStart,
        Self::MigrateEnd,
        Self::ValidateActive,
        Self::CandidateRecovery,
        Self::UsagePool,
        Self::UsageBound,
        Self::UsageScan1,
        Self::UsageUpgrade,
        Self::UsageValidate,
        Self::UsageScan2,
        Self::UsageRecord,
        Self::StoreReady,
        Self::ListenerBound,
        Self::EndpointPublished,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::OwnerMain => "owner-main",
            Self::OwnerLock => "owner-lock",
            Self::StartupLock => "startup-lock",
            Self::ExtractStart => "extract-start",
            Self::ExtractEnd => "extract-end",
            Self::CacheVerifyStart => "cache-verify-start",
            Self::CacheVerifyEnd => "cache-verify-end",
            Self::CreateStart => "create-start",
            Self::SupervisorSpawned => "supervisor-spawned",
            Self::ChannelAccepted => "channel-accepted",
            Self::SupervisorReady => "supervisor-ready",
            Self::ProbeVerified => "probe-verified",
            Self::TemplateCopied => "template-copied",
            Self::ColdCreated => "cold-created",
            Self::Activated => "activated",
            Self::MainPool => "main-pool",
            Self::VersionRead => "version-read",
            Self::MigrateStart => "migrate-start",
            Self::MigrateEnd => "migrate-end",
            Self::ValidateActive => "validate-active",
            Self::CandidateRecovery => "candidate-recovery",
            Self::UsagePool => "usage-pool",
            Self::UsageBound => "usage-bound",
            Self::UsageScan1 => "usage-scan-1",
            Self::UsageUpgrade => "usage-upgrade",
            Self::UsageValidate => "usage-validate",
            Self::UsageScan2 => "usage-scan-2",
            Self::UsageRecord => "usage-record",
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

/// True when this process's own environment holds the gate exactly; a
/// starter reads its owner's stream, and a Windows starter forwards the gate,
/// only then. A test scopes its own value instead, never the runner's
/// environment.
pub(crate) fn gate_set() -> bool {
    #[cfg(test)]
    if let Ok(gated) = GATE_OVERRIDE.try_with(|gated| *gated) {
        return gated;
    }
    enabled(std::env::var_os(ENV).as_deref())
}

/// Stamp `event` when this process installed a timeline; otherwise nothing.
pub(crate) fn stamp(event: Event) {
    if let Some(timeline) = INSTALLED.get() {
        timeline.stamp(event);
        // After the stamp released its log, so a held owner's other tasks
        // can still stamp.
        #[cfg(all(unix, any(test, feature = "test-support")))]
        hold::at(event);
    }
}

/// Start streaming this process's timeline into `directory`, when it
/// installed one; otherwise nothing. See [`Timeline::stream_to`].
pub(crate) fn stream_to(directory: &Path, tag: &str) {
    if let Some(timeline) = INSTALLED.get() {
        timeline.stream_to(directory, tag);
    }
}

/// Remove this process's stream, when it has one. See
/// [`Timeline::end_stream`].
pub(crate) fn end_stream() {
    if let Some(timeline) = INSTALLED.get() {
        timeline.end_stream();
    }
}

/// The stream file name for one starter token's activity tag.
pub(crate) fn stream_name(tag: &str) -> String {
    format!("{STREAM_PREFIX}{tag}")
}

/// One stream line: the event name, its offset and its wall-clock time.
fn stream_line(event: Event, ns: u64, unix_ns: u64) -> String {
    format!("{} {ns} {unix_ns}\n", event.name())
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

#[cfg(test)]
tokio::task_local! {
    /// One test's own starter-side gate, so an in-process starter is gated
    /// or ungated without touching the runner's environment.
    static GATE_OVERRIDE: bool;
}

/// Run `f` with this test's starter-side gate set to `gated`.
#[cfg(test)]
pub(crate) fn with_gate_sync<R>(gated: bool, f: impl FnOnce() -> R) -> R {
    GATE_OVERRIDE.sync_scope(gated, f)
}

/// Run `future` with this test's starter-side gate set to `gated`.
#[cfg(all(test, unix))]
pub(crate) async fn with_gate<F: std::future::Future>(gated: bool, future: F) -> F::Output {
    GATE_OVERRIDE.scope(gated, future).await
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
    stream: Option<Stream>,
}

/// The owner's own stream file, held with its checked directory so that
/// only this file is ever removed from its name.
struct Stream {
    directory: Directory,
    name: OsString,
    file: File,
    /// A failed write stops further writes; the file is still removed.
    failed: bool,
}

impl Stream {
    fn append(&mut self, event: Event, ns: u64, anchor_unix_ns: u64) {
        if self.failed {
            return;
        }
        let line = stream_line(event, ns, anchor_unix_ns.saturating_add(ns));
        self.failed = self.file.write_all(line.as_bytes()).is_err();
    }
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
                stream: None,
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
            let anchor_unix_ns = self.anchor_unix_ns;
            if let Some(stream) = &mut log.stream {
                stream.append(event, ns, anchor_unix_ns);
            }
        } else {
            log.dropped = log.dropped.saturating_add(1);
        }
        if event == Event::EndpointPublished {
            log.sealed = true;
        }
    }

    /// Create this owner's stream as `open-stream-<tag>` in the owner-private
    /// `directory` and write every entry logged so far; each later logged
    /// stamp appends its line. Create-only: a name already taken, by a
    /// predecessor with the same tag or anything else, leaves streaming off
    /// and that object untouched. Unsynced, and best effort: no failure here
    /// changes the open. Call once, while holding owner authority.
    pub(crate) fn stream_to(&self, directory: &Path, tag: &str) {
        let _ = self.try_stream_to(directory, tag);
    }

    fn try_stream_to(&self, directory: &Path, tag: &str) -> Result<()> {
        ensure!(
            !tag.is_empty() && tag.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "open timeline stream tag is not hex"
        );
        // Held across creation, so no stamp falls between the flush of the
        // logged entries and the stream's attachment.
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        ensure!(log.stream.is_none(), "open timeline already streams");
        crate::files::ensure_private_directory(directory)?;
        let directory =
            crate::files::open_directory(directory, Privacy::OwnerOnly, NameRetention::Movable)?;
        let name = OsString::from(stream_name(tag));
        let file = directory.create_new(&name)?;
        let mut stream = Stream {
            directory,
            name,
            file,
            failed: false,
        };
        for &(event, ns) in &log.entries {
            stream.append(event, ns, self.anchor_unix_ns);
        }
        log.stream = Some(stream);
        Ok(())
    }

    /// Stop streaming and remove the stream file this owner created. A
    /// removal a concurrent reader leaves uncertain on Windows is not
    /// retried; nothing here fails or waits.
    pub(crate) fn end_stream(&self) {
        let stream = self
            .log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stream
            .take();
        if let Some(Stream {
            directory,
            name,
            file,
            ..
        }) = stream
        {
            let _ = directory.remove_file(&name, file);
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

/// Test-support owner event hold (Unix): when a gated owner stamps `E`
/// and the directory named by [`hold::DIR_ENV`] holds a fifo pair
/// `E.entered` and `E.release`, it writes one byte to `E.entered`, then
/// blocks the stamping thread until it reads one byte from `E.release`.
/// Each event holds at most once per process. Without the pair, or without
/// a reader on `E.entered`, nothing waits. Only the stamps of an installed
/// timeline reach it, after the stamp released the log.
#[cfg(all(unix, any(test, feature = "test-support")))]
pub(crate) mod hold {
    use super::Event;
    use anyhow::{Context as _, Result, bail};
    use nix::{
        errno::Errno,
        fcntl::OFlag,
        poll::{PollFd, PollFlags, PollTimeout},
        sys::stat::Mode,
    };
    use std::{
        os::{fd::AsFd as _, unix::fs::FileTypeExt as _},
        path::{Path, PathBuf},
        sync::{
            OnceLock,
            atomic::{AtomicU64, Ordering},
        },
        time::{Duration, Instant},
    };

    /// Names the hold directory in the owner's environment.
    pub(crate) const DIR_ENV: &str = "KURU_TEST_MEMORY_TIMELINE_HOLD_DIR";
    /// The configuration maximum of `memory.startup_timeout_secs`. It only
    /// keeps a broken test from holding an owner forever; no assertion
    /// depends on it.
    const LIMIT: Duration = Duration::from_secs(300);

    static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
    static HELD: AtomicU64 = AtomicU64::new(0);

    pub(super) fn at(event: Event) {
        let Some(directory) =
            DIRECTORY.get_or_init(|| std::env::var_os(DIR_ENV).map(PathBuf::from))
        else {
            return;
        };
        let bit = 1_u64 << (event as u32);
        if HELD.fetch_or(bit, Ordering::Relaxed) & bit != 0 {
            return;
        }
        // Reported and passed, as a failed activity hold is.
        if let Err(error) = wait(directory, event) {
            eprintln!("open timeline hold on {} failed: {error:#}", event.name());
        }
    }

    fn wait(directory: &Path, event: Event) -> Result<()> {
        let name = event.name();
        let entered = match nix::fcntl::open(
            &directory.join(format!("{name}.entered")),
            OFlag::O_WRONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
            Mode::empty(),
        ) {
            Ok(entered) => std::fs::File::from(entered),
            // No pair, or nobody reading: no hold.
            Err(Errno::ENOENT | Errno::ENXIO) => return Ok(()),
            Err(error) => return Err(error).context("open the entered fifo"),
        };
        if !entered.metadata()?.file_type().is_fifo() {
            bail!("{name}.entered is not a fifo");
        }
        nix::unistd::write(&entered, b"e").context("signal the hold")?;
        drop(entered);
        let release = nix::fcntl::open(
            &directory.join(format!("{name}.release")),
            OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .context("open the release fifo")?;
        let deadline = Instant::now() + LIMIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX);
            let mut fds = [PollFd::new(release.as_fd(), PollFlags::POLLIN)];
            match nix::poll::poll(&mut fds, timeout) {
                Ok(0) => bail!("not released within {LIMIT:?}"),
                Ok(_) => break,
                Err(Errno::EINTR) => {}
                Err(error) => return Err(error).context("poll the release fifo"),
            }
        }
        let mut byte = [0_u8; 1];
        match nix::unistd::read(&release, &mut byte).context("read the release byte")? {
            1 => Ok(()),
            _ => bail!("the holding test went away"),
        }
    }
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
                "startup-lock",
                "extract-start",
                "extract-end",
                "cache-verify-start",
                "cache-verify-end",
                "create-start",
                "supervisor-spawned",
                "channel-accepted",
                "supervisor-ready",
                "probe-verified",
                "template-copied",
                "cold-created",
                "activated",
                "main-pool",
                "version-read",
                "migrate-start",
                "migrate-end",
                "validate-active",
                "candidate-recovery",
                "usage-pool",
                "usage-bound",
                "usage-scan-1",
                "usage-upgrade",
                "usage-validate",
                "usage-scan-2",
                "usage-record",
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

    /// The stream's lines, each split into its three fields.
    fn stream_lines(path: &Path) -> Vec<(String, u64, u64)> {
        std::fs::read_to_string(path)
            .expect("a readable stream")
            .lines()
            .map(|line| {
                let fields = line.split(' ').collect::<Vec<_>>();
                assert_eq!(fields.len(), 3, "{line:?}");
                (
                    fields[0].to_owned(),
                    fields[1].parse().expect("an offset"),
                    fields[2].parse().expect("a wall-clock time"),
                )
            })
            .collect()
    }

    fn new_tag() -> String {
        crate::service::activity::activity_tag(&uuid::Uuid::new_v4())
    }

    // T1.
    #[test]
    fn the_stream_mirrors_the_log_line_by_line() {
        let root = tempfile::tempdir().unwrap();
        let services = root.path().join("services");
        let tag = new_tag();
        let timeline = Timeline::new();
        // Logged before the stream attaches: flushed when it does.
        timeline.stamp(Event::OwnerMain);
        timeline.stamp(Event::OwnerLock);
        timeline.stream_to(&services, &tag);
        let path = services.join(stream_name(&tag));
        assert_eq!(stream_lines(&path).len(), 2);
        for event in [
            Event::StartupLock,
            Event::CreateStart,
            Event::SupervisorSpawned,
            Event::EndpointPublished,
        ] {
            timeline.stamp(event);
        }
        // Sealed: counted late, never streamed.
        timeline.stamp(Event::SupervisorReady);
        let (entries, _, dropped, late, _) = snapshot(&timeline);
        assert_eq!((dropped, late), (0, 1));
        let lines = stream_lines(&path);
        assert_eq!(
            lines,
            entries
                .iter()
                .map(|&(event, ns)| (event.name().to_owned(), ns, timeline.anchor_unix_ns + ns))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            entries
                .iter()
                .map(|&(event, ns)| stream_line(event, ns, timeline.anchor_unix_ns + ns))
                .collect::<String>()
                .into_bytes()
        );

        // A full log streams nothing more.
        let tag = new_tag();
        let full = Timeline::with_capacity(2);
        full.stream_to(&services, &tag);
        for _ in 0..5 {
            full.stamp(Event::MainPool);
        }
        assert_eq!(stream_lines(&services.join(stream_name(&tag))).len(), 2);
        assert_eq!(snapshot(&full).2, 3);

        // The longest line, and a full log of them, stay within the bounds.
        let longest = Event::ALL
            .iter()
            .map(|&event| stream_line(event, u64::MAX, u64::MAX).len())
            .max()
            .unwrap();
        assert!(longest <= MAX_LINE_BYTES, "{longest} bytes");
        assert!(CAPACITY * longest <= MAX_BYTES);
    }

    // T2.
    #[test]
    fn the_stream_is_create_only_private_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let services = root.path().join("services");
        crate::files::ensure_private_directory(&services).unwrap();
        let token = uuid::Uuid::new_v4();
        let tag = crate::service::activity::activity_tag(&token);
        let path = services.join(stream_name(&tag));
        assert!(!path.to_string_lossy().contains(&token.to_string()));
        assert!(!is_timeline_name(path.file_name().unwrap()));

        // A taken name: no stream, stamps still logged, and the occupying
        // file is never appended to or removed.
        let occupied = b"owner-main 0 1\n".as_slice();
        std::fs::write(&path, occupied).unwrap();
        let refused = Timeline::new();
        refused.stamp(Event::OwnerMain);
        refused.stream_to(&services, &tag);
        refused.stamp(Event::OwnerLock);
        refused.end_stream();
        assert_eq!(snapshot(&refused).0.len(), 2);
        assert_eq!(std::fs::read(&path).unwrap(), occupied);
        std::fs::remove_file(&path).unwrap();

        // A name that is not a tag never becomes a path.
        let timeline = Timeline::new();
        timeline.stamp(Event::OwnerMain);
        for bad in ["", "../escape", "ab/cd", "not hex"] {
            timeline.stream_to(&services, bad);
        }
        assert_eq!(std::fs::read_dir(&services).unwrap().count(), 0);

        timeline.stream_to(&services, &tag);
        timeline.stamp(Event::OwnerLock);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            let fields = line.split(' ').collect::<Vec<_>>();
            assert_eq!(fields.len(), 3, "{line:?}");
            assert!(Event::ALL.iter().any(|event| event.name() == fields[0]));
            assert!(fields[1..].iter().all(|field| field.parse::<u64>().is_ok()));
        }
        assert!(!text.contains(&tag) && !text.contains(&token.to_string()));
        timeline.end_stream();
        assert!(!path.exists(), "end_stream left the stream");
        // Ended: later stamps write nothing and recreate nothing.
        timeline.stamp(Event::StartupLock);
        timeline.end_stream();
        assert_eq!(std::fs::read_dir(&services).unwrap().count(), 0);
        assert_eq!(snapshot(&timeline).0.len(), 3);
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
