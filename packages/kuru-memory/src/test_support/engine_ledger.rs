//! Process-local quiescence records behind the fixture teardown invariant.
//!
//! The ledger never probes a lock. It holds two facts this process observed
//! itself:
//!
//! - **Live owners.** Every Dolt supervisor this process spawned registers its
//!   store directory when its `Owner` is built and stays live until this
//!   process has reaped that supervisor (`finish_owner`, or the thread a
//!   dropped `Owner` hands its child to, which reports the reap through a
//!   [`Reaper`]). The supervisor reaps Dolt before it exits, so a reaped
//!   supervisor means a reaped engine.
//! - **Quiescence records.** When a live owner is reaped, and when a fixture
//!   awaits quiescence through [`super::await_store_quiescence`] while it holds
//!   the store's lifecycle lease, the ledger stores a [`Snapshot`] of the
//!   store's engine-written files, keyed by the directory's native identity so
//!   the record follows a rename (staging to active, quarantine).
//!
//! A record is current only while its snapshot still matches: every engine
//! start publishes `endpoint.json` and every stop writes `server.log` and
//! retires the endpoint, so an engine that ran after the record, in any
//! process, makes it stale. Duplicated lock descriptors (a sibling thread's
//! child between `posix_spawn` and `exec`) change none of this.
//!
//! A native identity names a directory only while it exists: Linux recycles a
//! removed directory's inode at once, so a new store can carry the `(device,
//! inode)` of a recorded store that was since removed. Two measures keep a
//! removed store's record from standing for a new one. Releasing a fixture
//! root forgets every record taken beneath it ([`Ledger::forget_under`]), so
//! the stores removed with a root leave nothing behind. For a store removed
//! some other way while the process runs, the key also holds the directory's
//! birth time, which a rename preserves; Linux stamps it at clock-tick
//! granularity, and such a removal follows an engine run far longer than a
//! tick. Where the filesystem reports no birth time the key is the identity
//! alone.
//!
//! Every read and write of the ledger, and the fixture guard's whole scan,
//! runs inside one critical section ([`with`]). A teardown's scan and its
//! [`Ledger::forget_under`] then see one consistent ledger: no owner is
//! registered, reaped or recorded between them.
//!
//! Ledger and guard code runs only on threads that finish before their test
//! function returns. A thread still running when the test process exits is
//! caught mid-function by the exit-time coverage profile write: a function's
//! entry counter can be written while a later counter is not, which leaves a
//! counter expression negative, and the coverage partition refuses it (the
//! instrumented increments are atomic; no update is lost). A dropped `Owner`'s
//! reaper thread is such a thread, since it outlives its owner by design, so
//! it only sends a [`Reaped`] report, gathered with standard-library calls;
//! the ledger records it on whichever thread reads the ledger next
//! ([`with`] settles pending reports first).

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Instant, SystemTime},
};

/// A store directory: its identity, and its birth time where the filesystem
/// reports one.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Key {
    identity: Identity,
    born: Option<SystemTime>,
}

/// Native identity of a store directory, or its canonical path when the
/// directory cannot be opened as a private directory.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Identity {
    Native([u8; 24]),
    Path(PathBuf),
}

impl Key {
    /// The file name of this directory's external lifecycle lease, as the
    /// Windows `LifecycleLease` names it beneath its `lifecycles` root.
    pub(crate) fn lease_name(&self) -> Option<String> {
        let Identity::Native(bytes) = &self.identity else {
            return None;
        };
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Some(format!("{hex}.lock"))
    }
}

/// One engine-written file, as far as a later start or stop would change it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileState {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    inode: u64,
}

/// The engine-written files of one store directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Snapshot {
    server_log: Option<FileState>,
    endpoint: Option<FileState>,
}

struct LiveOwner {
    directory: PathBuf,
    key: Option<Key>,
    label: String,
    /// Set when a dropped owner hands its supervisor to a reaper thread: the
    /// channel its [`Reaped`] report arrives on, and the dropping test.
    reaper: Option<(mpsc::Receiver<Reaped>, String)>,
}

