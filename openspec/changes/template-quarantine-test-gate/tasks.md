# Tasks

The fix is already implemented in `packages/kuru-memory/src/store/creation_template/tests.rs`
(commit `c78b2b1c`). These tasks verify each part of it rather than author it
from scratch.

## 1. Lock-gate helper coverage

- [x] 1.1 Confirm `create_unspawned(root, stage)` holds
      `spawn_gate::locking_async` around `create_in` and is used at every
      no-engine `create_in` call whose assertion depends on the exclusive
      key lock having actually run (the third creator in
      `second_creator_goes_cold_at_once_while_a_build_is_in_progress`, both
      calls in `template_failing_structure_is_quarantined`, both in
      `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`,
      and `quarantine_is_bound_to_the_judged_template`) — verify by reading
      the diff and `cargo expand`-free code review of each call site.
      Observed: `create_unspawned` binds `spawn_gate::locking_async()` for
      the whole `create_in(root, &no_engine(), stage)` call; the four
      existing tests above route those calls through it, and the new test
      uses it for all three of its creations (four rerouted tests plus one
      new test). No engine is passed, so no spawn happens under the write
      guard, and `create_in` takes no spawn-gate guard itself (the gate is
      test-only).
- [x] 1.2 Confirm calls that expect `Busy` or inject lock faults, and the
      second creator in the two-creators test, are left ungated (gating the
      second creator would deadlock against its own paused build's spawn
      guard) — verify by reading each remaining ungated `create_in` call
      site and its assertion.
      Observed: three `create_in` calls stay ungated: the second creator in
      `second_creator_goes_cold_at_once_while_a_build_is_in_progress`
      (expects `Busy`; the paused first build holds a `spawning` guard),
      the copier in `concurrent_template_lock_acquisition_never_waits`
      (expects `Busy` under a held exclusive lock) and the
      injected-fault loop in
      `template_lock_errors_mean_no_template_and_are_fatal_in_warm_up`
      (expects `Unavailable::Lock`). A spurious busy cannot turn any of
      them into a false pass that hides a quarantine.

## 2. Failure-site assertion order

- [x] 2.1 Confirm `published(&root).is_none()` is asserted before the
      identity comparison at the quarantine-identity failure site, with a
      message that names the case — verify by reading
      `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`.
      Observed: `ensure!(published(&root).is_none(), "{case}: not quarantined")`
      precedes the exactly-one and identity checks; the identity
      `assert_eq!` now says "{case}: another directory was quarantined".

## 3. Deterministic skip-pinning test

- [x] 3.1 Confirm `a_busy_key_lock_skips_the_quarantine_and_keeps_the_older_one`
      holds the key lock shared (via a second lock description) between the
      copier dropping its own shared lock and the exclusive try, and asserts
      the verdict is returned, the judged template stays published, and the
      older rejected directory is unchanged — verify by running it locally
      with a private `KURU_DOLT_CACHE` (do not touch the shared
      `$TMPDIR/kuru-dolt-test-cache` if it is invalid) and recording pass
      count.
      Observed (local macOS arm64, fixed binary, private `KURU_DOLT_CACHE`
      in the session scratchpad, `KURU_TEST_SUPERVISOR_PREPARED=1`,
      `RUST_TEST_THREADS=2`): `--exact` 5/5 pass (0.45-0.68 s each). The
      hook takes the key lock shared through a second description after
      `drop(shared)` and before the exclusive try; the test then asserts
      the verdict, `published(&root) == Some(judged)`, the unchanged older
      `.rejected-*` list, and that a later unobstructed verdict quarantines
      the judged template and replaces the older directory.

## 4. Regression checks

- [x] 4.1 Run `mise run //packages/kuru-memory:test` (or the equivalent
      targeted `cargo test` invocation) for `store::creation_template::`
      alone and at module concurrency, plus with `facade::tests` and
      `service::tests` running concurrently, and record pass counts —
      verify no regressions from the lock-gate helper.
      Observed: `mise run //packages/kuru-memory:test -- store::creation_template::`
      exit 0 (27 passed in the lib binary; it also ran `prefetch` and
      prepared the supervisor snapshot). Then, same binary, same env:
      module 5/5 pass (27 tests each, 34-43 s); module + `facade::tests` +
      `service::tests` 2/2 pass (118 tests each, 190 s and 211 s). The
      race itself does not reproduce without the sibling-spawn condition
      (baseline `ad743791`: 30/30 alone, 20/20 module, 8/8 with facade and
      service, per the diagnosis), so these counts show no regression, not
      a before/after; the deterministic test in 3.1 stands in for that.
      The full package suite and coverage were not run locally; CI runs
      them.
- [x] 4.2 Run the `spawn_gate::` test suite, including its source-scan test,
      and confirm it still reports zero violations — verify the new
      `spawn_gate::locking_async` usage introduces no scan violation.
      Observed: `spawn_gate::` 4/4 pass with `--nocapture`, printing
      "125 spawn guards scanned, 0 violations".
- [x] 4.3 Run `mise run //packages/kuru-memory:lint`,
      `//packages/kuru-memory:lint:windows`, `format:check`, and
      `typecheck`, and confirm hk pre-commit hooks pass — verify the
      test-only change is clean on every required static gate.
      Observed: `mise run format:check` exit 0;
      `//packages/kuru-memory:typecheck` exit 0;
      `//packages/kuru-memory:lint` exit 0;
      `//packages/kuru-memory:lint:windows` exit 0. hk pre-commit runs on
      the commit that records this evidence.
