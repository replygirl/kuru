# Proposal

## Why

`store::creation_template::tests::template_failing_structure_is_quarantined`
failed in CI again (PR #133, run 36969597416, job 110720770956,
macos-latest coverage partition 1: `Error: another format: not
quarantined`, 152 passed, 1 failed) at head `0910c326` merged onto
`0e562595`, which already contains #155 (`70fa6c0a`) unmodified — confirmed
by reading `tests.rs` at the exact tested commit `5c689e638`: both
`create_in` calls in this test already route through `create_unspawned`,
#155's own fix. **#155's write guard was already held across the
verdict-producing call in the failing run, and the quarantine was still
skipped.** Re-applying `create_unspawned` to this test changes nothing; the
goal of making the failure impossible cannot be met by edits confined to
`tests.rs`/`open_tests.rs`.

Measured mechanism: the per-key template quarantine is best-effort by
design (`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`
section 3.7) — after a verdict the code drops its shared key lock and tries
the exclusive lock without waiting (`creation_template.rs`'s `quarantine`),
and a sibling spawn holding a duplicate of the just-released shared
`flock` through its `posix_spawn`/exec window makes that try return
`WouldBlock`, skipping the quarantine (the exact observed symptom:
`published(&root)` stays `Some`). This part is read directly from the
code and matches the observed symptom exactly, not inferred.

What *is* inferred, and is labeled as such: tracing every spawn site in
`packages/kuru-memory/src` found that `crate::spawn_gate`'s write guard
(`locking_async`) excludes a spawn only when that spawn takes the gate's
own shared guard (`spawning`/`spawning_blocking`) first — it cannot
exclude a spawn that never asks for the gate at all — and
`MemoryStore::temporary()`/`temporary_cold()` (confirmed at 92 call sites
by `grep -rn "::temporary()\|::temporary_cold()" src`, across `store.rs`,
`facade.rs`, `service.rs` and more) reach exactly such an ungated spawn:
`store.rs::open_inner` calls `Server::open_with_guard` unconditionally
(confirmed at store.rs ~line 2003 and ~2047) → `server.rs::open_inner`
(line 546-549) → `open_inner_with_probe_delay(options, reap_guard, None,
None)` (line 550, the only caller besides line 560) →
`command.spawn()` (line ~653/~727) under a `_test_spawn_guard` parameter
that is `None` here. Only `Server::open_with_initial_probe_delay` (line
560, one startup-probe test, not reached by `temporary()`) supplies a
real `spawn_gate::spawning()` guard through that same parameter.
`warm_runtime_cache()`'s own gate use (`test_support.rs::warm_engine`)
covers only the one-time, `OnceCell`-cached provisioning probe, not this
per-call supervisor spawn. With `RUST_TEST_THREADS=2` in CI's coverage job
(`packages/kuru-memory/mise.toml:56`,`:157`), this ungated path is
reachable from the other concurrent test thread. Supporting (not
conclusive) evidence for this specific run: the job log shows
`store::migrations::publication_record_tests::unrecorded_failed_attempt_is_still_fully_classified`
— which calls `MemoryStore::temporary_cold()` directly, ungated
(`publication_record_tests.rs:478`) — completing within the same
coverage partition close to the failure's report; cargo prints a
binary's failures only after every test in it finishes, so this shows
the ungated path was active in the same run, not a confirmed exact-moment
overlap with the failing assertion.

What this change does *not* claim: that this exact ungated spawn was the
one that raced `template_failing_structure_is_quarantined` in run
36969597416. No log evidence pins down *which* concurrent spawn (if any
single one did, rather than some other unaccounted-for ungated site) held
the flock duplicate at that moment; cargo test does not log that.

## What Changes

- A diagnosis record (this proposal, plus `tasks.md`'s evidence) stating
  the measured fact above: goal "every quarantine assertion holds the
  write guard" is already satisfied in `creation_template/tests.rs` and
  did not prevent the recorded failure, because the dominant spawner in the
  test binary never takes the gate.
- A completed sweep of every quarantine assertion in
  `packages/kuru-memory/src/store/creation_template/tests.rs` and
  `open_tests.rs`, confirming (not re-doing) that every one which can take
  the write guard already does, following #155's exact pattern, and naming
  the ones that structurally cannot (opens, which take only the shared
  guard — #155's own recorded follow-on) or need not (failed-build/refused-
  publication assertions, which have no pre-existing published template to
  race over).
- No code change to `creation_template/tests.rs`, `open_tests.rs`, or
  `spawn_gate.rs`: the sweep finds no additional gating opportunity inside
  the two files the brief scoped this change to. The real chokepoint —
  `open_temporary`'s call into `Server::open_with_guard` taking no
  `spawn_gate::spawning()` guard — sits outside those files and is reached
  by 92 call sites across the crate. `temporary()`, `temporary_cold()` and
  `open_temporary(` are already in `spawn_gate.rs`'s own `WARM` scan list
  (`no_spawn_guard_encloses_a_test_cache_warm_up`), so a caller already
  holding a guard when it calls them is already a lint violation today —
  gating the open itself adds no new deadlock there. The actual cost is
  wait time: a write-guard holder (like `create_unspawned`) would then
  queue behind any in-flight `temporary()` open (at most one more, with
  `RUST_TEST_THREADS=2`), not a cross-call deadlock. Two candidate shapes,
  not chosen between here: (a) wrap the whole open in `open_temporary`
  with `spawning()`, matching `spawn_gated_open`'s existing pattern; (b)
  thread a `#[cfg(test)]` `spawning()` guard through the existing
  `_test_spawn_guard` parameter on the default path, dropped right after
  `command.spawn()`, matching what `open_with_initial_probe_delay` and
  `tests.rs::spawn_child` already do. (b) is narrower but touches
  `server.rs`. That decision is recorded in `blocking-changes.md` as a
  soft blocker for the orchestrating session, not made unilaterally here.

## Impact

- Test-only, no product behavior change. No file in
  `packages/kuru-memory/src` is edited by this change.
- `openspec/changes/memory-template-quarantine-gate/` only.
- The goal "eliminate this failure family for every template test" is not
  achieved by this change; it requires a decision at `open_temporary`
  outside this change's scoped files, named here as a follow-on for the
  orchestrating session.