/// A quiescence record: the snapshot, the canonical directory it was taken
/// in, and when.
struct Record {
    directory: PathBuf,
    snapshot: Snapshot,
    taken: Instant,
}

impl Record {
    fn of(directory: &Path) -> Self {
        Self {
            directory: fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf()),
            snapshot: snapshot(directory),
            taken: Instant::now(),
        }
    }

    /// The record a reaper's report describes, as of the observed reap.
    fn reported(directory: PathBuf, report: Reaped) -> Self {
        Self {
            directory: report.directory.unwrap_or(directory),
            snapshot: Snapshot {
                server_log: state(report.server_log),
                endpoint: state(report.endpoint),
            },
            taken: report.at,
        }
    }
}

/// Something an open did to the shared store template that test support
/// charges to the fixture it opened for: a fixture's open never builds the
/// shared template (warm-up does) and quarantines it only on a real verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TemplateEvent {
    /// The open built the store template under the shared test root.
    Built,
    /// The open quarantined the shared store template.
    Quarantined,
}

/// One [`TemplateEvent`], by the canonical stage it was created for and the
/// test that caused it.
struct TemplateRecord {
    stage: PathBuf,
    event: TemplateEvent,
    label: String,
}

#[derive(Default)]
pub(crate) struct Ledger {
    live: HashMap<u64, LiveOwner>,
    records: HashMap<Key, Record>,
    template_events: Vec<TemplateRecord>,
    /// Supervisors this process started, per store directory, with the
    /// canonical directory of the first, for tests that prove a path starts
    /// no engine on a directory.
    #[cfg(test)]
    starts: HashMap<Key, (PathBuf, u64)>,
}

static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

/// The ledger survives a sibling test that panicked while holding it: its
/// entries are plain facts, so a poisoned guard is still consistent enough to
/// read, and a teardown during unwinding must never panic on it.
fn ledger() -> MutexGuard<'static, Option<Ledger>> {
    LEDGER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run `action` inside the ledger's single critical section, after
/// recording every reap a dropped owner's reaper has reported. Nothing called
/// from it may take the ledger again.
pub(crate) fn with<T>(action: impl FnOnce(&mut Ledger) -> T) -> T {
    let mut guard = ledger();
    let ledger = guard.get_or_insert_with(Ledger::default);
    ledger.settle();
    action(ledger)
}

pub(crate) fn key(directory: &Path) -> Option<Key> {
    let canonical = fs::canonicalize(directory).ok()?;
    let born = fs::metadata(&canonical)
        .and_then(|metadata| metadata.created())
        .ok();
    let identity = match crate::files::directory(&canonical) {
        Ok(opened) => Identity::Native(opened.identity().to_bytes()),
        Err(_) => Identity::Path(canonical),
    };
    Some(Key { identity, born })
}

fn file_state(path: &Path) -> Option<FileState> {
    state(fs::symlink_metadata(path))
}

fn state(metadata: io::Result<fs::Metadata>) -> Option<FileState> {
    let metadata = metadata.ok()?;
    Some(FileState {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        inode: std::os::unix::fs::MetadataExt::ino(&metadata),
    })
}

pub(crate) fn snapshot(directory: &Path) -> Snapshot {
    Snapshot {
        server_log: file_state(&directory.join("server.log")),
        endpoint: file_state(&directory.join("endpoint.json")),
    }
}

/// Record that `directory` has no live engine now. Callers hold the evidence
/// themselves: the store's lifecycle lease, or their own reap of its owner.
pub(crate) fn record(directory: &Path) {
    with(|ledger| {
        if let Some(key) = key(directory) {
            ledger.insert(key, Record::of(directory));
        }
    });
}

