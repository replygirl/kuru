//! Fixture roots that refuse to release a directory under a live memory owner.
//!
//! Dolt writes its final table-file manifest while it stops. Removing its data
//! directory first makes the engine panic at close (dolthub/dolt#10971), and a
//! supervisor's final `server.log` write then fails. So a fixture root is
//! released only after every Dolt it hosted has been reaped.
//!
//! [`TempDir`]'s teardown checks that invariant from records, never from the
//! state of a lock. A lock probe cannot tell a live owner from a descriptor a
//! sibling thread's child inherited between `posix_spawn` and `exec`, so the
//! guard only uses lock *files* to find the stores beneath its root:
//!
//! - a directory holding `lifecycle.lock` (the Unix lifecycle lease),
//! - `memory/<hash>` for a `memory/locks/<hash>.service-owner.lock`,
//! - a directory holding `identity.json` whose native identity names a
//!   `lifecycles/<identity>.lock` lease (the Windows lifecycle lease, which
//!   lives outside the store). A supervisor takes that lease before it writes
//!   `identity.json` and starts Dolt, so a template or an unopened copy,
//!   which carries `identity.json` but no lease, is not a store.
//!
//! Both lease forms are recognised on every platform.
//!
//! Each store must then be explained by the process-local ledger
//! ([`super::engine_ledger`]):
//!
//! - no Dolt supervisor this process spawned for it is still unreaped, and
//! - it has a quiescence record whose snapshot of the engine-written
//!   `server.log` and `endpoint.json` still matches, so no engine ran after
//!   it. This process writes the record itself when it reaps a supervisor
//!   (every store close path), and [`super::await_store_quiescence`] writes it
//!   while holding the lifecycle lease (managed services and engines run by
//!   other processes, through [`super::await_managed_quiescence`]).
//!
//! If any store is unexplained, teardown keeps the whole root, so a live
//! engine can finish, and fails the test with the fixture, the test and each
//! store. A thread that is already unwinding keeps the root without a second
//! panic. The check never waits and writes nothing to stdout or stderr. The
//! whole scan runs inside the ledger's critical section
//! ([`engine_ledger::with`]), so no two teardowns execute it concurrently,
//! and in the same section the teardown forgets the records beneath its
//! root.

use super::engine_ledger::{self, Key, Ledger};
use crate::files::PrivateTemp;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 4096;

/// A private fixture root, checked for unexplained memory owners before
/// removal.
pub struct TempDir {
    inner: Option<PrivateTemp>,
    label: String,
}

impl TempDir {
    pub fn new(prefix: &str, parent: Option<&Path>) -> anyhow::Result<Self> {
        Ok(Self {
            inner: Some(PrivateTemp::new(prefix, parent)?),
            label: super::lifecycle_trace::label(),
        })
    }

    pub fn path(&self) -> &Path {
        self.inner
            .as_ref()
            .expect("fixture root is present until drop")
            .path()
    }

    /// Keep the root in place, unchecked, for a fixture that reports its own
    /// cleanup failure. Nothing beneath it is removed.
    pub fn keep(mut self) -> PathBuf {
        self.inner
            .take()
            .expect("fixture root is present until drop")
            .keep()
    }
}

impl std::fmt::Debug for TempDir {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TempDir")
            .field("path", &self.inner.as_ref().map(PrivateTemp::path))
            .finish()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };
        // The records beneath the root go with it, kept or removed, so no
        // later directory that recycles a native identity inherits one.
        let violations = engine_ledger::with(|ledger| {
            let violations = scan(ledger, inner.path());
            ledger.forget_under(&canonical(inner.path()));
            violations
        });
        if violations.is_empty() {
            return;
        }
        // Never delete the tree under a possibly live engine, even while this
        // test is already failing; only a thread that is not unwinding reports.
        let kept = inner.keep();
        if std::thread::panicking() {
            return;
        }
        panic!(
            "fixture root {} (created by test {}) was released without awaited memory \
             quiescence, so it is kept in place. Close every store, and await \
             test_support::await_managed_quiescence (managed services) or \
             test_support::await_store_quiescence (engines run by another process) before \
             the root drops. Unexplained stores: {}",
            kept.display(),
            self.label,
            violations.join("; ")
        );
    }
}

