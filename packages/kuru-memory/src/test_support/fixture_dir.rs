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
//! The scan reads at most 8 directory levels and 4096 entries beneath the
//! root, and does not descend into `.dolt`, Dolt's own repository directory,
//! which never holds any of those files. Whatever it cannot read is itself a
//! violation: a directory past the depth budget, an entry past the entry
//! budget, an unreadable directory or entry, and a store whose identity
//! cannot be taken. A truncated scan never passes.
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
//! panic. A fixture that already has an outcome releases its root with
//! [`TempDir::release`] instead of dropping it, so a failing verdict is
//! attached to the fixture's own error rather than replacing it with a panic. The check never waits and writes nothing to stdout or stderr. The
//! whole scan runs inside the ledger's critical section
//! ([`engine_ledger::with`]), and in the same section the teardown forgets
//! the records beneath its root, so no owner is registered, reaped or
//! recorded between the verdict and the forgetting.
//!
//! The teardown runs on the thread that drops the root, and that thread must
//! finish before the test function returns. Never move a root into a
//! detached thread: one still inside the scan when the test process exits
//! has its coverage counters written mid-function, which leaves a counter
//! expression negative and fails the coverage partition, and its verdict
//! never reaches the test. A fixture that must first wait for a creator
//! process uses [`release_after_creator_exit`], which waits on the calling
//! thread and releases or keeps the root before it returns.

use super::engine_ledger::{self, Key, Ledger};
use crate::files::PrivateTemp;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 4096;
/// How often [`release_after_creator_exit`] queries its creator.
const CREATOR_POLL: Duration = Duration::from_millis(20);

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

impl TempDir {
    /// Release the root after a fixture's `outcome`, on the calling thread,
    /// with the same check as its drop. When a store is unexplained the root
    /// is kept, and the verdict is attached to `outcome`'s error as context,
    /// or returned as the error of a successful outcome: it never replaces
    /// the fixture's own error. The drop's panic remains the check for every
    /// root that is not released this way.
    #[must_use = "the guard's verdict is returned, not raised"]
    pub fn release<T>(mut self, outcome: anyhow::Result<T>) -> anyhow::Result<T> {
        let Some(inner) = self.inner.take() else {
            return outcome;
        };
        let Some(verdict) = check(inner, &self.label) else {
            return outcome;
        };
        match outcome {
            Ok(_) => Err(anyhow::anyhow!(verdict)),
            Err(error) => Err(error.context(format!(
                "the fixture failed, and its root was then kept: {verdict}"
            ))),
        }
    }
}

/// Scan and forget the records beneath `inner`'s root, then remove it, or
/// keep it and describe why.
fn check(inner: PrivateTemp, label: &str) -> Option<String> {
    // The records beneath the root go with it, kept or removed, so no
    // later directory that recycles a native identity inherits one.
    let violations = engine_ledger::with(|ledger| {
        let violations = scan(ledger, inner.path());
        ledger.forget_under(&canonical(inner.path()));
        violations
    });
    if violations.is_empty() {
        return None;
    }
    // Never delete the tree under a possibly live engine.
    let kept = inner.keep();
    Some(format!(
        "fixture root {} (created by test {label}) was released without awaited memory \
         quiescence, so it is kept in place. Close every store, and await \
         test_support::await_managed_quiescence (managed services) or \
         test_support::await_store_quiescence (engines run by another process) before \
         the root drops. Unexplained stores: {}",
        kept.display(),
        violations.join("; ")
    ))
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
        // The root is kept on a violation even while this test is already
        // failing; only a thread that is not unwinding reports.
        if let Some(verdict) = check(inner, &self.label)
            && !std::thread::panicking()
        {
            panic!("{verdict}");
        }
    }
}

/// How [`release_after_creator_exit`] ended. Either way the root was released
/// or kept on the calling thread before it returned.
#[derive(Debug, PartialEq, Eq)]
pub enum CreatorTeardown {
    /// The creator exited and the fixture proved its descendants stopped:
    /// the root went through the guard's teardown on the calling thread.
    Released,
    /// The creator exited, but the fixture did not prove its descendants
    /// stopped: the root is kept, unchecked, at this path.
    Unproven(PathBuf),
    /// The creator did not exit within the wait, or its status query failed:
    /// the root is kept, unchecked, at this path, and the caller must keep
    /// the creator.
    Delayed(PathBuf),
}

