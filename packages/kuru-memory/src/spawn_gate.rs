//! Serialises advisory-lock acquisition against child-process creation inside
//! this test binary.
//!
//! `File::try_lock`/`File::lock` acquire an advisory lock with
//! `flock(LOCK_EX | LOCK_NB)` on Unix (used here by `LifecycleLease`,
//! `cache_lock`, and every test that opens a lock file directly through
//! `Directory::lock_file`). `flock` ownership belongs to the *open file
//! description*, not to a descriptor or a process, so any duplicate of that
//! description anywhere keeps the lock held until the last duplicate closes.
//! `posix_spawn` (used to launch both the owned lifetime supervisor and the
//! Dolt engine itself) copies the whole parent descriptor table into the new
//! process and only closes `FD_CLOEXEC` descriptors at image activation,
//! which is strictly after the copy — so `O_CLOEXEC` does not close that
//! window. A thread spawning a child while another thread releases a lock
//! file therefore leaves the advisory lock held by the child's transient
//! duplicate, and the next `try_lock` on a freshly opened descriptor fails
//! with `EWOULDBLOCK`.
//!
//! `kuru-memory`'s own lib tests run lock-taking tests (`server_tests.rs`,
//! the `provision` cache-lock fixtures) concurrently with process-spawning
//! fixtures (real-Dolt store opens in `store/recovery_tests.rs`,
//! `store/migration_lifecycle_tests.rs`, `store/operational_gc_tests.rs`,
//! `provision/native_tests.rs`, plus the direct `/bin/sh` and
//! `crate::engine::spawn` calls in `server_tests.rs`) in one binary, which is
//! exactly that shape. Tests that acquire a lifecycle, startup or cache lock
//! take [`locking`]/[`locking_async`] (exclusive); tests that spawn a child
//! process take [`spawning`]/[`spawning_blocking`] (shared) across the
//! spawn, so spawning tests still run concurrently with one another and only
//! exclude lock acquisition. A test that spawns a child process without
//! taking the shared guard reopens the window.
//!
//! Child creation in the product code paths this test binary drives (the
//! lifetime supervisor launch in `Server::open*`, the Dolt engine in
//! `engine::spawn`, the project memory service in `service::spawn_service`)
//! takes [`child_creation`] under `cfg(test)`, held exactly across the spawn.
//! It is shared with every other spawn, so concurrent spawns still overlap
//! freely, and it excludes lock takers just as [`spawning`] does: a lock
//! taker's [`locking`]/[`locking_async`] returns only after every child
//! creation already in flight has finished, and a child creation waits while
//! a lock taker holds the gate. Unlike [`spawning`] it never waits behind a
//! lock taker that is merely *queued*. Many fixtures hold [`spawning`] across
//! a whole store or service open (see `test_support::spawn_gated_open`), and
//! the gate is fair, so a second fair shared acquisition nested inside one
//! would wait behind a queued writer forever while that writer waits for the
//! outer guard. A child creation by the task or thread that holds the
//! exclusive guard itself (an [`excluding_spawns`] restart, or a fixture that
//! holds the exclusive guard across its own owner spawn) is already excluded
//! from every sibling and does not wait. The scan test
//! `every_child_creation_takes_the_gate` fails when a function constructs a
//! child process without any guard. A spawn that bypasses the gate reopens
//! the window that a lock taker's exclusive guard is meant to close. That
//! scan is per function, not per span: a gate token anywhere in the
//! enclosing function body satisfies it, even a [`locking`] guard scoped to a
//! block the spawn lies outside, so a reviewer still checks that each guard
//! spans its spawn.
//!
//! The holder is matched by thread or by tokio task, and nothing else. The
//! Dolt version probes (`provision::owned_probe` and
//! `CheckedColdProbe::probe`) run `engine::spawn` on a thread of their own
//! with its own runtime, which matches neither. A test that holds
//! [`locking`]/[`locking_async`] while it triggers one therefore waits on
//! itself until its deadline. Both a cold managed provision and
//! `provision::provision` with a configured `dolt_binary` (which always
//! probes) trigger one. Warm the cache before taking the exclusive guard;
//! `no_spawn_guard_encloses_a_test_cache_warm_up` reports a warm-up or a
//! `provision(` call written while a [`locking`] guard is held, as well as a
//! warm-up under a [`spawning`] guard.
//!
//! Only a test holding the exclusive guard across the release and the next
//! non-waiting try is excluded from every sibling child creation. A test that
//! reaches such a try holding only a shared guard (an ordinary open through
//! `test_support::spawn_gated_open`) is not, by design: shared holders do not
//! exclude one another. A best-effort try there can still find a sibling's
//! duplicate, and such a test accepts the designed outcome (for the template
//! quarantine, `creation_template::tests::quarantined_or_busy`).
//!
//! This module is test-only and touches no production code path: it compiles
//! only under `cfg(test)`, so it is a strict no-op on every platform,
//! including Windows, where `flock`/`posix_spawn` do not apply — the module
//! still exists there so cross-platform test code compiles uniformly, but no
//! Windows test currently needs to take either guard, and [`child_creation`]
//! exists only on Unix.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

