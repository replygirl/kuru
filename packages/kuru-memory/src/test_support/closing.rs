//! A test body's teardown scope: every local store the body opens is closed
//! on every exit path, then the test must have no unawaited supervisor.
//!
//! A store dropped without [`crate::MemoryStore::close`] hands its Dolt supervisor
//! to a detached reaper thread. That supervisor sits outside the test's
//! process group, stops Dolt on its own, and can exit after the test process,
//! writing its coverage profile after the partition's tests. A teardown that
//! is the body's last statement runs only on success, so an assertion
//! failure, a panic or an early `?` return still drops the store live.
//! [`closing`] runs the body, then closes what it opened, whatever the
//! outcome.

use crate::server::Server;
use futures::FutureExt;
use std::{
    cell::RefCell,
    future::Future,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
};

thread_local! {
    /// The server handles of the active scope's stores, on the test's own
    /// thread. `#[tokio::test]` runs a current-thread runtime there, so tasks
    /// it spawns share it.
    static SCOPE: RefCell<Option<Vec<Server>>> = const { RefCell::new(None) };
}

/// Retain a newly opened local store's server handle while a scope is
/// active on this thread. Only the server is retained: its supervisor stays
/// owned until the scope closes it, while the store's views, pools and
/// fixture permit are released as the test drops them. Outside a scope,
/// nothing is retained and the store behaves as before.
pub(crate) fn register(server: Server) {
    SCOPE.with(|scope| {
        if let Some(servers) = scope.borrow_mut().as_mut() {
            servers.push(server);
        }
    });
}

/// Ends the scope even if the [`closing`] future itself is dropped.
struct Active;

impl Active {
    fn enter() -> Self {
        SCOPE.with(|scope| {
            let mut scope = scope.borrow_mut();
            assert!(scope.is_none(), "memory test teardown scopes do not nest");
            *scope = Some(Vec::new());
        });
        Self
    }

    fn take(&self) -> Vec<Server> {
        SCOPE.with(|scope| scope.borrow_mut().replace(Vec::new()).unwrap_or_default())
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        // `try_with`: the thread's locals may already be gone at exit.
        let _ = SCOPE.try_with(|scope| scope.borrow_mut().take());
    }
}

/// Run a test body, then close every local [`crate::MemoryStore`] it opened, on
/// success, on an early return and after a panic. Closing is idempotent, so
/// a store the body already closed is unaffected. A panic is resumed once
/// the stores are closed. Otherwise the test then fails, by name, if any
/// Dolt supervisor it started is still unreaped or was dropped without a
/// close (see [`super::unawaited_supervisors`]).
///
/// Use it as the whole body of an async test:
/// `kuru_memory::test_support::closing(async { ... }).await`.
pub async fn closing<T>(body: impl Future<Output = T>) -> T {
    let active = Active::enter();
    // The body, and every handle it holds, is dropped before the close.
    let outcome = AssertUnwindSafe(body).catch_unwind().await;
    let mut failures = Vec::new();
    // A close never opens a store, but take until none remain regardless.
    loop {
        let stores = active.take();
        if stores.is_empty() {
            break;
        }
        for server in stores {
            if let Err(error) = server.close().await {
                failures.push(format!("{error:#}"));
            }
        }
    }
    drop(active);
    let value = match outcome {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    assert!(
        failures.is_empty(),
        "closing the test's memory stores failed: {failures:#?}"
    );
    let unawaited = super::unawaited_supervisors(&super::test_supervisors());
    assert!(
        unawaited.is_empty(),
        "test {} returned without awaiting every Dolt supervisor it started; open its \
         stores inside `closing` and close any it hands elsewhere: {unawaited:#?}",
        super::lifecycle_trace::label()
    );
    value
}

/// The async tests in `source` whose body is not exactly one [`closing`]
/// scope, as `line: name`. A body outside the scope can drop a live store,
/// whose supervisor then outlives the test process and writes a late
/// coverage profile.
pub fn tests_outside_closing(source: &str) -> Vec<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut unscoped = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.trim_start().starts_with("#[tokio::test") {
            continue;
        }
        let Some(signature) = (index..lines.len()).find(|&at| lines[at].contains("async fn "))
        else {
            unscoped.push(format!("{}: no async fn after #[tokio::test]", index + 1));
            continue;
        };
        let name = lines[signature]
            .split("async fn ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '<']).next())
            .unwrap_or_default();
        let opened = (signature..lines.len()).find(|&at| lines[at].trim_end().ends_with('{'));
        let scoped = opened
            .and_then(|at| lines.get(at + 1))
            .is_some_and(|first| {
                first
                    .trim_start()
                    .starts_with("kuru_memory::test_support::closing(")
            });
        if !scoped {
            unscoped.push(format!("{}: {name}", signature + 1));
        }
    }
    unscoped
}

