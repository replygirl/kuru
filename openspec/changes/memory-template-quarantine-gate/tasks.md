# Tasks

## 1. Confirm #155's fix was already present at the failing commit

- [x] 1.1 Confirm `70fa6c0a` (#155) is an ancestor of `origin/main`'s tip
      and of the exact commit CI tested for PR #133 — verify by
      `git merge-base --is-ancestor` and by reading `tests.rs` at that exact
      SHA.
      Observed: `git merge-base --is-ancestor 70fa6c0a HEAD` (HEAD =
      `0e562595`) succeeds. The tested commit was
      `5c689e6383031004e4991a565a0de4f1eef87dbe` ("Merge 0910c326... into
      0e562595..."), itself a descendant of `0e562595`.
      `git show 5c689e638...:packages/kuru-memory/src/store/creation_template/tests.rs`
      shows `template_failing_structure_is_quarantined` already calling
      `create_unspawned` at both call sites (the manifest-read-error case
      and the loop's damage case), byte-identical to the fixed version on
      `origin/main`.
- [x] 1.2 Fetch the failing job's log and record the exact failure —
      verify by reading it directly, not by re-running CI.
      Observed: `gh api --allow-escape-sequences repos/replygirl/kuru/actions/jobs/110720770956/logs`,
      job "Coverage partition (macos-latest, 1)" on run `36969597416`:
      `store::creation_template::tests::template_failing_structure_is_quarantined`
      FAILED with `Error: another format: not quarantined`; `test result:
      FAILED. 152 passed; 1 failed`. This is the "another format" case of
      the loop (the second case; the first, "another key", passed).

## 2. Trace the chokepoint that let the race recur despite #155

- [x] 2.1 Confirm `quarantine()`/`quarantine_held()` in `creation_template.rs`
      implement the documented best-effort skip (non-waiting exclusive
      `try_key_lock`, `WouldBlock` → `Quarantine::Skipped`) and that
      `create_unspawned`'s `spawn_gate::locking_async()` guard spans the
      whole `create_in` call that reaches it — verify by reading both
      functions.
      Observed: `creation_template.rs::quarantine` (~line 1385) calls
      `try_key_lock(root, key, Mode::Exclusive)`, treating `Ok(None)`
      (`WouldBlock`, per `try_key_lock`'s `Mode` match ~line 394-400) as
      `skipped("the store template key lock is busy")`.
      `tests.rs::create_unspawned` (~line 542) binds
      `_gate = crate::spawn_gate::locking_async().await` for its whole
      body, including the `create_in` call that reaches `quarantine()`.
      The guard is correctly scoped; it just cannot exclude a spawn that
      never takes `spawn_gate`'s own guard.
- [x] 2.2 Enumerate every process-spawn site in `packages/kuru-memory/src`
      and classify each as gated (`spawn_gate::spawning`/`spawning_blocking`
      held across the actual `.spawn()`) or not — verify by reading each
      call site, not by trusting the module doc's fixture list.
      Observed: every spawn site under `#[cfg(test)]` test modules already
      named in `spawn_gate.rs`'s own doc comment (`server_tests.rs`,
      `store/recovery_tests.rs`, `store/migration_lifecycle_tests.rs`,
      `store/operational_gc_tests.rs`, `provision/native_tests.rs`,
      `store/engine_contract_tests.rs`, `store/template_stage_tests.rs`,
      `store/migrations/main_pool_classification_tests.rs`,
      `store/creation_template/tests.rs`'s own `spawn_child`,
      `test_support/template.rs`) takes `spawn_gate::spawning()` or
      `spawning_blocking()` immediately around its `.spawn()` call, or
      routes through `test_support::spawn_gated_open` (shared guard across
      the whole `MemoryStore::open`). One path does not: production
      `server.rs::open_inner_with_probe_delay` (unix ~line 629-657, windows
      ~line 697-730) takes `command.spawn()` under a caller-supplied
      `_test_spawn_guard: Option<RwLockReadGuard>` parameter — but that
      parameter is `None` on the path reached from `Server::open_inner` /
      `Server::open_with_guard` (server.rs ~line 542-551), which is what
      `store.rs::open_temporary` calls unconditionally. Only
      `Server::open_with_initial_probe_delay` (`#[cfg(test)]`, one
      startup-probe test) supplies a real `spawn_gate::spawning()` guard
      through that parameter.
- [x] 2.3 Confirm `warm_runtime_cache()` does not cover the per-call
      supervisor spawn — verify by reading it and its `OnceCell` caching.
      Observed: `test_support.rs::warm_engine` (~line 192) takes
      `spawn_gate::spawning()` only inside a
      `tokio::sync::OnceCell::get_or_init` closure that runs exactly once
      per process (the one-time provisioning probe). `warmed_open_options`/
      `temporary()` call `warm_runtime_cache()` before opening, but the
      open itself (the actual per-call supervisor spawn traced in 2.2) runs
      after the gate from 2.1 is long released, ungated.
- [x] 2.4 Confirm `MemoryStore::temporary()`/`temporary_cold()` are the
      call sites that reach the ungated path, and count their use across
      the crate — verify by reading `store.rs::temporary`/`temporary_cold`/
      `open_temporary` and grepping call sites.
      Observed: `store.rs::temporary` calls `Self::open_temporary` which
      calls `Server::open_with_guard` with no guard taken before or after;
      that path reaches the unconditional `open_inner_with_probe_delay`
      call traced in 2.2. `grep -rn "::temporary()\|::temporary_cold()" src`
      counts 92 call sites across `store.rs`, `facade.rs`, `service.rs`,
      and other files in the crate's test suite.
- [x] 2.5 Confirm the coverage job's test concurrency makes the race
      reachable, not merely theoretical — verify by finding the CI
      thread-count configuration.
      Observed: `packages/kuru-memory/mise.toml:56` sets
      `env.RUST_TEST_THREADS = "2"` for the coverage task (confirmed also
      at `mise.toml:157`); the job log's own env block (job
      110720770956) does not override it. Two concurrent test threads is
      enough for one ungated `temporary()`/`temporary_cold()` call on the
      other thread to race the exclusive-lock try inside this test's
      write guard.

## 3. Gate the chokepoint: default open path takes the spawn guard under cfg(test)

- [ ] 3.1 Thread a real `#[cfg(test)]` `spawn_gate::spawning()` guard
      through `server.rs::open_inner`/`Server::open_with_guard` into
      `open_inner_with_probe_delay`'s existing `_test_spawn_guard`
      parameter, instead of the hardcoded `None` at
      `open_inner`'s call site, on both the unix (~line 629-657) and
      windows (~line 697-730) arms — lint runs for the Windows target as
      well as the host. Non-test builds keep passing `None` (the parameter
      is already typed `Option<...>`, so this is additive, not a signature
      change visible outside `cfg(test)`). Drop the guard at the same
      point the existing `drop(_test_spawn_guard)` calls do, immediately
      after `command.spawn()`/`.spawn().await` returns — verify by reading
      the diff and confirming no non-`cfg(test)` line changed.
- [ ] 3.2 Confirm this does not introduce a deadlock against
      `spawn_gate.rs`'s own `WARM`-list scan: a caller that already holds
      `spawning()` (or `locking_async()`) must not call `temporary()`/
      `temporary_cold()`/`open_temporary` itself, since tokio's
      write-preferring `RwLock` would then block a nested shared acquire
      behind a queued writer — verify by re-running the existing scan
      (`no_spawn_guard_encloses_a_test_cache_warm_up` or its successor from
      4.2) and by grepping call sites of `temporary()`/`temporary_cold()`
      for an enclosing `locking_async()`/`spawning()` scope.
- [ ] 3.3 Record the throughput effect: a `locking_async()` write-guard
      holder now queues behind at most one in-flight `spawning()` spawn
      (shared-to-exclusive contention is bounded by `RUST_TEST_THREADS`),
      not an entire open's readiness wait, since the guard is held only
      across `command.spawn()` — verify by reading the guard's drop point
      relative to the readiness-wait `timeout_at` call, which must remain
      outside the guarded span.

## 4. Audit every other spawn site and extend the scan

- [ ] 4.1 List every `.spawn()`/`NativeSpawnSpec::spawn()` call in
      `packages/kuru-memory/src` outside `#[cfg(test)]` test modules
      (`grep -rn "\.spawn(" packages/kuru-memory/src`) and classify each:
      `engine.rs::spawn` (the Dolt engine process — called by the
      supervisor process itself, not reachable from the test binary's own
      process, and also called directly by `server_tests.rs` fixtures,
      which `spawn_gate.rs`'s doc already names as gated at the call
      site), `service.rs`'s supervisor/project-service launches,
      `provision.rs`'s probe spawns, `server/windows_fixture.rs`,
      `test_support/template.rs`'s stage/template builders. Record, per
      site: reachable from the `kuru-memory` test binary's own process
      or not; if reachable, gated by an enclosing `spawning()`/
      `spawning_blocking()` or by 3.1's new default-path guard, or left
      ungated with the specific reason (e.g. runs inside the supervisor's
      own process, which is not the test binary racing its own flock
      duplicates). This list becomes the PR body's per-site audit table.
- [ ] 4.2 Extend `spawn_gate.rs`'s existing scan
      (`no_spawn_guard_encloses_a_test_cache_warm_up` or a sibling test)
      so a spawn site newly added to the test binary with no guard in
      scope fails a test, not just a manual grep — verify with a
      deliberate negative fixture (a throwaway ungated `.spawn()` added
      temporarily to confirm the extended scan catches it, then removed
      before the final diff).
- [ ] 4.3 Confirm no quarantine assertion in
      `packages/kuru-memory/src/store/creation_template/tests.rs` or
      `open_tests.rs` needs to change: 3.1's fix closes the gap at the
      chokepoint, so #155's existing `create_unspawned`/write-guard
      pattern in those two files remains correct and sufficient — verify
      by re-reading both files' quarantine assertions against the sweep
      already on record in this change's prior revision (git history) and
      confirming none both (a) asserts a quarantine outcome and (b) is
      reachable through an exclusive guard it does not already hold.

## 5. Local verification

- [ ] 5.1 Run `mise run //packages/kuru-memory:test -- store::creation_template::`
      and the full `//packages/kuru-memory:test` locally and record the
      pass count (this cannot reproduce the cross-binary race on demand,
      per #155's own prior finding; it confirms no regression, not the
      race's absence) — verify by reading the test binary's own summary
      line.
- [ ] 5.2 Run `mise run format:check`, `//packages/kuru-memory:lint`,
      `//packages/kuru-memory:lint:windows`, and
      `//packages/kuru-memory:typecheck` and confirm each passes — verify
      by reading each command's exit code. Name any check not run and why.
