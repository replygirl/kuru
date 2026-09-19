## 1. Test-only spawn gate

- [x] 1.1 Add a `#[cfg(test)]` `spawn_gate` module to the `kuru-memory` crate
      (declared in `lib.rs`) owning one `RwLock<()>`, with exclusive
      `locking`/`locking_async` helpers for advisory-lock acquisition/release
      and shared `spawning`/`spawning_blocking` helpers for child-process
      creation, documented with the `flock` open-file-description and
      `posix_spawn` descriptor-table rationale, and verify it compiles out of
      non-test builds (`cargo build -p kuru-memory` has no `spawn_gate`
      symbol).
- [x] 1.2 Add a deterministic regression test in `spawn_gate.rs` that injects
      the observation instead of timing it, using a private `Gate` instance:
      while the shared guard is held, a thread blocked on the exclusive guard
      has demonstrably not acquired it, and it acquires only after the shared
      guard is released; verify the test is ordering-sensitive and never
      wall-clock dependent.
- [x] 1.3 Take the exclusive guard around every raw advisory-lock
      acquisition/release in `server_tests.rs` and the one raw-lock test in
      `store/migration_lifecycle_tests.rs`
      (`interrupted_migration_close_handoff_retains_guard_until_supervisor_quiesces`),
      and the shared guard around every direct child-process spawn in
      `server_tests.rs` (`/bin/sh`, `crate::engine::spawn`,
      `supervise`/`supervise_with_port_hook`, `Server::open`,
      `crate::provision::provision`); verify no assertion, timeout, retry or
      `#[ignore]` was introduced.
- [x] 1.4 Add `crate::test_support::spawn_gated_open` (`#[cfg(test)]`,
      wrapping `MemoryStore::open` with the shared guard) and mechanically
      replace every `MemoryStore::open(` call in `store/recovery_tests.rs`,
      `store/migration_lifecycle_tests.rs` and `store/operational_gc_tests.rs`
      with it; verify every call site was rewritten (`grep -rn
      "MemoryStore::open(" packages/kuru-memory/src/store/*.rs` matches only
      the one pre-existing doc-comment reference) and none of those files'
      assertions changed.
- [x] 1.5 Add `gated_cache_lock`/`gated_verify_version` (`#[cfg(test)]`,
      wrapping `cache_lock`/`verify_version` with the exclusive/shared guard
      respectively) in `provision/native_tests.rs` and mechanically replace
      every `cache_lock(`/`verify_version(` call in that file with them;
      verify the fixture and test names themselves were not corrupted by the
      mechanical rewrite.
- [x] 1.6 Give the previously bare final assertion in
      `supervisor_rejects_bad_configuration_and_parent_eof_without_spawning`
      (`server_tests.rs`) an `{error:#}` diagnostic, matching its sibling
      assertion two lines above it.
- [x] 1.7 Run `mise run format:check`, `mise run //packages/kuru-memory:lint`,
      `mise run typecheck`, `mise run //packages/kuru-memory:test` (with the
      real bundled engine) to completion, then run the `server::` and
      `provision::` test subsets at least two more times each; record the
      observed pass/fail evidence for every run, naming which side
      (locking/spawning) each rerun exercises. This is the regression check:
      the target test failed intermittently before this change and must pass
      consistently across all reruns after it.

      Observed: `format:check` pass; `//packages/kuru-memory:lint` pass;
      `typecheck` (workspace) pass; `//packages/kuru-memory:test` — every
      binary "ok" (lib 155/155, main 0/0, parent-fixture 0/0, bundle_build
      6/6, memory 5/5, server_lifecycle 12/12, supervisor_snapshot 1/1,
      windows_lifecycle 0/0 — 179 tests total, 0 failed) in 262.85s.
      `cargo test -p kuru-memory --lib --all-features -- server::` (mixed
      locking+spawning side, includes the target test and every test
      gated in this change) 18/18 three consecutive times (3.71s/3.67s/3.66s).
      `cargo test -p kuru-memory --lib --all-features -- provision::`
      (locking side: `gated_cache_lock`; spawning side: `gated_verify_version`
      is Windows-only and not exercised on this host) 30/30 three consecutive
      times (7.31s/7.66s/7.72s). No failure observed in any run.
- [x] 1.8 Run `mise run cospec:validate` and `mise run cospec -- archive
      serialize-memory-test-lock-and-spawn` before the final branch commit.