/// Charge `event` on the shared store template to the fixture whose stage
/// `stage` (beneath its root) was being created.
pub(crate) fn template_event(stage: &Path, event: TemplateEvent) {
    let stage = fs::canonicalize(stage).unwrap_or_else(|_| stage.to_path_buf());
    let label = super::lifecycle_trace::label();
    with(|ledger| {
        ledger.template_events.push(TemplateRecord {
            stage,
            event,
            label,
        });
    });
}

impl Ledger {
    /// The store template events charged to stages beneath `root`
    /// (canonical), for a failure message.
    pub(crate) fn template_events_under(&self, root: &Path) -> Vec<String> {
        self.template_events
            .iter()
            .filter(|record| record.stage.starts_with(root))
            .map(|record| {
                let what = match record.event {
                    TemplateEvent::Built => "built the shared store template",
                    TemplateEvent::Quarantined => "quarantined the shared store template",
                };
                format!(
                    "the open creating {} (by test {}) {what}",
                    record.stage.display(),
                    record.label
                )
            })
            .collect()
    }

    /// Store `record` unless the store already has a later one: a reaper's
    /// report is recorded after the fact, so an earlier reap can arrive after
    /// a later record.
    fn insert(&mut self, key: Key, record: Record) {
        if self
            .records
            .get(&key)
            .is_none_or(|current| current.taken <= record.taken)
        {
            self.records.insert(key, record);
        }
    }

    /// Record, on the calling thread, each dropped owner whose reaper has
    /// reported the reap. An owner whose reaper is gone without a report (its
    /// thread never ran, or panicked) stays live, so its root fails.
    fn settle(&mut self) {
        let mut reaped = Vec::new();
        self.live.retain(|_, owner| {
            let Some((receiver, origin)) = &owner.reaper else {
                return true;
            };
            let Ok(report) = receiver.try_recv() else {
                return true;
            };
            let origin = origin.clone();
            reaped.push((
                std::mem::take(&mut owner.directory),
                owner.key.take(),
                origin,
                report,
            ));
            false
        });
        for (directory, key, origin, report) in reaped {
            super::lifecycle_trace::event(
                "owner_dropped_reaped",
                format_args!(
                    "origin={origin} dir_exists={} reaped_t={} directory={}",
                    report.directory.is_ok(),
                    report
                        .wall
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_nanos()),
                    directory.display()
                ),
            );
            if let Some(key) = key {
                self.insert(key, Record::reported(directory, report));
            }
        }
    }

    /// The current record for a store, if any.
    pub(crate) fn recorded(&self, key: &Key) -> Option<&Snapshot> {
        self.records.get(key).map(|record| &record.snapshot)
    }

    /// Forget every record taken beneath `root` (canonical), whose tree is
    /// being released.
    pub(crate) fn forget_under(&mut self, root: &Path) {
        self.records
            .retain(|_, record| !record.directory.starts_with(root));
        self.template_events
            .retain(|record| !record.stage.starts_with(root));
        #[cfg(test)]
        self.starts
            .retain(|_, (directory, _)| !directory.starts_with(root));
    }

    /// How many supervisors this process has started for the store directory
    /// `key` names. The key follows the directory through a rename.
    #[cfg(test)]
    pub(crate) fn starts(&self, key: &Key) -> u64 {
        self.starts.get(key).map_or(0, |(_, starts)| *starts)
    }

    /// Every store directory this process started a supervisor for, by the
    /// canonical directory of its first start, with its start count.
    #[cfg(test)]
    pub(crate) fn start_counts(&self) -> Vec<(PathBuf, u64)> {
        let mut counts = self.starts.values().cloned().collect::<Vec<_>>();
        counts.sort();
        counts
    }

    /// How many supervisors this process has started for every store
    /// directory first started beneath `root` (canonical): a store's stage
    /// and its active directory, which the key follows through the rename,
    /// and any template build store there, including one since removed.
    #[cfg(test)]
    pub(crate) fn starts_under(&self, root: &Path) -> u64 {
        self.starts
            .values()
            .filter(|(directory, _)| directory.starts_with(root))
            .map(|(_, starts)| starts)
            .sum()
    }

    /// Live owners whose store lies beneath `root` (canonical), for a failure
    /// message.
    pub(crate) fn live_under(&self, root: &Path) -> Vec<String> {
        let mut owners = self
            .live
            .values()
            .filter(|owner| owner.directory.starts_with(root))
            .map(|owner| {
                format!(
                    "in-process Dolt supervisor for store {} (started by test {}) has not been \
                     reaped",
                    owner.directory.display(),
                    owner.label
                )
            })
            .collect::<Vec<_>>();
        owners.sort();
        owners
    }
}

