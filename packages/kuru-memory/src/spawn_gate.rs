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

    #[cfg(unix)]
    fn spawning_blocking(&self) -> RwLockReadGuard<'_, ()> {
        self.0.blocking_read()
    }

    async fn excluding<'a, T>(
        &'a self,
        held: RwLockReadGuard<'a, ()>,
        restart: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<(T, RwLockReadGuard<'a, ()>)> {
        drop(held);
        let exclusive = self.locking_async().await;
        let value = restart.await?;
        Ok((value, exclusive.downgrade()))
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
#[cfg(unix)]
pub(crate) fn spawning_blocking() -> RwLockReadGuard<'static, ()> {
    GATE.spawning_blocking()
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
pub(crate) async fn excluding_spawns<T>(
    held: RwLockReadGuard<'static, ()>,
    restart: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<(T, RwLockReadGuard<'static, ()>)> {
    GATE.excluding(held, restart).await
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
        assert!(GATE.0.try_write().is_err());
        drop(shared);
        release.send(()).unwrap();
        writer.await.unwrap();
    }

    /// The test cache warm-up (`test_support::warm_runtime_cache` and every
    /// form that calls it) holds a shared guard while it provisions and
    /// builds. The gate is fair, so a first warm-up started under a caller's
    /// own guard, with a writer queued, would wait behind that writer while
    /// the writer waits for the caller. Fixtures therefore warm before they
    /// take a guard; this scan of the crate's sources finds a warm-up call
    /// written while a guard bound in the same scope is still held.
    #[test]
    fn no_spawn_guard_encloses_a_test_cache_warm_up() {
        const WARM: [&str; 5] = [
            "warm_runtime_cache(",
            ".warmed()",
            "warmed_open_options(",
            "warmed_cache_dir(",
            "warm_blocking(",
        ];
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
        let mut violations = Vec::new();
        let mut guards_seen = 0;
        for file in &files {
            let source = code_only(&std::fs::read_to_string(file).unwrap());
            // (binding, brace depth, line) of each guard still held.
            let mut held: Vec<(String, usize, usize)> = Vec::new();
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
                        held.retain(|(_, at, _)| *at <= depth);
                    }
                    _ => {}
                }
                if rest.starts_with("spawn_gate::spawning") {
                    guards_seen += 1;
                    let start = source[..index].rfind('\n').map_or(0, |at| at + 1);
                    let prefix = source[start..index].trim_start();
                    if let Some(binding) = prefix.strip_prefix("let ").and_then(|after| {
                        after
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .next()
                    }) {
                        held.push((binding.to_owned(), depth, line));
                    }
                } else if let Some(after) = rest.strip_prefix("drop(") {
                    let binding: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    held.retain(|(name, _, _)| *name != binding);
                } else if let Some(warm) = WARM.iter().find(|warm| rest.starts_with(*warm)) {
                    let start = source[..index].rfind('\n').map_or(0, |at| at + 1);
                    let defining = source[start..index].contains("fn ");
                    if !defining && let Some((name, _, at)) = held.last() {
                        violations.push(format!(
                            "{}:{line}: {warm} while the spawn guard `{name}` from line {at} is held",
                            file.display()
                        ));
                    }
                }
            }
        }
        assert!(
            guards_seen > 50,
            "the scan found only {guards_seen} spawn guards"
        );
        assert!(
            violations.is_empty(),
            "warm the test cache before taking a spawn guard:\n{}",
            violations.join("\n")
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
        drop(GATE.0.blocking_read());
    }
}