struct Gate {
    lock: RwLock<()>,
    #[cfg(unix)]
    creations: creation::Creations,
}

/// The exclusive guard: held by a test across advisory-lock acquisition or
/// release.
pub(crate) struct Exclusive {
    /// The gate whose creation bookkeeping this guard holds; Windows keeps
    /// no such bookkeeping, so the guard there is only the write lock.
    #[cfg(unix)]
    gate: &'static Gate,
    guard: Option<RwLockWriteGuard<'static, ()>>,
}

impl Exclusive {
    /// Give the exclusive guard up for a shared one atomically.
    fn downgrade(mut self) -> RwLockReadGuard<'static, ()> {
        let guard = self.guard.take().expect("an exclusive guard is held");
        #[cfg(unix)]
        self.gate.creations.release();
        RwLockWriteGuard::downgrade(guard)
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            #[cfg(unix)]
            self.gate.creations.release();
            drop(guard);
        }
    }
}

impl Gate {
    const fn new() -> Self {
        Self {
            lock: RwLock::const_new(()),
            #[cfg(unix)]
            creations: creation::Creations::new(),
        }
    }

    fn locking(&'static self) -> Exclusive {
        let guard = self.lock.blocking_write();
        let exclusive = self.exclusive(guard);
        #[cfg(unix)]
        self.creations.drain_blocking();
        exclusive
    }

    async fn locking_async(&'static self) -> Exclusive {
        let guard = self.lock.write().await;
        let exclusive = self.exclusive(guard);
        #[cfg(unix)]
        self.creations.drain().await;
        exclusive
    }

    /// Wrap a just-acquired write guard; from here a child creation by any
    /// other holder waits, and dropping the result releases it again.
    fn exclusive(&'static self, guard: RwLockWriteGuard<'static, ()>) -> Exclusive {
        #[cfg(unix)]
        self.creations.hold();
        Exclusive {
            #[cfg(unix)]
            gate: self,
            guard: Some(guard),
        }
    }

    async fn spawning(&self) -> RwLockReadGuard<'_, ()> {
        self.lock.read().await
    }

    #[cfg(unix)]
    fn spawning_blocking(&self) -> RwLockReadGuard<'_, ()> {
        self.lock.blocking_read()
    }

    async fn excluding<T>(
        &'static self,
        held: RwLockReadGuard<'static, ()>,
        restart: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<(T, RwLockReadGuard<'static, ()>)> {
        drop(held);
        let exclusive = self.locking_async().await;
        let value = restart.await?;
        Ok((value, exclusive.downgrade()))
    }
}

static GATE: Gate = Gate::new();

/// Hold across advisory-lock acquisition (or release) in a synchronous test.
/// Excludes every in-binary spawn.
pub(crate) fn locking() -> Exclusive {
    GATE.locking()
}

/// [`locking`] for an asynchronous test, which must not block its runtime.
pub(crate) async fn locking_async() -> Exclusive {
    GATE.locking_async().await
}

/// Hold across child-process creation in an asynchronous test. Shared between
/// concurrent spawners.
pub(crate) async fn spawning() -> RwLockReadGuard<'static, ()> {
    GATE.spawning().await
}

