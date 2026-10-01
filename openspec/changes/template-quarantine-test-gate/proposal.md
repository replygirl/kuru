# Proposal

## Why

`store::creation_template::tests::template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`
failed once in CI (PR #150, run 36831872991, macos-latest coverage partition
2, job 110270430882) with a `FileIdentity` mismatch. Diagnosis (see
`tmp/roadmap/store-creation-design/diag-quarantine-identity.md`, independently
verified) found the test, not the product, at fault: design 3.7 makes
quarantine best-effort — the copier drops its shared key lock and tries the
exclusive lock without waiting, and any sibling test in the same binary
spawning a child at that instant can keep the released shared `flock` alive
through `posix_spawn` until `exec` (see `spawn_gate.rs`), so the exclusive
try returns `WouldBlock` and the quarantine is skipped by design. The test
assumed a quarantine always happens and its assertion order reported the
skip as an identity mismatch. Product behaviour is correct and unchanged.

## What Changes

- `packages/kuru-memory/src/store/creation_template/tests.rs` (commit
  `c78b2b1c`, already applied in this branch):
  - Adds a `create_unspawned(root, stage)` helper that runs a no-engine
    `create_in` under `spawn_gate::locking_async`, so no sibling test's
    spawn can hold the key lock's `flock` through the exclusive try. Used
    for every no-engine `create_in` whose assertion needs the lock
    acquisition to have actually run: the third creator in
    `second_creator_goes_cold_at_once_while_a_build_is_in_progress` (after
    the first build has joined and released its own spawn guard, so no
    deadlock), both calls in `template_failing_structure_is_quarantined`,
    both in `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`,
    and `quarantine_is_bound_to_the_judged_template`.
  - Leaves ungated: calls that expect `Busy` or inject lock faults (a
    spurious busy still satisfies them), and the second creator in the
    two-creators test, whose paused build already holds a spawn guard —
    gating it there would deadlock.
  - At the failure site, reorders the assertions so `published(&root).is_none()`
    ("{case}: not quarantined") is checked before the identity comparison,
    and the message names the case.
  - Adds `a_busy_key_lock_skips_the_quarantine_and_keeps_the_older_one`,
    which holds the key lock shared through a second lock description in a
    `before_quarantine` hook (after the copier drops its own shared lock,
    before the exclusive try) to pin the designed skip deterministically:
    the verdict is still returned, the judged template stays published,
    and the older rejected directory is unchanged.

## Impact

- Test-only: no product code, quarantine semantics, or spawn-gate behaviour
  changes; no new environment variable.
- `packages/kuru-memory` test binary only; `cog.toml` maps `test` to the
  same `bump_patch` as `fix`.
- The race itself does not reproduce without the sibling-spawn condition:
  measured 30/30 running the affected test alone, 20/20 at module
  concurrency, and 8/8 with `facade::tests` and `service::tests` also
  running, on the pre-fix baseline (`ad743791`). The new deterministic test
  stands in for a before/after demonstration of the scheduling race, which
  is not reproducible on demand.
- `spawn_gate::` source-scan test still passes (no new violations;
  "125 spawn guards scanned, 0 violations" on the fixed binary).