/// Teardown for a fixture root a creator process worked in: query `status`
/// every 20 ms, for at most `wait`, until the creator has exited, then
/// release the root through the guard if `descendants_stopped`, else keep it.
///
/// Everything happens on the calling thread, before this returns, so the
/// guard's verdict reaches the test and no guard code outlives it.
pub fn release_after_creator_exit<S>(
    root: TempDir,
    wait: Duration,
    descendants_stopped: bool,
    status: impl FnMut() -> io::Result<Option<S>>,
) -> CreatorTeardown {
    if !await_creator_exit(wait, status) {
        return CreatorTeardown::Delayed(root.keep());
    }
    if descendants_stopped {
        drop(root);
        CreatorTeardown::Released
    } else {
        CreatorTeardown::Unproven(root.keep())
    }
}

/// Query `status` every 20 ms on the calling thread, for at most `wait`,
/// until the creator has exited. False when the wait ended or a status query
/// failed; the caller then keeps what the creator worked in, and the creator.
/// The wait for [`release_after_creator_exit`] and for any fixture that
/// holds something other than a guarded root until its creator exits.
pub fn await_creator_exit<S>(
    wait: Duration,
    mut status: impl FnMut() -> io::Result<Option<S>>,
) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        match status() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(CREATOR_POLL),
            _ => return false,
        }
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
    if found.exhausted {
        found.unscanned.push(format!(
            "the scan of {} reached its {MAX_ENTRIES}-entry budget before it finished, so the \
             stores beyond it were not checked",
            root.display()
        ));
    }
    let mut violations = ledger.live_under(&canonical);
    violations.append(&mut found.unscanned);
    let mut stores = BTreeMap::new();
    for store in found.leased {
        match engine_ledger::key(&store) {
            Some(key) => {
                stores.entry(store).or_insert(key);
            }
            None => violations.push(unkeyed(&store)),
        }
    }
    for store in found.identified {
        let Some(key) = engine_ledger::key(&store) else {
            violations.push(unkeyed(&store));
            continue;
        };
        if key
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

fn unkeyed(store: &Path) -> String {
    format!(
        "the identity of store {} could not be taken, so it was not checked",
        store.display()
    )
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
    /// What the scan could not read: each is a violation, never a pass.
    unscanned: Vec<String>,
    /// An entry was left unread when the entry budget ran out.
    exhausted: bool,
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

/// Dolt's own repository directory. Kuru never places a store, a lock file
/// or `identity.json` inside it, and a real store's repository nests past the
/// depth budget (`data/kuru/.dolt/stats/.dolt/noms/oldgen`), so the scan does
/// not descend into it. It is the only directory the scan skips.
const DOLT_REPOSITORY: &str = ".dolt";

/// Read `directory` into `found`. Anything left unread (a directory past the
/// depth budget, an unreadable directory or entry, an entry past the entry
/// budget) is recorded as a violation, so a truncated scan never passes.
fn collect(directory: &Path, depth: usize, budget: &mut usize, found: &mut Found) {
    if depth > MAX_DEPTH {
        found.unscanned.push(format!(
            "directory {} is deeper than the scan's {MAX_DEPTH}-level depth budget, so the \
             stores beneath it were not checked",
            directory.display()
        ));
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            found.unscanned.push(format!(
                "directory {} could not be read ({error}), so the stores beneath it were not \
                 checked",
                directory.display()
            ));
            return;
        }
    };
    for entry in entries {
        if *budget == 0 {
            found.exhausted = true;
            return;
        }
        *budget -= 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                found.unscanned.push(format!(
                    "an entry of directory {} could not be read ({error}), so it was not checked",
                    directory.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                found.unscanned.push(format!(
                    "the type of {} could not be read ({error}), so it was not checked",
                    path.display()
                ));
                continue;
            }
        };
        if kind.is_dir() {
            if entry.file_name() != DOLT_REPOSITORY {
                collect(&path, depth + 1, budget, found);
            }
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

    /// A creator process as the Windows owner fixtures hold one: it exits
    /// after `polls` status queries. `handle` stands for its retained process
    /// handle.
    struct Creator {
        polls: usize,
        queried: usize,
        handle: std::sync::Arc<()>,
    }

    impl Creator {
        fn exiting_after(polls: usize) -> Self {
            Self {
                polls,
                queried: 0,
                handle: std::sync::Arc::new(()),
            }
        }

        fn try_wait(&mut self) -> io::Result<Option<()>> {
            self.queried += 1;
            Ok((self.queried > self.polls).then_some(()))
        }
    }

    /// The shape of `tests/windows_lifecycle.rs`'s `Fixture`, whose drop
    /// waits for its creator and then releases or keeps the root.
    struct CreatorFixture {
        root: Option<TempDir>,
        creator: Option<Creator>,
        descendants_stopped: bool,
        wait: Duration,
        outcome: std::sync::mpsc::Sender<CreatorTeardown>,
    }

    impl CreatorFixture {
        fn new(
            root: TempDir,
            creator: Creator,
            descendants_stopped: bool,
            wait: Duration,
        ) -> (Self, std::sync::mpsc::Receiver<CreatorTeardown>) {
            let (outcome, received) = std::sync::mpsc::channel();
            let fixture = Self {
                root: Some(root),
                creator: Some(creator),
                descendants_stopped,
                wait,
                outcome,
            };
            (fixture, received)
        }
    }

    impl Drop for CreatorFixture {
        // The same match as the real fixture's drop, so its borrows compile
        // here too: the creator is kept only when the teardown was delayed.
        fn drop(&mut self) {
            let root = self.root.take().unwrap();
            let mut creator = self.creator.take().unwrap();
            match release_after_creator_exit(root, self.wait, self.descendants_stopped, || {
                creator.try_wait()
            }) {
                CreatorTeardown::Delayed(root) => {
                    // As the real fixture keeps its creator's process handle.
                    std::mem::forget(creator);
                    self.outcome.send(CreatorTeardown::Delayed(root)).unwrap();
                }
                outcome => self.outcome.send(outcome).unwrap(),
            }
        }
    }

    /// The Windows owner fixtures' teardown releases the root on the test
    /// thread: it is gone by the time the fixture's drop returns.
    #[test]
    fn a_creator_fixture_releases_its_root_before_its_drop_returns() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        engine_ledger::record(&store);
        let path = root.path().to_path_buf();
        let creator = Creator::exiting_after(2);
        let handle = creator.handle.clone();
        let (fixture, outcome) = CreatorFixture::new(root, creator, true, Duration::from_secs(40));
        drop(fixture);
        assert_eq!(outcome.try_recv(), Ok(CreatorTeardown::Released));
        assert!(
            !path.exists(),
            "the root must be released before the fixture's drop returns"
        );
        assert_eq!(
            std::sync::Arc::strong_count(&handle),
            1,
            "an exited creator is released"
        );
    }

    /// The guard's verdict reaches the test thread: an unexplained store
    /// fails the fixture's own drop, which a detached thread never could.
    #[test]
    fn a_creator_fixture_fails_its_own_drop_for_an_unexplained_store() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let path = root.path().to_path_buf();
        let (fixture, outcome) = CreatorFixture::new(
            root,
            Creator::exiting_after(0),
            true,
            Duration::from_secs(40),
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(fixture)))
            .expect_err("the guard must fail the fixture's drop on this thread");
        let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
        let expected = format!("store {} has no quiescence record", store.display());
        assert!(
            message.contains(&expected),
            "{expected:?} missing: {message}"
        );
        assert!(
            outcome.try_recv().is_err(),
            "the failing drop reported no outcome"
        );
        assert!(path.exists(), "an unexplained root is kept");
        remove_kept(&path);
    }

    /// Unproven descendants and a creator that outlives the wait keep the
    /// root, unchecked, before the drop returns: an unexplained store in it
    /// does not fail.
    #[test]
    fn a_creator_fixture_keeps_its_root_for_unproven_descendants_or_a_live_creator() {
        for (creator, descendants_stopped, wait) in [
            (Creator::exiting_after(1), false, Duration::from_secs(40)),
            (Creator::exiting_after(usize::MAX), true, Duration::ZERO),
        ] {
            let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
            stopped_store(root.path());
            let path = root.path().to_path_buf();
            let handle = creator.handle.clone();
            let (fixture, outcome) = CreatorFixture::new(root, creator, descendants_stopped, wait);
            drop(fixture);
            // Only a creator that did not exit keeps its handle.
            let (expected, handles) = if descendants_stopped {
                (CreatorTeardown::Delayed(path.clone()), 2)
            } else {
                (CreatorTeardown::Unproven(path.clone()), 1)
            };
            assert_eq!(outcome.try_recv(), Ok(expected));
            assert!(path.exists(), "a kept root stays in place");
            assert_eq!(std::sync::Arc::strong_count(&handle), handles);
            remove_kept(&path);
        }
    }

    /// A failed status query keeps the root at once, as the fixture reported
    /// a failed query at once before.
    #[test]
    fn a_failed_creator_status_query_keeps_the_root_without_waiting() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let path = root.path().to_path_buf();
        let mut queries = 0;
        let outcome = release_after_creator_exit(root, Duration::from_secs(40), true, || {
            queries += 1;
            Err::<Option<()>, _>(io::Error::other("controlled status query failure"))
        });
        assert_eq!(outcome, CreatorTeardown::Delayed(path.clone()));
        assert_eq!(queries, 1);
        assert!(path.exists());
        remove_kept(&path);
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

    /// A directory below the depth budget was never read, so a store in it
    /// could not be checked: the scan reports it instead of passing.
    #[test]
    fn a_directory_beyond_the_depth_budget_is_a_violation() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let mut deep = root.path().to_path_buf();
        for level in 0..=MAX_DEPTH {
            deep.push(format!("level-{level}"));
        }
        fs::create_dir_all(&deep).unwrap();
        let found = violations(root.path());
        assert!(
            found
                .iter()
                .any(|violation| violation.contains("depth budget")
                    && violation.contains(&deep.display().to_string())),
            "{found:?}"
        );
        let path = root.path().to_path_buf();
        let message = panic_message(root);
        assert!(message.contains("depth budget"), "{message}");
        remove_kept(&path);
    }

    /// Entries past the entry budget were never read: the scan reports the
    /// exhausted budget instead of passing.
    #[test]
    fn an_exhausted_entry_budget_is_a_violation() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        for index in 0..=MAX_ENTRIES {
            fs::write(root.path().join(format!("entry-{index}")), b"").unwrap();
        }
        let found = violations(root.path());
        assert!(
            found
                .iter()
                .any(|violation| violation.contains("entry budget")),
            "{found:?}"
        );
        let path = root.path().to_path_buf();
        let message = panic_message(root);
        assert!(message.contains("entry budget"), "{message}");
        remove_kept(&path);
    }

    /// A directory the scan cannot read may hold a store: it is reported.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_is_a_violation() {
        use std::os::unix::fs::PermissionsExt;
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let sealed = root.path().join("sealed");
        fs::create_dir(&sealed).unwrap();
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
        let found = violations(root.path());
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            found
                .iter()
                .any(|violation| violation.contains("could not be read")
                    && violation.contains(&sealed.display().to_string())),
            "{found:?}"
        );
    }

    /// A real store's Dolt repository nests past the depth budget
    /// (`data/kuru/.dolt/stats/.dolt/noms/oldgen`). The scan does not descend
    /// into a `.dolt` directory, so a recorded store with a deep repository
    /// still passes, within both budgets.
    #[test]
    fn a_recorded_store_with_a_deep_dolt_repository_passes() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let mut deep = store.join("data").join("kuru").join(".dolt");
        for name in ["stats", ".dolt", "noms", "oldgen", "a", "b", "c", "d"] {
            deep.push(name);
        }
        fs::create_dir_all(&deep).unwrap();
        for index in 0..=MAX_ENTRIES {
            fs::write(deep.join(format!("table-{index}")), b"").unwrap();
        }
        engine_ledger::record(&store);
        assert_eq!(violations(root.path()), Vec::<String>::new());
        let path = root.path().to_path_buf();
        drop(root);
        assert!(!path.exists(), "a recorded root is removed");
    }

    /// `release` returns the guard's verdict instead of panicking: a
    /// successful outcome becomes the verdict, a failed one keeps its own
    /// error with the verdict attached, and a clean root is removed.
    #[test]
    fn release_attaches_the_verdict_to_the_fixture_outcome() {
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let path = root.path().to_path_buf();
        assert_eq!(root.release(Ok(7)).unwrap(), 7);
        assert!(!path.exists(), "a clean root is removed");

        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let path = root.path().to_path_buf();
        let verdict = root.release(Ok(())).unwrap_err().to_string();
        let expected = format!("store {} has no quiescence record", store.display());
        assert!(verdict.contains(&expected), "{verdict}");
        assert!(path.exists(), "an unexplained root is kept");
        remove_kept(&path);

        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = stopped_store(root.path());
        let path = root.path().to_path_buf();
        let error = root
            .release::<()>(Err(anyhow::anyhow!("fixture failure")))
            .unwrap_err();
        let expected = format!("store {} has no quiescence record", store.display());
        assert_eq!(error.root_cause().to_string(), "fixture failure");
        assert!(format!("{error:#}").contains(&expected), "{error:#}");
        assert!(path.exists(), "an unexplained root is kept");
        remove_kept(&path);
    }
}