/// A spawned supervisor this process has not reaped yet. Drop it only after
/// this process observed the supervisor's exit; dropping records quiescence.
pub(crate) struct LiveEngine {
    id: u64,
}

/// Register a supervisor this process just spawned for `directory`
/// (canonical, as `Server::open` resolves it).
pub(crate) fn register(directory: &Path) -> LiveEngine {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    with(|ledger| {
        #[cfg(test)]
        if let Some(key) = key(directory) {
            ledger
                .starts
                .entry(key)
                .or_insert_with(|| (directory.to_path_buf(), 0))
                .1 += 1;
        }
        let owner = LiveOwner {
            directory: directory.to_path_buf(),
            key: key(directory),
            label: super::lifecycle_trace::label(),
            reaper: None,
        };
        ledger.live.insert(id, owner);
    });
    LiveEngine { id }
}

impl Drop for LiveEngine {
    fn drop(&mut self) {
        with(|ledger| {
            // The supervisor wrote its final `server.log` and retired its
            // endpoint before it exited, and this process has reaped it. The
            // key was taken while the directory was known to be this store.
            if let Some(LiveOwner {
                directory,
                key: Some(key),
                ..
            }) = ledger.live.remove(&self.id)
            {
                ledger.insert(key, Record::of(&directory));
            }
        });
    }
}

/// A dropped owner's handoff to its reaper thread, which outlives the owner
/// and may still be running when the test process exits.
pub(crate) struct Reaper {
    sender: mpsc::Sender<Reaped>,
    directory: PathBuf,
}

/// What a reaper thread observed of the store once its supervisor exited.
pub(crate) struct Reaped {
    at: Instant,
    wall: SystemTime,
    directory: io::Result<PathBuf>,
    server_log: io::Result<fs::Metadata>,
    endpoint: io::Result<fs::Metadata>,
}

impl Reaper {
    /// On the dropping thread: stop `live` from recording when it drops and
    /// wait for the reaper's report instead. Without a live entry the report
    /// goes nowhere.
    pub(crate) fn handoff(live: Option<LiveEngine>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let directory = live
            .and_then(|live| {
                let id = live.id;
                // Its drop would record now, before the reap.
                std::mem::forget(live);
                let origin = super::lifecycle_trace::label();
                with(|ledger| {
                    ledger.live.get_mut(&id).map(|owner| {
                        owner.reaper = Some((receiver, origin));
                        owner.directory.clone()
                    })
                })
            })
            .unwrap_or_default();
        Self { sender, directory }
    }

    /// On the reaper thread, once it has observed the supervisor's exit.
    ///
    /// This thread can still be running when the test process exits and
    /// writes its coverage profile, so this function must stay branch-free
    /// and call only the standard library: it has a single coverage counter,
    /// which no partial run can leave inconsistent. The ledger records the
    /// report on the next thread that reads it.
    pub(crate) fn report(self) {
        let Self { sender, directory } = self;
        let _ = sender.send(Reaped {
            at: Instant::now(),
            wall: SystemTime::now(),
            directory: fs::canonicalize(&directory),
            server_log: fs::symlink_metadata(directory.join("server.log")),
            endpoint: fs::symlink_metadata(directory.join("endpoint.json")),
        });
    }
}

