//! Fixture roots that refuse to release a directory under a live memory owner.
//!
//! Dolt writes its final table-file manifest while it stops. Removing its data
//! directory first makes the engine panic at close (dolthub/dolt#10971), and a
//! supervisor's final `server.log` write then fails. So a fixture root is
//! released only after every Dolt it hosted has been reaped.
//!
//! [`TempDir`]'s teardown checks that invariant without waiting. Beneath its
//! root it makes one non-blocking attempt on every lock that an owner holds
//! until its Dolt is reaped:
//!
//! - `lifecycle.lock` in a store or stage directory (the Unix lifecycle lease
//!   held by the owned supervisor),
//! - `*.lock` in a `lifecycles` directory (the Windows external lifecycle
//!   lease),
//! - `*.service-owner.lock` (a managed service owner, held until its Dolt is
//!   reaped).
//!
//! If any is still held, teardown keeps the whole root, so the live engine can
//! finish, and fails the test with the fixture, the test and each live owner.
//! A thread that is already unwinding keeps the root without a second panic.
//! The check never waits, so it cannot hide a missing retirement: fixtures
//! close their stores, or call [`super::await_managed_quiescence`] for managed
//! services, before the root drops. It writes nothing to stdout or stderr
//! itself, and skips the check while the thread is already unwinding.

use crate::files::PrivateTemp;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 4096;
const ENDPOINT_LIMIT: u64 = 64 * 1024;

/// A private fixture root, checked for live memory owners before removal.
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
        let owners = live_owners(inner.path());
        if owners.is_empty() {
            return;
        }
        // Never delete the tree under a live engine, even while this test is
        // already failing; only a thread that is not unwinding reports it.
        let kept = inner.keep();
        if std::thread::panicking() {
            return;
        }
        panic!(
            "fixture root {} (created by test {}) was released while a memory owner is \
             still alive, so it is kept in place for the engine to stop. Close every store \
             and retire managed services (test_support::await_managed_quiescence) before \
             the root drops. Live owners: {}",
            kept.display(),
            self.label,
            owners.join("; ")
        );
    }
}

/// Every lock beneath `root` whose owner is still alive, described for a
/// failure message. Secrets in endpoint records are never read into it.
pub(crate) fn live_owners(root: &Path) -> Vec<String> {
    let mut locks = Vec::new();
    let mut budget = MAX_ENTRIES;
    collect_locks(root, 0, &mut budget, &mut locks);
    locks
        .into_iter()
        .filter_map(|(path, kind)| held(&path).map(|how| describe(&path, kind, &how)))
        .collect()
}

#[derive(Clone, Copy)]
enum LockKind {
    Lifecycle,
    WindowsLifecycle,
    ServiceOwner,
}

fn classify(path: &Path) -> Option<LockKind> {
    let name = path.file_name()?.to_str()?;
    if name == "lifecycle.lock" {
        return Some(LockKind::Lifecycle);
    }
    if name.ends_with(".service-owner.lock") {
        return Some(LockKind::ServiceOwner);
    }
    let parent = path.parent()?.file_name()?;
    (parent == "lifecycles" && name.ends_with(".lock")).then_some(LockKind::WindowsLifecycle)
}

fn collect_locks(
    directory: &Path,
    depth: usize,
    budget: &mut usize,
    locks: &mut Vec<(PathBuf, LockKind)>,
) {
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
            collect_locks(&path, depth + 1, budget, locks);
        } else if kind.is_file()
            && let Some(lock) = classify(&path)
        {
            locks.push((path, lock));
        }
    }
}

/// `Some(reason)` when a non-blocking exclusive attempt finds the lock held.
fn held(path: &Path) -> Option<String> {
    let file = match File::open(path) {
        Ok(file) => file,
        // A Windows owner may hold the lock file without read sharing.
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(32) => {
            return Some("lock file is open without sharing".into());
        }
        Err(_) => return None,
    };
    match file.try_lock() {
        Ok(()) => None,
        Err(std::fs::TryLockError::WouldBlock) => Some("lock is held".into()),
        Err(std::fs::TryLockError::Error(_)) => None,
    }
}

