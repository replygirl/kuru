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

## 3. Gate every product-path child creation under cfg(test), nesting-safely

- [x] 3.1 Add `spawn_gate::child_creation()` (Unix, `cfg(test)`): it waits
      only while a lock taker holds the exclusive guard, never behind a
      queued one; it counts creations in flight and makes
      `locking()`/`locking_async()` wait for the count to drain; and it does
      not wait when the current test (thread or tokio task) holds the
      exclusive guard. Leave `spawning`/`spawning_blocking`/
      `excluding_spawns` and the tokio `RwLock` unchanged — verify with new
      unit tests: a nested creation under a shared guard with a queued
      writer completes; a lock taker waits for an in-flight creation; a
      creation by another test waits while the taker holds, and the taker's
      own does not.
      Observed: `spawn_gate.rs` adds `child_creation()` (`#[cfg(unix)]`)
      over a `creation::Creations` counter plus a holder mark; `locking()`/
      `locking_async()` now return an `Exclusive` wrapper that drains the
      counter before returning and clears the mark on drop/downgrade. The
      tokio `RwLock` and `spawning`/`spawning_blocking`/`excluding_spawns`
      bodies are unchanged. Clause-to-test map, all passing in
      `cargo test -p kuru-memory --lib spawn_gate` (8 passed, 0 failed):
      nested creation under a shared guard with a queued writer →
      `a_child_creation_inside_a_shared_guard_never_waits_behind_a_queued_writer`;
      lock taker waits for an in-flight creation →
      `an_in_flight_spawn_excludes_lock_acquisition_until_it_finishes`;
      another test's creation waits while the taker holds, and the taker's
      own creation does not → both asserted inside
      `a_lock_taker_excludes_child_creations_of_other_tests` (the holder
      runs on a spawned thread with its own runtime and its own
      `creations.enter()` completes before it signals `acquired`, which the
      test awaits under a 5 s deadline; the main test thread then shows its
      creation blocked for 200 ms and released only after the holder drops);
      the restart path →
      `a_restart_returns_its_shared_guard_without_waiting_behind_a_queued_writer`;
      panic safety → `a_panicking_holder_releases_the_gate`.
- [x] 3.2 Take `child_creation()` exactly across `command.spawn()` in
      `server.rs::open_inner_with_probe_delay` (Unix arm; dropped at the
      existing `drop(_test_spawn_guard)` point), `engine.rs::spawn` (Unix
      arm) and `service.rs::spawn_service` (Unix fn), each under
      `#[cfg(test)]` — verify by reading the diff: no non-`cfg(test)` line
      and no Windows arm changed.
      Observed: `git diff -U0 -- src/server.rs src/engine.rs src/service.rs`
      is 14 added lines and 0 removed: three `#[cfg(test)] let … =
      crate::spawn_gate::child_creation().await;` acquisitions (plus one
      `#[cfg(test)] drop(creation);` in `server.rs` at the existing guard
      release point) and their two-line comments. Every executable added
      line sits under `#[cfg(test)]`; all three sites are in the
      `#[cfg(unix)]` arm or a `#[cfg(unix)]` fn. The Windows arms
      (`server.rs` `NativeSpawnSpec` branch, `service.rs:1031`
      `spawn_service`, `service.rs:3733` `windows_starter_fixture`) have no
      diff lines.
- [x] 3.3 Confirm no deadlock: the literal `spawning().await` would nest
      under the outer shared guards (`spawn_gated_open`, 76 call sites, and
      about 100 facade/service/served-owner/activity fixtures) and under
      exclusive holders that spawn (`excluding_spawns` restarts,
      `service.rs` fixtures ~3086/~3253). `child_creation` must not wait in
      any of them — verify by running the full `kuru-memory` test task and
      seeing no test reach its deadline.
      Observed: no test reached its deadline in any of the three full-task
      runs recorded under 5.1 (`RUST_TEST_THREADS=2`). Their logs show no
      "deadline exceeded" and no hang, and the only panic is the
      template-fingerprint assertion discussed in 5.1. Every real-store
      open nested a `child_creation` under its fixture's outer `spawning()`
      guard without waiting.