/// Every store beneath `root` that no quiescence record explains, described
/// for a failure message, without forgetting any record: the teardown's scan
/// as the guard's own tests observe it mid-fixture.
#[cfg(test)]
fn violations(root: &Path) -> Vec<String> {
    engine_ledger::with(|ledger| scan(ledger, root))
}

fn canonical(root: &Path) -> PathBuf {
    fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

fn scan(ledger: &Ledger, root: &Path) -> Vec<String> {
    let canonical = canonical(root);
    let mut found = Found::default();
    let mut budget = MAX_ENTRIES;
    collect(root, 0, &mut budget, &mut found);
    let mut violations = ledger.live_under(&canonical);
    let mut stores = BTreeMap::new();
    for store in found.leased {
        if let Some(key) = engine_ledger::key(&store) {
            stores.entry(store).or_insert(key);
        }
    }
    for store in found.identified {
        if let Some(key) = engine_ledger::key(&store)
            && key
                .lease_name()
                .is_some_and(|name| found.leases.contains(OsStr::new(&name)))
        {
            stores.entry(store).or_insert(key);
        }
    }
    violations.extend(
        stores
            .into_iter()
            .filter_map(|(store, key)| unexplained(ledger, &store, &key)),
    );
    violations
}

fn unexplained(ledger: &Ledger, store: &Path, key: &Key) -> Option<String> {
    let Some(record) = ledger.recorded(key) else {
        return Some(format!(
            "store {} has no quiescence record",
            store.display()
        ));
    };
    (*record != engine_ledger::snapshot(store)).then(|| {
        format!(
            "store {} ran an engine after its quiescence record (server.log or endpoint.json \
             changed)",
            store.display()
        )
    })
}

/// The lease files and store candidates one scan finds.
#[derive(Default)]
struct Found {
    /// Store directories named by an in-store or service-owner lock.
    leased: Vec<PathBuf>,
    /// Directories holding `identity.json`.
    identified: Vec<PathBuf>,
    /// File names beneath every `lifecycles` directory.
    leases: BTreeSet<OsString>,
}

impl Found {
    fn file(&mut self, directory: &Path, path: &Path) {
        if let Some(store) = store_of(path) {
            self.leased.push(store);
        }
        let Some(name) = path.file_name() else {
            return;
        };
        if name == "identity.json" {
            self.identified.push(directory.to_path_buf());
        } else if directory.file_name() == Some(OsStr::new("lifecycles")) {
            self.leases.insert(name.to_owned());
        }
    }
}

/// The store directory a lock file beneath the root stands for.
fn store_of(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    if name == "lifecycle.lock" {
        return path.parent().map(Path::to_path_buf);
    }
    let hash = name.strip_suffix(".service-owner.lock")?;
    let locks = path.parent()?;
    (locks.file_name()? == "locks")
        .then(|| locks.parent().map(|memory| memory.join(hash)))
        .flatten()
        .filter(|store| store.is_dir())
}

fn collect(directory: &Path, depth: usize, budget: &mut usize, found: &mut Found) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect(&path, depth + 1, budget, found);
        } else if kind.is_file() {
            found.file(directory, &path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files;

    /// A stopped store as its supervisor leaves it: private, with a lease
    /// file and a final `server.log`.
    fn stopped_store(root: &Path) -> PathBuf {
        // Joined by component, so the expected path matches the one the
        // guard reads back from the directory on every platform.
        let store = root.join("memory").join("abcdef");
        for directory in [root.join("memory"), store.clone()] {
            files::private_dir(&directory).unwrap();
        }
        files::write(&store.join("server.log"), b"Kuru engine shutdown: Ok").unwrap();
        files::write(&store.join("lifecycle.lock"), b"").unwrap();
        store
    }

    fn panic_message(root: TempDir) -> String {
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(root)))
            .expect_err("an unexplained store must fail teardown");
        panic.downcast_ref::<String>().cloned().unwrap_or_default()
    }

    fn remove_kept(path: &Path) {
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn lock_files_name_their_stores() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let locks = root.path().join("memory/locks");
        files::private_dir(&locks).unwrap();
        let expected = [
            (store.join("lifecycle.lock"), Some(store.clone())),
            (locks.join("abcdef.service-owner.lock"), Some(store.clone())),
            (locks.join("missing.service-owner.lock"), None),
            (locks.join("abcdef.service-start.lock"), None),
            (root.path().join("memory/lifecycles/00ff.lock"), None),
            (root.path().join("cache/versions/install.lock"), None),
        ];
        for (path, store) in expected {
            // Inside the ledger's section, as every scan calls it.
            let named = engine_ledger::with(|_| store_of(&path));
            assert_eq!(named, store, "{}", path.display());
        }
        engine_ledger::record(&store);
    }

    /// The Windows lease lives outside the store, as
    /// `lifecycles/<identity>.lock`. A directory holding `identity.json` is a
    /// store only once such a lease names it: a template or an unopened copy
    /// carries the file but never ran an engine.
    #[test]
    fn an_identified_directory_is_a_store_only_under_its_external_lease() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let memory = root.path().join("memory");
        let store = memory.join("0123");
        for directory in [&memory, &store] {
            files::private_dir(directory).unwrap();
        }
        files::write(&store.join("identity.json"), b"{}").unwrap();
        assert_eq!(violations(root.path()), Vec::<String>::new());
        let lease = engine_ledger::with(|_| engine_ledger::key(&store))
            .and_then(|key| key.lease_name())
            .expect("a private store directory has a native identity");
        let lifecycles = memory.join("lifecycles");
        files::private_dir(&lifecycles).unwrap();
        files::write(&lifecycles.join("00ff.lock"), b"").unwrap();
        assert_eq!(violations(root.path()), Vec::<String>::new());
        files::write(&lifecycles.join(lease), b"").unwrap();
        assert_eq!(
            violations(root.path()),
            [format!(
                "store {} has no quiescence record",
                store.display()
            )]
        );
        engine_ledger::record(&store);
        assert_eq!(violations(root.path()), Vec::<String>::new());
    }

    #[test]
    fn a_store_without_a_quiescence_record_keeps_the_root_and_names_the_test() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let path = root.path().to_path_buf();
        let message = panic_message(root);
        assert!(path.exists(), "a violating root must be kept: {message}");
        for expected in [
            path.display().to_string(),
            "a_store_without_a_quiescence_record_keeps_the_root_and_names_the_test".into(),
            format!("store {} has no quiescence record", store.display()),
        ] {
            assert!(
                message.contains(&expected),
                "{expected:?} missing: {message}"
            );
        }
        remove_kept(&path);
    }

    /// A released root's records go with it, so a later directory that
    /// recycles one of its stores' native identities, even within the
    /// filesystem's birth-time granularity, finds no record.
    #[test]
    fn releasing_a_root_forgets_the_records_beneath_it() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        engine_ledger::record(&store);
        let key = engine_ledger::with(|_| engine_ledger::key(&store)).unwrap();
        assert!(engine_ledger::with(|ledger| ledger
            .recorded(&key)
            .is_some()));
        drop(root);
        assert!(
            engine_ledger::with(|ledger| ledger.recorded(&key).is_none()),
            "a released root's records must not outlive it"
        );
    }

    /// A record describes the directory it was taken for, not whichever
    /// directory later carries the same native identity. Linux recycles a
    /// removed directory's inode at once, so a new store can share the
    /// `(device, inode)` of a recorded store that no longer exists. Moving the
    /// recorded store's birth time stands in for that recycled identity here.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_record_for_an_earlier_directory_with_the_same_identity_explains_nothing() {
        use std::os::darwin::fs::FileTimesExt;
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        engine_ledger::record(&store);
        assert!(violations(root.path()).is_empty());
        let born = fs::metadata(&store).unwrap().created().unwrap();
        fs::File::open(&store)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_created(born - std::time::Duration::from_secs(3600)),
            )
            .unwrap();
        let path = root.path().to_path_buf();
        let message = panic_message(root);
        let expected = format!("store {} has no quiescence record", store.display());
        assert!(
            message.contains(&expected),
            "{expected:?} missing: {message}"
        );
        remove_kept(&path);
    }

    #[test]
    fn an_unreaped_in_process_owner_fails_until_its_reap_records_quiescence() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = fs::canonicalize(stopped_store(root.path())).unwrap();
        let live = engine_ledger::register(&store);
        assert!(
            violations(root.path()).iter().any(|violation| violation
                .contains("has not been reaped")
                && violation.contains(&store.display().to_string())
                && violation.contains(
                    "an_unreaped_in_process_owner_fails_until_its_reap_records_quiescence"
                )),
            "{:?}",
            violations(root.path())
        );
        // This process observed the reap: the owner leaves the ledger and
        // records the store it served.
        drop(live);
        let path = root.path().to_path_buf();
        drop(root);
        assert!(!path.exists());
    }

    #[test]
    fn an_engine_run_after_the_record_makes_it_stale() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        engine_ledger::record(&store);
        assert!(violations(root.path()).is_empty());
        // A later engine publishes an endpoint; its stop rewrites the log.
        files::write(
            &store.join("endpoint.json"),
            br#"{"instance":"later","port":4242}"#,
        )
        .unwrap();
        let path = root.path().to_path_buf();
        let message = panic_message(root);
        assert!(
            message.contains("ran an engine after its quiescence record"),
            "{message}"
        );
        remove_kept(&path);
    }

    /// The fork race the old lock probe mistook for a live owner: the lease
    /// descriptor the quiescence wait held is duplicated into a child that
    /// has not reached `exec` yet, so the flock is still held when the root
    /// drops. The record, not the lock, decides.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_duplicated_lease_descriptor_after_awaited_quiescence_does_not_fail() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let mut inherited = None;
        super::super::await_store_quiescence_observed(&store, None, |lease| {
            inherited = Some(
                lease
                    .try_clone()
                    .expect("duplicate the held lease descriptor"),
            );
        })
        .await
        .unwrap();
        let inherited = inherited.expect("the lease was observed while held");
        let probe = fs::File::open(store.join("lifecycle.lock")).unwrap();
        assert!(
            matches!(probe.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            "the duplicated descriptor must still hold the lease"
        );
        drop(probe);
        let path = root.path().to_path_buf();
        drop(root);
        assert!(!path.exists(), "a recorded root is removed");
        drop(inherited);
    }

    #[test]
    fn a_panicking_test_keeps_the_root_without_a_second_panic() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let joined = std::thread::Builder::new()
            .name("fixture-invariant-unwinding".into())
            .spawn(move || {
                let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
                stopped_store(root.path());
                sender.send(root.path().to_path_buf()).unwrap();
                panic!("original test failure");
            })
            .unwrap()
            .join();
        let payload = joined.expect_err("the test body panicked");
        assert_eq!(
            payload.downcast_ref::<&str>().copied(),
            Some("original test failure"),
            "the guard must not replace or add to the original panic"
        );
        let path = receiver.recv().unwrap();
        assert!(path.exists(), "an unexplained root is kept while unwinding");
        remove_kept(&path);
    }
}