/// The Rust sources under `directory`, recursively and sorted.
pub fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Fail, naming each one as `path:line: name` relative to `root`, if any
/// async test in `sources` does not run its body in [`closing`], or if
/// `sources` hold no async test at all. A crate's guard test calls this
/// with the sources whose tests open memory stores, so a forgotten teardown
/// fails locally and by name instead of in a coverage partition's export.
pub fn assert_async_tests_run_in_closing(root: &Path, sources: &[PathBuf]) {
    let mut scanned = 0;
    let mut unscoped = Vec::new();
    for path in sources {
        let source = std::fs::read_to_string(path).unwrap();
        scanned += source.matches("#[tokio::test").count();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        unscoped.extend(
            tests_outside_closing(&source)
                .into_iter()
                .map(|test| format!("{relative}:{test}")),
        );
    }
    assert!(scanned > 0, "found no async tests under {}", root.display());
    assert!(
        unscoped.is_empty(),
        "run each async test's body in `kuru_memory::test_support::closing(async {{ ... }}).await`, \
         so every store it opens is closed on every exit path: {unscoped:#?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use crate::test_support::{supervisor_mark, unawaited_supervisors};

    fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn a_store_the_body_drops_is_closed_and_awaited() {
        let mark = supervisor_mark();
        let returned = closing(async {
            let store = MemoryStore::temporary().await.unwrap();
            drop(store);
            7
        })
        .await;
        assert_eq!(returned, 7);
        assert_eq!(unawaited_supervisors(&mark), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_panicking_body_still_closes_its_store_and_resumes_the_panic() {
        let mark = supervisor_mark();
        let panic = AssertUnwindSafe(closing(async {
            let _store = MemoryStore::temporary().await.unwrap();
            panic!("assertion failed before teardown");
        }))
        .catch_unwind()
        .await
        .expect_err("the body's panic is resumed");
        assert_eq!(panic_message(&*panic), "assertion failed before teardown");
        assert_eq!(unawaited_supervisors(&mark), Vec::<String>::new());
    }

    #[tokio::test]
    async fn an_early_error_return_still_closes_its_store() {
        let mark = supervisor_mark();
        let returned: anyhow::Result<()> = closing(async {
            let _store = MemoryStore::temporary().await?;
            anyhow::bail!("early return before teardown")
        })
        .await;
        assert_eq!(
            returned.unwrap_err().to_string(),
            "early return before teardown"
        );
        assert_eq!(unawaited_supervisors(&mark), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_closed_store_is_closed_again_without_error() {
        closing(async {
            let store = MemoryStore::temporary().await.unwrap();
            store.close().await.unwrap();
        })
        .await;
    }

    /// The scope retains only each store's server: a store the test closed
    /// and dropped returns its fixture permit (four per process), so a loop
    /// can open more stores than that within one scope.
    #[tokio::test]
    async fn closed_stores_in_a_scope_release_their_fixture_permits() {
        let mark = supervisor_mark();
        closing(async {
            for _ in 0..5 {
                let opened = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    MemoryStore::temporary(),
                )
                .await
                .expect("a closed store in the scope kept its fixture permit")
                .unwrap();
                opened.close().await.unwrap();
            }
        })
        .await;
        assert_eq!(unawaited_supervisors(&mark), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_supervisor_outside_the_scope_fails_the_test_by_name() {
        // Opened before the scope, so the scope neither retains nor closes it.
        let outside = MemoryStore::temporary().await.unwrap();
        let panic = AssertUnwindSafe(closing(async {}))
            .catch_unwind()
            .await
            .expect_err("an unreaped supervisor fails the scope");
        let message = panic_message(&*panic);
        assert!(
            message.contains(
                "test test_support::closing::tests::a_supervisor_outside_the_scope_fails_the_test_by_name"
            ) && message.contains("has not been reaped"),
            "{message}"
        );
        outside.close().await.unwrap();
    }

    #[tokio::test]
    async fn scopes_do_not_nest_and_end_when_dropped() {
        let outer = std::pin::pin!(closing(async {
            let nested = AssertUnwindSafe(closing(async {})).catch_unwind().await;
            assert!(nested.is_err(), "a nested scope is refused");
        }));
        outer.await;
        // A scope future dropped before it completes leaves no scope behind.
        drop(closing(async {}));
        let dropped = closing(std::future::pending::<()>());
        let mut dropped = Box::pin(dropped);
        assert!(futures::poll!(dropped.as_mut()).is_pending());
        drop(dropped);
        closing(async {}).await;
    }

    #[test]
    fn the_closing_scan_names_a_test_outside_the_scope() {
        let source = "\
@test]
async fn scoped() {
    kuru_memory::test_support::closing(async {
        let _ = 1;
    })
    .await
}

@test(start_paused = true)]
async fn drops_its_store() -> anyhow::Result<()> {
    let memory = MemoryStore::temporary().await?;
    drop(memory);
    Ok(())
}
";
        // Assembled here, so a crate guard that scans this file does not read it as a test.
        let source = source.replace("@test", "#[tokio::test");
        assert_eq!(tests_outside_closing(&source), ["10: drops_its_store"]);
    }
}
