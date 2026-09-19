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
//! This module is test-only and touches no production code path: it compiles
//! only under `cfg(test)`, so it is a strict no-op on every platform,
//! including Windows, where `flock`/`posix_spawn` do not apply — the module
//! still exists there so cross-platform test code compiles uniformly, but no
//! Windows test currently needs to take either guard.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

struct Gate(RwLock<()>);

impl Gate {
    const fn new() -> Self {
        Self(RwLock::const_new(()))
    }

    fn locking(&self) -> RwLockWriteGuard<'_, ()> {
        self.0.blocking_write()
    }

    async fn locking_async(&self) -> RwLockWriteGuard<'_, ()> {
        self.0.write().await
    }

    async fn spawning(&self) -> RwLockReadGuard<'_, ()> {
        self.0.read().await
    }

    fn spawning_blocking(&self) -> RwLockReadGuard<'_, ()> {
        self.0.blocking_read()
    }
}

static GATE: Gate = Gate::new();

/// Hold across advisory-lock acquisition (or release) in a synchronous test.
/// Excludes every in-binary spawn.
pub(crate) fn locking() -> RwLockWriteGuard<'static, ()> {
    GATE.locking()
}

/// [`locking`] for an asynchronous test, which must not block its runtime.
pub(crate) async fn locking_async() -> RwLockWriteGuard<'static, ()> {
    GATE.locking_async().await
}

/// Hold across child-process creation in an asynchronous test. Shared between
/// concurrent spawners.
pub(crate) async fn spawning() -> RwLockReadGuard<'static, ()> {
    GATE.spawning().await
}

/// [`spawning`] for a synchronous test, which has no runtime to `.await` on.
pub(crate) fn spawning_blocking() -> RwLockReadGuard<'static, ()> {
    GATE.spawning_blocking()
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
        assert!(GATE.0.try_read().is_ok());
        assert!(GATE.0.try_write().is_err());
        let waiter = thread::spawn(move || {
            let exclusive = GATE.locking();
            // A live lock guard excludes every spawn, so this thread can only
            // have been released after the spawn guard above was dropped.
            assert!(GATE.0.try_read().is_err());
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

    #[test]
    fn a_panicking_holder_releases_the_gate() {
        static GATE: Gate = Gate::new();
        let holder = thread::spawn(|| {
            let _exclusive = GATE.locking();
            panic!("holder failed");
        });
        assert!(holder.join().is_err());
        drop(GATE.locking());
        drop(GATE.0.blocking_read());
    }
}
