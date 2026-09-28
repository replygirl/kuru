//! Process-local quiescence records behind the fixture teardown invariant.
//!
//! The ledger never probes a lock. It holds two facts this process observed
//! itself:
//!
//! - **Live owners.** Every Dolt supervisor this process spawned registers its
//!   store directory when its `Owner` is built and stays live until this
//!   process has reaped that supervisor (`finish_owner`, or the thread a
//!   dropped `Owner` hands its child to). The supervisor reaps Dolt before it
//!   exits, so a reaped supervisor means a reaped engine.
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
//! inode)` of a recorded store that was since removed. A key therefore also
//! holds the directory's birth time, which a rename preserves and a recycled
//! identity does not. Where the filesystem reports no birth time the key is
//! the identity alone, and a record for a removed directory can again stand
//! for a new one with the same snapshot.
//!
//! Every read and write of the ledger, and the fixture guard's whole scan,
//! runs inside one critical section ([`with`]). Its work is then never
//! concurrent, so the instrumented coverage counters of this code (plain,
//! non-atomic increments) cannot lose an update. A lost update in the guard's
//! scan loop, which every test thread's teardown runs, left a counter
//! expression negative, which the coverage partition refuses.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
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
}

#[derive(Default)]
pub(crate) struct Ledger {
    live: HashMap<u64, LiveOwner>,
    records: HashMap<Key, Snapshot>,
}

static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

/// The ledger survives a sibling test that panicked while holding it: its
/// entries are plain facts, so a poisoned guard is still consistent enough to
/// read, and a teardown during unwinding must never panic on it.
fn ledger() -> MutexGuard<'static, Option<Ledger>> {
    LEDGER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run `action` inside the ledger's single critical section. Nothing called
/// from it may take the ledger again.
pub(crate) fn with<T>(action: impl FnOnce(&mut Ledger) -> T) -> T {
    let mut guard = ledger();
    action(guard.get_or_insert_with(Ledger::default))
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
    let metadata = fs::symlink_metadata(path).ok()?;
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
            ledger.records.insert(key, snapshot(directory));
        }
    });
}

impl Ledger {
    /// The current record for a store, if any.
    pub(crate) fn recorded(&self, key: &Key) -> Option<&Snapshot> {
        self.records.get(key)
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
        let owner = LiveOwner {
            directory: directory.to_path_buf(),
            key: key(directory),
            label: super::lifecycle_trace::label(),
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
                ledger.records.insert(key, snapshot(&directory));
            }
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