impl std::fmt::Debug for LiveEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveEngine")
            .field("id", &self.id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{files, test_support::TempDir};
    use std::time::Duration;

    /// A stopped store beneath `root`, canonical, as `Server::open` registers
    /// it.
    fn stopped_store(root: &Path) -> PathBuf {
        let store = root.join("store");
        files::private_dir(&store).unwrap();
        files::write(&store.join("server.log"), b"Kuru engine shutdown: Ok").unwrap();
        fs::canonicalize(store).unwrap()
    }

    /// A later engine stops and rewrites the store's log.
    fn later_engine_ran(store: &Path) {
        files::write(
            &store.join("server.log"),
            b"a later engine ran here\nKuru engine shutdown: Graceful\n",
        )
        .unwrap();
    }

    /// Send `reaper`'s report from another thread while this thread holds the
    /// ledger. A report that took the ledger could not complete.
    fn report_while_ledger_is_held(reaper: Reaper, while_held: impl FnOnce(&mut Ledger)) {
        let (done, reported) = mpsc::channel();
        let reporter = with(|ledger| {
            let reporter = std::thread::spawn(move || {
                reaper.report();
                done.send(()).unwrap();
            });
            assert_eq!(
                reported.recv_timeout(Duration::from_secs(10)),
                Ok(()),
                "a reaper's report must not wait for the ledger"
            );
            while_held(ledger);
            reporter
        });
        reporter.join().unwrap();
    }

    /// A dropped owner's reaper thread runs no ledger code: its report is
    /// recorded by the next reader, as of the reap it observed.
    #[test]
    fn a_reapers_report_is_recorded_by_the_next_reader_as_of_the_reap() {
        let root = TempDir::new("kuru-engine-ledger-", None).unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let store = stopped_store(root.path());
        let key = key(&store).unwrap();
        let reaper = Reaper::handoff(Some(register(&store)));
        assert_eq!(with(|ledger| ledger.live_under(&canonical_root).len()), 1);
        let at_reap = snapshot(&store);
        report_while_ledger_is_held(reaper, |ledger| {
            assert_eq!(
                ledger.live_under(&canonical_root).len(),
                1,
                "a report is recorded only by a later reader"
            );
        });
        later_engine_ran(&store);
        let (live, recorded) = with(|ledger| {
            (
                ledger.live_under(&canonical_root),
                ledger.recorded(&key).cloned(),
            )
        });
        assert_eq!(live, Vec::<String>::new());
        assert_eq!(recorded, Some(at_reap));
        assert_ne!(
            recorded,
            Some(snapshot(&store)),
            "the reap, not the read, is recorded"
        );
        record(&store);
        drop(root);
    }

    /// Reports are recorded after the fact, so an earlier reap can arrive
    /// after a later record; it must not replace it.
    #[test]
    fn an_earlier_reap_reported_after_a_later_record_does_not_replace_it() {
        let root = TempDir::new("kuru-engine-ledger-", None).unwrap();
        let store = stopped_store(root.path());
        let key = key(&store).unwrap();
        let reaper = Reaper::handoff(Some(register(&store)));
        report_while_ledger_is_held(reaper, |ledger| {
            later_engine_ran(&store);
            ledger.insert(key.clone(), Record::of(&store));
        });
        assert_eq!(
            with(|ledger| ledger.recorded(&key).cloned()),
            Some(snapshot(&store))
        );
        drop(root);
    }

    /// An owner whose reaper is gone without a report stays live, so its root
    /// fails; a reaper without a live entry reports nowhere.
    #[test]
    fn a_reaper_gone_without_a_report_leaves_its_owner_live() {
        let root = TempDir::new("kuru-engine-ledger-", None).unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let store = stopped_store(root.path());
        drop(Reaper::handoff(Some(register(&store))));
        Reaper::handoff(None).report();
        let live = with(|ledger| ledger.live_under(&canonical_root));
        assert_eq!(live.len(), 1, "{live:?}");
        assert!(live[0].contains("has not been reaped"), "{live:?}");
        with(|ledger| {
            ledger
                .live
                .retain(|_, owner| !owner.directory.starts_with(&canonical_root));
        });
        record(&store);
        drop(root);
    }
}
