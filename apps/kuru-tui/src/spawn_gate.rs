//! Serialises advisory-lock acquisition against child-process creation inside
//! this test binary.
//!
//! `trust::lock` acquires an advisory lock with `flock(LOCK_EX | LOCK_NB)` on
//! Unix. `flock` ownership belongs to the *open file description*, not to a
//! descriptor or a process, so any duplicate of that description anywhere keeps
//! the lock held until the last duplicate closes. `posix_spawn` copies the whole
//! parent descriptor table into the new process and only closes `FD_CLOEXEC`
//! descriptors at image activation, which is strictly after the copy — so
//! `O_CLOEXEC` does not close that window. A thread spawning a child while
//! another thread releases a lock file therefore leaves the advisory lock held
//! by the child's transient duplicate, and the next `try_lock` on a freshly
//! opened descriptor fails with `EWOULDBLOCK`.
//!
//! The library tests run lock-taking and process-spawning tests concurrently in
//! one binary, which is exactly that shape. Tests that acquire a store or trust
//! lock take [`locking`] (exclusive); tests that spawn a child process take
//! [`spawning`] (shared) across the spawn, so spawning tests still run
//! concurrently with one another and only exclude lock acquisition. A test that
//! spawns a child process without holding [`spawning`] reopens the window.

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
}

static GATE: Gate = Gate::new();

/// Hold across advisory-lock acquisition in a synchronous test.
/// Excludes every in-binary spawn.
pub(crate) fn locking() -> RwLockWriteGuard<'static, ()> {
    GATE.locking()
}

/// [`locking`] for an asynchronous test, which must not block its runtime.
pub(crate) async fn locking_async() -> RwLockWriteGuard<'static, ()> {
    GATE.locking_async().await
}

/// Hold across child-process creation. Shared between concurrent spawners.
pub(crate) async fn spawning() -> RwLockReadGuard<'static, ()> {
    GATE.spawning().await
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