fn describe(path: &Path, kind: LockKind, how: &str) -> String {
    match kind {
        LockKind::Lifecycle => {
            let directory = path.parent().unwrap_or(path);
            format!(
                "Dolt supervisor for store {} ({}; {how}: {})",
                directory.display(),
                store_endpoint(directory),
                path.display()
            )
        }
        LockKind::WindowsLifecycle => {
            format!(
                "Dolt supervisor lifecycle lease ({how}: {})",
                path.display()
            )
        }
        LockKind::ServiceOwner => format!(
            "managed memory service owner ({}; {how}: {})",
            service_endpoint(path),
            path.display()
        ),
    }
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let bytes = crate::files::read_bytes(path, ENDPOINT_LIMIT).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The supervised server's published instance and loopback port.
fn store_endpoint(directory: &Path) -> String {
    read_json(&directory.join("endpoint.json")).map_or_else(
        || "no published endpoint".into(),
        |endpoint| {
            format!(
                "instance {} on 127.0.0.1:{}",
                endpoint["instance"].as_str().unwrap_or("?"),
                endpoint["port"]
                    .as_u64()
                    .map_or_else(|| "?".into(), |port| port.to_string())
            )
        },
    )
}

/// The service's published transport address only; its authority record
/// carries a connection secret that is never read into a message.
fn service_endpoint(lock: &Path) -> String {
    let endpoint = lock
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".service-owner.lock"))
        .zip(lock.parent().and_then(Path::parent))
        .and_then(|(hash, memory)| {
            read_json(&memory.join("services").join(hash).join("endpoint.json"))
        });
    endpoint
        .as_ref()
        .and_then(|record| record["address"].as_str())
        .map_or_else(
            || "no published endpoint".into(),
            |address| format!("address {address}"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_only_owner_locks() {
        let kinds = [
            ("a/lifecycle.lock", Some("lifecycle")),
            ("memory/locks/abc.service-owner.lock", Some("service")),
            ("memory/locks/abc.service-start.lock", None),
            ("memory/lifecycles/00ff.lock", Some("windows")),
            ("profile/kuru-test-supervisors/prepare.lock", None),
            ("cache/versions/install.lock", None),
        ];
        for (path, expected) in kinds {
            let kind = classify(Path::new(path)).map(|kind| match kind {
                LockKind::Lifecycle => "lifecycle",
                LockKind::WindowsLifecycle => "windows",
                LockKind::ServiceOwner => "service",
            });
            assert_eq!(kind, expected, "{path}");
        }
    }

    #[test]
    fn a_held_owner_lock_keeps_the_root_and_names_the_test_and_owner() {
        // This test acquires real flocks; see `crate::spawn_gate`.
        let _gate = crate::spawn_gate::locking();
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        for directory in [
            "memory",
            "memory/abcdef",
            "memory/locks",
            "memory/services",
            "memory/services/abcdef",
        ] {
            crate::files::private_dir(&root.path().join(directory)).unwrap();
        }
        let store = root.path().join("memory/abcdef");
        let locks = root.path().join("memory/locks");
        let services = root.path().join("memory/services/abcdef");
        // Records are private files, as the product publishes them.
        crate::files::write(
            &store.join("endpoint.json"),
            br#"{"instance":"fixture-instance","port":4242}"#,
        )
        .unwrap();
        crate::files::write(
            &services.join("endpoint.json"),
            br#"{"authority":{"connection_secret":"never-printed"},"address":"kuru-fixture.sock"}"#,
        )
        .unwrap();
        let lifecycle = File::create(store.join("lifecycle.lock")).unwrap();
        let owner = File::create(locks.join("abcdef.service-owner.lock")).unwrap();
        File::create(locks.join("abcdef.service-start.lock"))
            .unwrap()
            .lock()
            .unwrap();
        assert!(
            live_owners(root.path()).is_empty(),
            "unlocked files are not owners"
        );
        lifecycle.lock().unwrap();
        owner.lock().unwrap();

        let path = root.path().to_path_buf();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(root)))
            .expect_err("a live owner must fail teardown");
        let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(path.exists(), "a violating root must be kept: {message}");
        for expected in [
            path.display().to_string(),
            "a_held_owner_lock_keeps_the_root_and_names_the_test_and_owner".into(),
            "instance fixture-instance on 127.0.0.1:4242".into(),
            "managed memory service owner (address kuru-fixture.sock".into(),
        ] {
            assert!(
                message.contains(&expected),
                "{expected:?} missing: {message}"
            );
        }
        assert!(!message.contains("never-printed"), "{message}");
        assert!(!message.contains("service-start"), "{message}");

        drop((lifecycle, owner));
        assert!(live_owners(&path).is_empty());
        let container = path.parent().unwrap().to_path_buf();
        fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn released_owner_locks_remove_the_root_quietly() {
        let _gate = crate::spawn_gate::locking();
        let root = TempDir::new("kuru-fixture-invariant-", None).unwrap();
        let store = root.path().join("memory/abcdef");
        fs::create_dir_all(&store).unwrap();
        let lifecycle = File::create(store.join("lifecycle.lock")).unwrap();
        lifecycle.lock().unwrap();
        drop(lifecycle);
        let path = root.path().to_path_buf();
        drop(root);
        assert!(!path.exists());
    }
}