## 4. Audit every child-creation site and extend the scan

- [x] 4.1 List every child-process construction in `packages/kuru-memory/src`
      and record its treatment (also the PR body's audit table) — verify
      by grep of `Command::new(`, `NativeSpawnSpec::new(`,
      `isolated_command(`, `engine::spawn(` and by reading each site.
      Observed (grep over `src`, excluding `spawn_gate.rs`'s own fixtures;
      line numbers refreshed by that grep on the final bytes of the review
      round in section 6; "fn" is the innermost enclosing function):
      | site | fn | treatment |
      |---|---|---|
      | `engine.rs:32` (`isolated_command(`) | `spawn`, unix arm | newly gated: `child_creation` |
      | `engine.rs:51` (`NativeSpawnSpec::new(`) | `spawn`, windows arm | Windows-only arm, unchanged; the scan passes it because the shared fn body holds `child_creation` |
      | `server.rs:633` (`Command::new(`) | `open_inner_with_probe_delay`, unix arm | newly gated: `child_creation`, dropped at the existing `_test_spawn_guard` release |
      | `server.rs:708` (`NativeSpawnSpec::new(`) | `open_inner_with_probe_delay`, windows arm | Windows-only arm, unchanged |
      | `server.rs:2673` (`engine::spawn(`) | `supervise_with_port_hook` | call, not a construction; gated inside `engine::spawn` |
      | `service.rs:999` (`Command::new(`) | `spawn_service`, `#[cfg(unix)]` | newly gated: `child_creation` |
      | `service.rs:1031` (`NativeSpawnSpec::new(`) | `spawn_service`, `#[cfg(windows)]` | Windows-only fn, unchanged |
      | `service.rs:2900`, `:2935` (`/bin/sh`) | `starter_wait_surfaces_the_exited_child_stderr` | already gated: `spawning()` |
      | `service.rs:3733` (`NativeSpawnSpec::new(`) | `windows_starter_fixture`, `#[cfg(windows)]` | Windows-only fn |
      | `service.rs:6898` (`Command::new(`) | `separate_cold_starters_share_one_owner_and_preserve_both_writes` | already gated: `spawning()` |
      | `server_tests.rs:65`, `:135` (`/bin/sh`) | two reap-guard tests | already gated: `spawning_blocking()` |
      | `server_tests.rs:220` (`engine::spawn(`) | `cleanup_observation_error_keeps_actual_lifecycle_lease_until_child_exit` | call; gated inside `engine::spawn`, outer `spawning()`/`locking_async` |
      | `provision.rs:1857` (`Command::new(`) | `isolated_command` | the builder: constructs but never spawns; scan-exempt by name |
      | `provision.rs:1892` (`engine::spawn(`) | `verify_version_recorded` | call; gated inside `engine::spawn`. Reached from the version probes on their own threads (`owned_probe`, `CheckedColdProbe::probe`); see the hang hazard in 6.2 |
      | `server/windows_fixture.rs:14`, `:76` | `unconfigured`, `partial_readiness` | Windows-only file |
      | `test_support/windows.rs:176` (`engine::spawn(`) | `forced_engine_cleanup` | Windows-only file; call |
      | `test_support/template.rs:846` (`Command::new(`) | `spawn_child`, `#[cfg(unix)]` | already gated: `spawning()` |
      | `test_support/template.rs:893` (`NativeSpawnSpec::new(`) | `spawn_child`, `#[cfg(windows)]` | Windows-only fn |
      | `store/engine_contract_tests.rs:283` (`/bin/hostname`) | `os_hostname`, `cfg(all(unix, not(linux)))` | **found by the scan, newly gated**: `child_creation` across creation only (`spawn()` then `wait_with_output()` outside the guard) |
      | `store/creation_template/tests.rs:2037`, `:2083` | `spawn_child` unix/windows | already gated: `spawning()` |
      | `store/recovery_tests.rs:96`, `:142` | `spawn_process_loss_creator` unix/windows | already gated: `spawning()` |
- [x] 4.2 Add `every_child_creation_takes_the_gate`: each construction's
      innermost enclosing function must contain a spawn-gate token
      (`spawning`, `child_creation`, `locking`, `excluding_spawns`), except
      Windows-only functions/files and the builder
      `provision::isolated_command` — verify with a permanent synthetic
      negative test (`the_child_creation_scan_reports_an_ungated_construction`)
      in place of the earlier throwaway fixture, plus the real-tree run.
      Observed: both tests pass in the `spawn_gate` lib run above. The
      real-tree scan asserts it saw more than 15 constructions and zero
      ungated. Before the hostname helper was gated, the same scan reported
      `store/engine_contract_tests.rs` `os_hostname` as ungated, which is
      the find recorded in 4.1.
- [x] 4.3 Confirm no quarantine assertion changes — verify with
      `git diff origin/main -- packages/kuru-memory/src/store/creation_template/`
      showing no change.
      Observed: the literal command is no longer empty, because
      `origin/main` advanced four commits past this branch's base
      (`0e562595`): main's new commits edit `creation_template/hooks.rs`
      (6 lines) and add 171 lines to `creation_template/open_tests.rs`.
      What this change itself does:
      `git diff 0e562595 -- packages/kuru-memory/src/store/creation_template/`
      is empty, `git log origin/main..HEAD -- …/creation_template/` lists no
      commit, and the working tree has no diff there. The quarantine tests
      are untouched by this change (at that revision; see 5.3 and 6.1).
      Rebased onto `origin/main` (new tip
      `b9453c5b`); `cargo test -p kuru-memory --lib spawn_gate` re-run after
      the rebase: 8 passed, including `every_child_creation_takes_the_gate`
      against main's new `server.rs`/`engine.rs` lines.

## 5. Local verification

- [x] 5.1 Run `mise run //packages/kuru-memory:test` (the full package
      task, `RUST_TEST_THREADS=2`) and record each binary's summary line.
      This run cannot reproduce the cross-test race on demand; it shows no
      regression and no deadlock, not that the race is absent.
      Observed (macOS host, this worktree). Two sessions ran the full task
      in this worktree at overlapping times (an implementer instance and a
      duplicate of it), so there are three runs:
      - Run A (implementer, started ~02:37, task exit 101): unittests
        `src/lib.rs` FAILED, 595 passed, 1 failed, 6 ignored, 900.60 s.
        The failure was
        `test_support::template::tests::concurrent_processes_create_one_template`
        at `template.rs:1032`:
        `left: ["Created", "Created"]`, `right: ["Created", "Reused"]`,
        child test ok, child stderr empty. The other binaries were ok:
        bundle_build 10, memory 5, server_lifecycle 12,
        supervisor_snapshot 1, and main/parent/windows_lifecycle 0 tests.
      - Run B (duplicate instance, overlapping run A): unittests
        `src/lib.rs` ok, 596 passed, 0 failed, 6 ignored, 850.18 s. Other
        binaries ok: bundle_build 10 (0.06 s), memory 5 (3.55 s),
        server_lifecycle 12 (28.96 s), supervisor_snapshot 1 (2.58 s).
      - Run C (implementer, started ~02:59 on the final bytes, after the
        rustfmt wraps and the `#[cfg(unix)]` field fix, task exit 0):
        unittests `src/lib.rs` ok, 596 passed, 0 failed, 6 ignored,
        774.67 s. Other binaries ok: bundle_build 10, memory 5,
        server_lifecycle 12 (23.88 s), supervisor_snapshot 1, and
        main/parent/windows_lifecycle 0 tests.
      `test_support::template::tests` also passed 3/3 when run in
      isolation (run by the implementer), and
      `concurrent_processes_create_one_template` passed 5/5 when run alone
      (run by the duplicate). That shows the failure is intermittent; it
      does not establish a cause.
      Run A's failure, from the code: the gate cannot produce two
      `Created` outcomes. `Created` needs `NotFound` under the
      cross-process exclusive key flock, and the gate is in-process and
      only delays (full argument in
      `tmp/roadmap/store-creation-design/diag-quarantine-structure.md`,
      "Separate finding"). Two inferences remain, both unverified. (1) The
      duplicate's build replaced the lib test binary or the prepared
      supervisor snapshot during run A, so the child (`current_exe()`) and
      the parent computed different template fingerprints. This holds only
      if a fingerprint input changed between the two builds. The compiled
      `SOURCES` (`store.rs`, `store/migrations.rs`, `store/usage_ledger.rs`,
      `server.rs`, `test_support/template.rs`) did not change in that
      window (only `spawn_gate.rs` did), so it would need the supervisor
      snapshot hash to differ. (2) A pre-existing intermittent defect in
      the template's cross-process exclusion. Left unchanged here and
      reported for separate routing.
      Run C ran on the final source bytes. `spawn_gate` (8/8), lint, Windows
      lint, typecheck and format were also re-run on them.
- [x] 5.2 Run `mise run format:check`, `//packages/kuru-memory:lint`,
      `//packages/kuru-memory:lint:windows`, and
      `//packages/kuru-memory:typecheck`, confirm each passes, and name any
      check that was not run.
      Observed: `format:check` first failed on two rustfmt wraps in
      `spawn_gate.rs`'s new tests; fixed with `format:fix`, then exit 0.
      `lint:windows` first failed: `field gate is never read` on
      `spawn_gate::Exclusive` for the Windows target, because the field is
      only read inside `#[cfg(unix)]` release calls; the field and its
      initialiser are now `#[cfg(unix)]` too, matching the module's
      Windows no-op shape. After that: `//packages/kuru-memory:lint` exit
      0, `//packages/kuru-memory:lint:windows` exit 0,
      `//packages/kuru-memory:typecheck` exit 0, `format:check` exit 0;
      `cargo test -p kuru-memory --lib spawn_gate` re-run after the field
      change: 8 passed. Not run: `coverage` (CI-enforced, not a hook) and
      the Linux and Windows native test legs (CI). The apply gate was not
      run in that session. It was run in the review round (section 6),
      before any code change: `mise run cospec -- validate
      memory-template-quarantine-gate --strict` exit 0 ("0 errors, 0
      warnings"), then `mise run cospec -- apply
      memory-template-quarantine-gate --json` exit 0, `gate.state` `clear`,
      no hard or soft blockers, 15/15 tasks complete.
- [x] 5.3 Note the scope of the 4.3 check: it held for the first revision.
      Section 6 changes three quarantine assertions to name the designed
      skip, which the brief allows ("unchanged except where an assertion
      must name the designed skip").

## 6. Review round on PR #171 (head `7156fd91`)

- [x] 6.1 Resolve the shared-guard residual: an open-path quarantine
      assertion that reaches `creation_template.rs::quarantine` (the
      non-waiting exclusive try) while the test holds only
      `spawn_gated_open`'s shared guard must name the designed skip —
      verify by reading each named assertion's quarantine path, then the
      diff and the tests.
      Observed (read at `7156fd91`): three of the four named tests reach
      `quarantine()`:
      `damaged_templates_send_the_opener_cold_and_preserve_copy_remnants`
      (structure and digest cases, via `condemn`),
      `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template`
      and `adoption_verdict_quarantines_and_leaves_the_stage_in_place` (both
      via `quarantine_after_adoption`). The fourth,
      `failed_copy_after_the_build_fails_the_open_without_a_cold_retry`,
      quarantines its verdict case with `quarantine_held` under the
      exclusive lock its own build still holds (`create_in`'s
      `Inspection::Absent` arm), so no busy skip can occur there. It is
      left unchanged.
      Change: under `cfg(test)`, `account()` (called after every quarantine
      outcome) records it by template root in a process-wide registry
      (`creation_template/hooks.rs`: `quarantined`, `quarantines`). It is
      process-wide because `quarantine_after_adoption` runs in the creation
      worker outside any `HOOKS` scope. `git diff -U0` of
      `creation_template.rs` is one doc line and
      `#[cfg(test)] hooks::quarantined(root, moved);`; no non-test line
      changed. `tests.rs::quarantined_or_busy(root, judged)` requires
      exactly one attempt for the test's private root. It accepts `Moved`
      with the judged template moved, or `Skipped("the store template key
      lock is busy")` with the judged template still published and nothing
      quarantined. Any other outcome fails. The three tests call it. In the
      shape test the skip ends the test before its "next project builds
      again" step, which needs the move.
      `a_busy_key_lock_skips_the_quarantine_and_keeps_the_older_one` now
      asserts the recorded `[Moved, Skipped(busy)]` sequence. The new
      `quarantined_or_busy_accepts_only_the_designed_skip` exercises the
      accept branch and two rejections (no attempt; a template that is not
      the published one) deterministically.
      Remedy (a), holding `locking_async()` across a direct
      `MemoryStore::open`, was rejected. It serialises every sibling spawn
      for the whole open, and an open that reaches a version probe would
      wait on itself (6.2).
- [x] 6.2 Name the cross-thread hang hazard and scan for it — verify with a
      synthetic negative test and the real-tree scan.
      Observed: the `spawn_gate.rs` module doc now says the holder is
      matched by thread or task only. The version probes
      (`provision::owned_probe`, `CheckedColdProbe::probe`) run
      `engine::spawn` on their own thread and runtime, so a `locking`
      holder that triggers a cold managed provision or a configured-binary
      `provision` waits on itself. It also says that
      `every_child_creation_takes_the_gate` is per function, not per span.
      `no_spawn_guard_encloses_a_test_cache_warm_up` now delegates to
      `warm_ups_under_guards`, which also tracks `spawn_gate::locking`
      bindings: a warm-up under either guard, or a word-bounded
      `provision(` under an exclusive guard, is reported. The new
      `the_warm_up_scan_reports_calls_under_a_held_guard` shows it reports
      lines 3, 8 and 9 of its sample and accepts the shared-guard
      `provision(`, the `gated_provision(` name, and calls after `drop` or
      outside the guard's scope. Real tree: "144 spawn guards and 52
      exclusive guards scanned, 0 violations"; "20 child constructions
      scanned, 0 ungated". `cargo test -p kuru-memory --lib spawn_gate`:
      9 passed.
- [x] 6.3 Record a further finding with the same mechanism, not fixed here —
      verify by reading `create_in` and `creation_worker.rs`.
      Observed [read]: on an empty private root, `create_in` takes the
      shared key lock, inspects `Absent`, drops it, and tries the exclusive
      lock without waiting (`creation_template.rs:1825`). `Ok(None)` returns
      `Created::Unavailable(Busy)`, which `creation_worker.rs:165` turns
      into a cold open. [inferred] A sibling's child holding a duplicate of
      the dropped shared lock makes that upgrade busy. An open-path test
      that asserts a build on an empty private root through
      `spawn_gated_open` (for example both cases of
      `failed_copy_after_the_build_fails_the_open_without_a_cold_retry`)
      would then fail. CI history was not searched for this shape.
      Accepting cold there would change what those tests test, so the
      finding is recorded in
      `tmp/roadmap/store-creation-design/diag-quarantine-structure.md` and
      reported for separate routing. This change's "impossible" claim
      covers the quarantine verdict only.
      Amended after an independent round-2 review: the residual also
      covers two tests already using `quarantined_or_busy`, through a
      different try. `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template`
      (`open_tests.rs:965-976`) and
      `adoption_verdict_quarantines_and_leaves_the_stage_in_place`
      (`open_tests.rs:1417`) build their flawed template with
      `tests::ensure` (`tests.rs:54-57`), which takes only the shared
      spawn-gate guard and drops the private root's exclusive key lock on
      return (`ensure_in`, `creation_template.rs:1592-1600`; released by
      dropping the `File`, not by `unlock()`). The subsequent `open`
      reaches `create_in`'s first non-waiting shared try
      (`creation_template.rs:1791`), whose `Ok(None)` is also
      `Unavailable(Busy)` and sends the open cold
      (`creation_worker.rs:165`). [inferred] A sibling child creation that
      straddles the drop and that try — admitted, because this test holds
      only a shared guard — would make the cold open succeed; the test
      then fails with "a copy of a flawed template was activated", before
      `quarantined_or_busy` is reached at all. This is the same mechanism
      and class as the paragraph above, not a regression; both tests are
      added to the roadmap diagnosis record's residual list for the same
      separate routing.
- [x] 6.4 Verify the review round — record each check.
      Observed (macOS host, this worktree): `format:check`,
      `//packages/kuru-memory:lint`, `//packages/kuru-memory:lint:windows`
      and `//packages/kuru-memory:typecheck` exit 0.
      `RUST_TEST_THREADS=2 cargo test -p kuru-memory --lib --all-features
      --locked store::creation_template`: 50 passed, 0 failed, 153.27 s,
      before the new helper test was added; the new test and
      `quarantine_is_bound_to_the_judged_template` then passed, 2/2.
      `mise run //packages/kuru-memory:test` (full package task,
      `RUST_TEST_THREADS=2`) on the final source bytes of this round, task
      exit 0: unittests `src/lib.rs` ok, 647 passed, 0 failed, 6 ignored,
      813.53 s; bundle_build 10, memory 5, server_lifecycle 12 (17.84 s),
      supervisor_snapshot 1, and the 0-test binaries ok. No test reached
      its deadline. This run shows no regression or deadlock; it cannot
      force the cross-test race, so the skip branch is covered by the two
      deterministic tests in 6.1.
- [x] 6.5 Independent review round 2 on head `0851eab8`, and the CI legs
      5.2 named as not run (coverage gate, Linux/Windows native legs).
      Observed: an independent reviewer (a subagent that did not write the
      change) read the full diff, the touched files and the diagnosis
      record, and returned verdict ACCEPT with no must-fix. Both round-1
      must-fixes were re-verified independently (cospec archived as a pure
      rename; the three open-path assertions reach `quarantined_or_busy`
      correctly). Should-fix 1 (the 6.3 residual was incomplete) is
      resolved above; should-fix 2 (`quarantined_or_busy`'s third arm
      untested) and the visibility nit are code changes, out of scope for
      a record-only amendment after archiving. Should-fix 4 (PR body
      wording) is not part of the cospec record.
      CI evidence (run `36989424837`, head `0851eab8139039574917ae42a72262f5ae19c38d`,
      `gh pr checks 171 --repo replygirl/kuru`): every job passed except
      the two Windows arm64 `dolt-windows-arm64` reproducibility legs,
      which report `skipping` (unrelated cache-key gating, not a failure).
      This closes what 5.2 named as not run: `native-tests (ubuntu-latest,
      windows-latest, windows-11-arm, macos-latest) / Require native
      coverage and installation checks` all pass; the coverage-merge jobs
      for macOS, Ubuntu and Windows all pass. A green run does not by
      itself prove the cross-test race is closed — it cannot be forced
      from a test — so this does not change the 6.3/6.4 analysis, only
      records that the previously-pending legs finished green.
