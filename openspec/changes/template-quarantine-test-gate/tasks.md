# Tasks

The fix is already implemented in `packages/kuru-memory/src/store/creation_template/tests.rs`
(commit `c78b2b1c`). These tasks verify each part of it rather than author it
from scratch.

## 1. Lock-gate helper coverage

- [ ] 1.1 Confirm `create_unspawned(root, stage)` holds
      `spawn_gate::locking_async` around `create_in` and is used at every
      no-engine `create_in` call whose assertion depends on the exclusive
      key lock having actually run (the third creator in
      `second_creator_goes_cold_at_once_while_a_build_is_in_progress`, both
      calls in `template_failing_structure_is_quarantined`, both in
      `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`,
      and `quarantine_is_bound_to_the_judged_template`) — verify by reading
      the diff and `cargo expand`-free code review of each call site.
- [ ] 1.2 Confirm calls that expect `Busy` or inject lock faults, and the
      second creator in the two-creators test, are left ungated (gating the
      second creator would deadlock against its own paused build's spawn
      guard) — verify by reading each remaining ungated `create_in` call
      site and its assertion.

## 2. Failure-site assertion order

- [ ] 2.1 Confirm `published(&root).is_none()` is asserted before the
      identity comparison at the quarantine-identity failure site, with a
      message that names the case — verify by reading
      `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`.

## 3. Deterministic skip-pinning test

- [ ] 3.1 Confirm `a_busy_key_lock_skips_the_quarantine_and_keeps_the_older_one`
      holds the key lock shared (via a second lock description) between the
      copier dropping its own shared lock and the exclusive try, and asserts
      the verdict is returned, the judged template stays published, and the
      older rejected directory is unchanged — verify by running it locally
      with a private `KURU_DOLT_CACHE` (do not touch the shared
      `$TMPDIR/kuru-dolt-test-cache` if it is invalid) and recording pass
      count.

## 4. Regression checks

- [ ] 4.1 Run `mise run //packages/kuru-memory:test` (or the equivalent
      targeted `cargo test` invocation) for `store::creation_template::`
      alone and at module concurrency, plus with `facade::tests` and
      `service::tests` running concurrently, and record pass counts —
      verify no regressions from the lock-gate helper.
- [ ] 4.2 Run the `spawn_gate::` test suite, including its source-scan test,
      and confirm it still reports zero violations — verify the new
      `spawn_gate::locking_async` usage introduces no scan violation.
- [ ] 4.3 Run `mise run //packages/kuru-memory:lint`,
      `//packages/kuru-memory:lint:windows`, `format:check`, and
      `typecheck`, and confirm hk pre-commit hooks pass — verify the
      test-only change is clean on every required static gate.
