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

- `packages/kuru-memory/src/store/creation_template/tests.rs` (already
  applied in this branch: the test change in `2f904264`, its post-rebase
  return-type correction in `8bd2f7ad`):
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
    and the older rejected directory is unchanged. Its hooked
    `create_in` is bounded by `PROMPT`: the shared holder is released only
    after that call returns, so a quarantine that waited for the exclusive
    lock fails the test with a diagnostic instead of hanging it.

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
  "125 spawn guards scanned, 0 violations" on the fixed binary). That pass
  says nothing about the new call sites: the scan
  (`no_spawn_guard_encloses_a_test_cache_warm_up`) tracks only
  `spawn_gate::spawning` bindings and cannot see `locking_async`. That no
  new write-guard site deadlocks rests on call-site review: each
  `create_unspawned` call passes no engine, so nothing spawns under the
  write guard, and none runs while the same task holds a `spawning` guard.

## Follow-ons (not in this change)

- A test-only `Quarantine` outcome hook that would name the skip reason on
  a future failure.
- The same designed skip remains reachable from open tests that assert a
  quarantine but cannot take the write guard, because the open itself
  spawns (`spawn_gated_open` takes only the shared `spawning` guard,
  `test_support/template.rs`): in `store/creation_template/open_tests.rs`,
  `damaged_templates_send_the_opener_cold_and_preserve_copy_remnants`
  (structure and digest cases),
  `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template`
  and `adoption_verdict_quarantines_and_leaves_the_stage_in_place`. A
  sibling spawn between the opener's shared-lock release and its exclusive
  try would skip the quarantine there too; they need their own treatment.