/// [`spawning`] for a synchronous test, which has no runtime to `.await` on.
#[cfg(unix)]
pub(crate) fn spawning_blocking() -> RwLockReadGuard<'static, ()> {
    GATE.spawning_blocking()
}

/// Hold exactly across one child creation on a product code path. Safe to
/// take while this task already holds [`spawning`]; see the module doc.
#[cfg(unix)]
pub(crate) async fn child_creation() -> creation::ChildCreation {
    GATE.creations.enter().await
}

/// Release a lock and acquire it again with every other in-binary spawn
/// excluded: give up the caller's shared guard, run `restart` under the
/// exclusive guard, then downgrade that guard atomically to the shared guard
/// the rest of the test holds. The caller never waits after `restart`: a
/// successor owner served inside it is already running and must not wait for
/// the gate afterwards, and a writer queued meanwhile can wait for other
/// tests for a long time.
///
/// A one-shot acquisition right after a release (for example a successor
/// `ServiceOwner::open` after its predecessor closed) needs this. A shared
/// guard does not exclude a sibling test's spawn, and that sibling's child can
/// hold a duplicate of the just-released description until its exec. `restart`
/// must span the release as well as the reacquisition, and the caller must not
/// hold another shared guard: the gate is fair, so a nested shared acquisition
/// would wait behind this writer forever. On an error the caller keeps no
/// guard, which is harmless for a fixture that only tears down afterwards.
/// The restart's own child creations, made by this task, do not wait for the
/// exclusive guard it holds.
pub(crate) async fn excluding_spawns<T>(
    held: RwLockReadGuard<'static, ()>,
    restart: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<(T, RwLockReadGuard<'static, ()>)> {
    GATE.excluding(held, restart).await
}

#[cfg(unix)]
mod creation {
    use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
    use std::thread::{self, ThreadId};

    use tokio::sync::Notify;

    /// The test (thread or task) holding the exclusive guard. A libtest test
    /// owns its thread and its runtime's threads, so a match on either names
    /// the same test.
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Holder {
        thread: ThreadId,
        task: Option<tokio::task::Id>,
    }

    impl Holder {
        fn current() -> Self {
            Self {
                thread: thread::current().id(),
                task: tokio::task::try_id(),
            }
        }

        fn is_current(self) -> bool {
            let current = Self::current();
            self.thread == current.thread || (self.task.is_some() && self.task == current.task)
        }
    }

    struct State {
        in_flight: usize,
        exclusive: Option<Holder>,
    }

    pub(super) struct Creations {
        state: Mutex<State>,
        drained: Condvar,
        changed: Notify,
    }

    /// One child creation in flight; dropping it ends the creation.
    #[must_use = "hold the guard across the spawn"]
    pub(crate) struct ChildCreation(Option<&'static Creations>);

    impl Drop for ChildCreation {
        fn drop(&mut self) {
            if let Some(creations) = self.0 {
                let mut state = creations.state();
                state.in_flight -= 1;
                let drained = state.in_flight == 0;
                drop(state);
                if drained {
                    creations.drained.notify_all();
                    creations.changed.notify_waiters();
                }
            }
        }
    }

    impl Creations {
        pub(super) const fn new() -> Self {
            Self {
                state: Mutex::new(State {
                    in_flight: 0,
                    exclusive: None,
                }),
                drained: Condvar::new(),
                changed: Notify::const_new(),
            }
        }

        fn state(&self) -> MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        /// Mark the exclusive guard held by the current test.
        pub(super) fn hold(&self) {
            self.state().exclusive = Some(Holder::current());
        }

        pub(super) fn release(&self) {
            self.state().exclusive = None;
            self.changed.notify_waiters();
        }

        /// Wait until no child creation is in flight. New ones already wait
        /// for the exclusive guard marked by [`Self::hold`].
        pub(super) async fn drain(&self) {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.state().in_flight == 0 {
                    return;
                }
                changed.await;
            }
        }

        pub(super) fn drain_blocking(&self) {
            let mut state = self.state();
            while state.in_flight > 0 {
                state = self
                    .drained
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }

        pub(super) async fn enter(&'static self) -> ChildCreation {
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                {
                    let mut state = self.state();
                    match state.exclusive {
                        None => {
                            state.in_flight += 1;
                            return ChildCreation(Some(self));
                        }
                        Some(holder) if holder.is_current() => return ChildCreation(None),
                        Some(_) => {}
                    }
                }
                changed.await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;

    use super::Gate;

    #[tokio::test]
    async fn an_in_flight_spawn_excludes_lock_acquisition_until_it_finishes() {
        // A private gate, so these observations are unaffected by whatever the
        // rest of the suite is doing to the process-wide one.
        static GATE: Gate = Gate::new();
        let (acquired, observed) = mpsc::channel();
        let spawning = GATE.spawning().await;
        // A live spawn guard admits other spawns and excludes lock acquisition.
        assert!(GATE.lock.try_read().is_ok());
        assert!(GATE.lock.try_write().is_err());
        let waiter = thread::spawn(move || {
            let exclusive = GATE.locking();
            // A live lock guard excludes every spawn, so this thread can only
            // have been released after the spawn guard above was dropped.
            assert!(GATE.lock.try_read().is_err());
            drop(exclusive);
            acquired.send(()).unwrap();
        });
        assert!(matches!(
            observed.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(spawning);
        waiter.join().unwrap();
        observed.recv().unwrap();
    }

    #[tokio::test]
    async fn a_restart_returns_its_shared_guard_without_waiting_behind_a_queued_writer() {
        // A fixture serves its successor inside the restart; the successor is
        // running from then on. A writer that queued meanwhile (another
        // fixture's restart, itself waiting for other tests) must not delay
        // the caller's shared guard.
        static GATE: Gate = Gate::new();
        let (queued, queued_seen) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let held = GATE.spawning().await;
        let restarted = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            GATE.excluding(held, async {
                let writer = tokio::spawn(async move {
                    queued.send(()).unwrap();
                    let _exclusive = GATE.locking_async().await;
                    let _ = released.await;
                });
                // On this current-thread runtime the writer has registered
                // its wait before this receiver can resume.
                queued_seen.await?;
                Ok(writer)
            }),
        )
        .await;
        let (writer, shared) = restarted
            .expect("the restart's shared guard waited behind a writer queued during it")
            .unwrap();
        assert!(GATE.lock.try_write().is_err());
        drop(shared);
        release.send(()).unwrap();
        writer.await.unwrap();
    }

    /// A fixture holding a fair shared guard across a whole open reaches a
    /// product child creation inside it. With a lock taker queued meanwhile,
    /// a second fair shared acquisition would wait forever; the child
    /// creation must not.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_creation_inside_a_shared_guard_never_waits_behind_a_queued_writer() {
        static GATE: Gate = Gate::new();
        let (queued, queued_seen) = tokio::sync::oneshot::channel();
        let shared = GATE.spawning().await;
        let writer = tokio::spawn(async move {
            queued.send(()).unwrap();
            drop(GATE.locking_async().await);
        });
        // On this current-thread runtime the writer has registered its wait
        // before this receiver can resume.
        queued_seen.await.unwrap();
        assert!(GATE.lock.try_read().is_err(), "the writer is not queued");
        let creation =
            tokio::time::timeout(std::time::Duration::from_secs(5), GATE.creations.enter())
                .await
                .expect("a nested child creation waited behind a queued writer");
        drop(creation);
        drop(shared);
        writer.await.unwrap();
    }

    /// A lock taker returns only after every child creation in flight has
    /// finished, and while it holds the gate another test's child creation
    /// waits; the holder's own child creation does not.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_lock_taker_excludes_child_creations_of_other_tests() {
        static GATE: Gate = Gate::new();
        let creation = GATE.creations.enter().await;
        let (acquired, mut acquired_seen) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        // Another test: its own thread and runtime.
        let taker = thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let exclusive = GATE.locking_async().await;
                    // The taker's own child creation is already excluded.
                    drop(GATE.creations.enter().await);
                    acquired.send(()).unwrap();
                    released.await.unwrap();
                    drop(exclusive);
                });
        });
        let early =
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut acquired_seen).await;
        assert!(
            early.is_err(),
            "the lock taker overlapped a child creation in flight"
        );
        drop(creation);
        tokio::time::timeout(std::time::Duration::from_secs(5), acquired_seen)
            .await
            .expect("the lock taker never acquired after the child creation ended")
            .unwrap();
        // This test is not the holder now: its child creation waits.
        let mut waiting = Box::pin(GATE.creations.enter());
        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut waiting).await;
        assert!(
            blocked.is_err(),
            "a child creation overlapped the lock taker"
        );
        release.send(()).unwrap();
        let creation = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("a child creation stayed excluded after the lock taker released");
        drop(creation);
        taker.join().unwrap();
    }

    /// The test cache warm-up (`test_support::warm_runtime_cache` and every
    /// form that calls it) holds a shared guard while it provisions and
    /// builds. The gate is fair, so a first warm-up started under a caller's
    /// own guard, with a writer queued, would wait behind that writer while
    /// the writer waits for the caller. A first warm-up under the caller's
    /// exclusive guard waits for that guard itself. A `provision(` call under
    /// the exclusive guard can run a version probe on its own thread, whose
    /// child creation waits for that guard too (see the module doc).
    /// Fixtures therefore warm before they take a guard; this scan of the
    /// crate's sources finds a warm-up call written while a guard bound in
    /// the same scope is still held, and a `provision(` call written while an
    /// exclusive guard is.
    #[test]
    fn no_spawn_guard_encloses_a_test_cache_warm_up() {
        let mut violations = Vec::new();
        let (mut guards_seen, mut exclusive_seen) = (0, 0);
        for file in &crate_sources() {
            let source = code_only(&std::fs::read_to_string(file).unwrap());
            let scanned = warm_ups_under_guards(&file.display().to_string(), &source);
            guards_seen += scanned.spawning;
            exclusive_seen += scanned.exclusive;
            violations.extend(scanned.violations);
        }
        println!(
            "{guards_seen} spawn guards and {exclusive_seen} exclusive guards scanned, {} \
             violations",
            violations.len()
        );
        assert!(
            guards_seen > 50,
            "the scan found only {guards_seen} spawn guards"
        );
        assert!(
            exclusive_seen > 20,
            "the scan found only {exclusive_seen} exclusive guards"
        );
        assert!(
            violations.is_empty(),
            "warm the test cache before taking a spawn guard:\n{}",
            violations.join("\n")
        );
    }

    /// What [`warm_ups_under_guards`] found in one source.
    struct WarmUpScan {
        spawning: usize,
        exclusive: usize,
        violations: Vec<String>,
    }

    /// The warm-ups `source` (already passed through [`code_only`]) writes
    /// while a guard bound by `let` in an enclosing scope is still held, and
    /// the `provision(` calls it writes while an exclusive guard is.
    fn warm_ups_under_guards(label: &str, source: &str) -> WarmUpScan {
        // Direct warm-ups, then the helpers that warm first.
        const WARM: [&str; 10] = [
            "warm_runtime_cache(",
            ".warmed()",
            "warmed_open_options(",
            "warmed_cache_dir(",
            "warm_blocking(",
            "temporary()",
            "temporary_cold()",
            "open_temporary(",
            "spawn_logged_owner(",
            "cache_dir()",
        ];
        // Calls that can probe the engine version on a thread of its own,
        // flagged under an exclusive guard only.
        const PROBES: [&str; 1] = ["provision("];
        let mut scan = WarmUpScan {
            spawning: 0,
            exclusive: 0,
            violations: Vec::new(),
        };
        // (binding, brace depth, line, exclusive) of each guard still held.
        let mut held: Vec<(String, usize, usize, bool)> = Vec::new();
        let mut depth = 0usize;
        let mut line = 1;
        let bytes = source.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            let rest = &source[index..];
            match byte {
                b'\n' => line += 1,
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    held.retain(|(_, at, _, _)| *at <= depth);
                }
                _ => {}
            }
            let boundary = index == 0
                || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
            let exclusive = rest.starts_with("spawn_gate::locking");
            if exclusive || rest.starts_with("spawn_gate::spawning") {
                if exclusive {
                    scan.exclusive += 1;
                } else {
                    scan.spawning += 1;
                }
                let start = source[..index].rfind('\n').map_or(0, |at| at + 1);
                let prefix = source[start..index].trim_start();
                if let Some(binding) = prefix.strip_prefix("let ").and_then(|after| {
                    after
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                }) {
                    held.push((binding.to_owned(), depth, line, exclusive));
                }
            } else if let Some(after) = rest.strip_prefix("drop(") {
                let binding: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                held.retain(|(name, _, _, _)| *name != binding);
            } else if let Some((call, probe)) = WARM
                .iter()
                .map(|warm| (warm, false))
                .chain(PROBES.iter().map(|probe| (probe, true)))
                .find(|&(call, probe)| rest.starts_with(*call) && (!probe || boundary))
            {
                let start = source[..index].rfind('\n').map_or(0, |at| at + 1);
                let defining = source[start..index].contains("fn ");
                let guard = held
                    .iter()
                    .rev()
                    .find(|(_, _, _, exclusive)| *exclusive || !probe);
                if !defining && let Some((name, _, at, exclusive)) = guard {
                    let kind = if *exclusive { "exclusive" } else { "spawn" };
                    scan.violations.push(format!(
                        "{label}:{line}: {call} while the {kind} guard `{name}` from line {at} is held"
                    ));
                }
            }
        }
        scan
    }

    /// The warm-up scan reports a warm-up under either guard and a
    /// `provision(` call under an exclusive guard, and accepts a
    /// `provision(` call under a shared guard, a call whose name only ends
    /// in `provision(`, and a call after the guard's drop or scope.
    #[test]
    fn the_warm_up_scan_reports_calls_under_a_held_guard() {
        let source = code_only(concat!(
            "async fn shared() {\n",
            "    let _gate = crate::spawn_gate::spawning().await;\n",
            "    crate::test_support::warm_runtime_cache().await;\n",
            "    crate::provision::provision(&config, &cache).await;\n",
            "}\n",
            "async fn exclusive() {\n",
            "    let gate = crate::spawn_gate::locking_async().await;\n",
            "    let store = MemoryStore::temporary().await;\n",
            "    crate::provision::provision(&config, &cache).await;\n",
            "    gated_provision(&config).await;\n",
            "    drop(gate);\n",
            "    crate::provision::provision(&config, &cache).await;\n",
            "    {\n",
            "        let _scoped = crate::spawn_gate::locking();\n",
            "    }\n",
            "    crate::provision::provision(&config, &cache).await;\n",
            "}\n",
        ));
        let scan = warm_ups_under_guards("src/sample.rs", &source);
        assert_eq!((scan.spawning, scan.exclusive), (1, 2));
        let lines: Vec<_> = scan
            .violations
            .iter()
            .map(|violation| violation.split(':').nth(1).unwrap())
            .collect();
        assert_eq!(lines, ["3", "8", "9"], "{:?}", scan.violations);
        assert!(scan.violations[0].contains("the spawn guard `_gate`"));
        assert!(scan.violations[2].contains("provision( while the exclusive guard `gate`"));
    }

    /// Every Rust source file of this crate, sorted.
    fn crate_sources() -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        let mut pending = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }

    /// Constructions of a child process. `isolated_command` is the crate's
    /// one command builder; its callers spawn what it returns.
    const CHILD_CONSTRUCTIONS: [&str; 3] = [
        "Command::new(",
        "NativeSpawnSpec::new(",
        "isolated_command(",
    ];

    /// Any of these in the enclosing function body counts as the gate.
    const GATE_TOKENS: [&str; 4] = [
        "spawn_gate::spawning",
        "spawn_gate::child_creation",
        "spawn_gate::locking",
        "spawn_gate::excluding_spawns",
    ];

    /// Functions that build a child process without spawning it, keyed by
    /// path suffix and name; their callers are scanned instead.
    const BUILDERS: [(&str, &str); 1] = [("provision.rs", "isolated_command")];

    /// Windows-only sources, where `flock`/`posix_spawn` do not apply and no
    /// test takes the gate (see the module doc).
    const WINDOWS_ONLY: [&str; 3] = [
        "server/windows_fixture.rs",
        "server/windows_tests.rs",
        "test_support/windows.rs",
    ];

    /// The child-process constructions in `source` (already passed through
    /// [`code_only`]) whose innermost enclosing function takes no gate
    /// guard, and the number of constructions seen. The check is per
    /// function, not per statement: a gate token anywhere in the enclosing
    /// function body satisfies it, so a reviewer still checks that the guard
    /// spans the spawn itself.
    fn ungated_child_constructions(label: &str, source: &str) -> (usize, Vec<String>) {
        if WINDOWS_ONLY.iter().any(|suffix| label.ends_with(suffix)) {
            return (0, Vec::new());
        }
        let bytes = source.as_bytes();
        // (name, keyword index, body open, depth at open) of open bodies.
        let mut open: Vec<(String, usize, usize, usize)> = Vec::new();
        // (name, keyword index, body open, body close) of closed bodies.
        let mut bodies: Vec<(String, usize, usize, usize)> = Vec::new();
        // A signature seen but its body not yet opened, and the parentheses
        // and brackets open within it (an array type's `;` is not an end).
        let mut pending: Option<(String, usize)> = None;
        let mut nesting = 0usize;
        let mut depth = 0usize;
        for (index, byte) in bytes.iter().enumerate() {
            let rest = &source[index..];
            let boundary = index == 0
                || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
            if boundary && rest.starts_with("fn ") {
                let name: String = rest[3..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                pending = Some((name, index));
                nesting = 0;
            }
            match byte {
                b'(' | b'[' => nesting += 1,
                b')' | b']' => nesting = nesting.saturating_sub(1),
                b';' if nesting == 0 => pending = None,
                b'{' => {
                    if let Some((name, keyword)) = pending.take() {
                        open.push((name, keyword, index, depth));
                    }
                    depth += 1;
                }
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if open.last().is_some_and(|(_, _, _, at)| *at == depth) {
                        let (name, keyword, start, _) = open.pop().unwrap();
                        bodies.push((name, keyword, start, index));
                    }
                }
                _ => {}
            }
        }
        let mut seen = 0;
        let mut violations = Vec::new();
        for (index, _) in source.char_indices() {
            let rest = &source[index..];
            let Some(construction) = CHILD_CONSTRUCTIONS
                .iter()
                .find(|construction| rest.starts_with(*construction))
            else {
                continue;
            };
            let boundary = index == 0 || {
                let before = source.as_bytes()[index - 1];
                !(before.is_ascii_alphanumeric() || before == b'_')
            };
            // The builder's own definition is not a construction.
            if !boundary || source[..index].trim_end().ends_with("fn") {
                continue;
            }
            seen += 1;
            let line = source[..index].matches('\n').count() + 1;
            let Some((name, keyword, start, end)) = bodies
                .iter()
                .filter(|(_, _, start, end)| *start < index && index < *end)
                .max_by_key(|(_, _, start, _)| *start)
            else {
                violations.push(format!(
                    "{label}:{line}: {construction} outside any function"
                ));
                continue;
            };
            let body = &source[*start..*end];
            let attributes_from = source[..*keyword]
                .rfind(['}', ';', '{'])
                .map_or(0, |at| at + 1);
            let windows_only = source[attributes_from..*keyword].contains("cfg(windows)");
            let builder = BUILDERS
                .iter()
                .any(|(suffix, builder)| label.ends_with(suffix) && name == builder);
            if !windows_only && !builder && !GATE_TOKENS.iter().any(|gate| body.contains(gate)) {
                violations.push(format!(
                    "{label}:{line}: {construction} in `fn {name}`, which takes no spawn gate guard"
                ));
            }
        }
        (seen, violations)
    }

    /// Every function in this crate that constructs a child process takes a
    /// spawn gate guard (or is a Windows-only or builder function). A
    /// construction without one would let a sibling test's child keep a
    /// just-released advisory lock held while a lock taker holds the
    /// exclusive guard, which is the window the gate closes.
    #[test]
    fn every_child_creation_takes_the_gate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut seen = 0;
        let mut violations = Vec::new();
        for file in crate_sources() {
            let label = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            let source = code_only(&std::fs::read_to_string(&file).unwrap());
            let (found, ungated) = ungated_child_constructions(&label, &source);
            seen += found;
            violations.extend(ungated);
        }
        println!(
            "{seen} child constructions scanned, {} ungated",
            violations.len()
        );
        assert!(seen > 15, "the scan found only {seen} child constructions");
        assert!(
            violations.is_empty(),
            "hold a spawn gate guard across child creation; see `crate::spawn_gate`:\n{}",
            violations.join("\n")
        );
    }

    /// The scan reports an ungated construction, including one in a helper
    /// nested inside a gated function, and accepts the exemptions.
    #[test]
    fn the_child_creation_scan_reports_an_ungated_construction() {
        let source = code_only(concat!(
            "fn ungated() {\n",
            "    let child = std::process::Command::new(\"/bin/true\").spawn();\n",
            "}\n",
            "async fn gated() {\n",
            "    let _creation = crate::spawn_gate::child_creation().await;\n",
            "    let child = tokio::process::Command::new(\"/bin/true\").spawn();\n",
            "    fn nested() { let _ = isolated_command(binary, home).spawn(); }\n",
            "}\n",
            "#[cfg(windows)]\n",
            "async fn native() { let spec = NativeSpawnSpec::new(binary, cwd); }\n",
            "pub(crate) fn isolated_command(binary: &Path) -> Command {\n",
            "    Command::new(binary)\n",
            "}\n",
        ));
        let (seen, violations) = ungated_child_constructions("src/provision.rs", &source);
        assert_eq!(seen, 5, "{violations:?}");
        assert_eq!(violations.len(), 2, "{violations:?}");
        assert!(violations[0].contains(":2: Command::new( in `fn ungated`"));
        assert!(violations[1].contains(":7: isolated_command( in `fn nested`"));
        let (_, elsewhere) = ungated_child_constructions("src/service.rs", &source);
        assert!(
            elsewhere
                .iter()
                .any(|violation| violation.contains("`fn isolated_command`")),
            "the builder exemption applies only to its own file: {elsewhere:?}"
        );
    }

    /// `source` with comments, string and character literals blanked, so
    /// braces and calls are only read from code. Line breaks are kept.
    fn code_only(source: &str) -> String {
        let bytes = source.as_bytes();
        let mut out = String::with_capacity(source.len());
        let mut index = 0;
        let blank = |text: &str, out: &mut String| {
            out.extend(text.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        };
        while index < bytes.len() {
            let rest = &source[index..];
            let end = if rest.starts_with("//") {
                rest.find('\n').unwrap_or(rest.len())
            } else if rest.starts_with("/*") {
                rest.find("*/").map_or(rest.len(), |at| at + 2)
            } else if rest.starts_with("r#\"") || rest.starts_with("r\"") {
                let hashes = rest[1..].chars().take_while(|c| *c == '#').count();
                let close = format!("\"{}", "#".repeat(hashes));
                rest[2 + hashes..]
                    .find(&close)
                    .map_or(rest.len(), |at| 2 + hashes + at + close.len())
            } else if rest.starts_with('"') {
                let mut at = 1;
                while at < rest.len() && rest.as_bytes()[at] != b'"' {
                    at += if rest.as_bytes()[at] == b'\\' { 2 } else { 1 };
                }
                (at + 1).min(rest.len())
            } else if rest.starts_with('\'')
                && (rest.get(2..3) == Some("'") || rest.starts_with("'\\"))
            {
                rest[1..].find('\'').map_or(1, |at| at + 2).min(4)
            } else {
                let next = rest.chars().next().map_or(1, char::len_utf8);
                out.push_str(&rest[..next]);
                index += next;
                continue;
            };
            blank(&rest[..end], &mut out);
            index += end;
        }
        out
    }

    #[test]
    fn a_panicking_holder_releases_the_gate() {
        static GATE: Gate = Gate::new();
        let holder = thread::spawn(|| {
            let _exclusive = GATE.locking();
            panic!("holder failed");
        });
        assert!(holder.join().is_err());
        drop(GATE.locking());
        drop(GATE.lock.blocking_read());
    }
}
