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

Root cause, read from the lock/spawn code (not inferred): the per-key
template quarantine is best-effort by design (`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`
section 3.7) — after a verdict the code drops its shared key lock and tries
the exclusive lock without waiting (`creation_template.rs`'s `quarantine`),
and any sibling spawn holding a duplicate of the just-released shared
`flock` through its `posix_spawn`/exec window makes that try return
`WouldBlock`, skipping the quarantine (the exact observed symptom:
`published(&root)` stays `Some`). `crate::spawn_gate`'s write guard
(`locking_async`) excludes this only for spawns that take the gate's shared
guard (`spawning`/`spawning_blocking`) — it cannot exclude a spawn that
never asks for the gate at all.

Tracing every spawn site in `packages/kuru-memory/src` found exactly that:
`MemoryStore::temporary()`/`temporary_cold()` (used at roughly 90 call
sites across the crate's test suite — `store.rs`, `facade.rs`, `service.rs`,
and more) spawn the private memory lifetime supervisor through
`store.rs::open_inner` → `Server::open_with_guard` →
`server.rs::open_inner_with_probe_delay` → `command.spawn()`
(`server.rs:~653`/`~727`). That function takes an optional
`_test_spawn_guard: Option<RwLockReadGuard>` for exactly this purpose, but
every caller on this path passes `None` — only the dedicated
`Server::open_with_initial_probe_delay` test helper (used by one
startup-probe test, not by `temporary()`) supplies a real
`spawn_gate::spawning()` guard. `warm_runtime_cache()`'s own gate use
(`test_support.rs::warm_engine`) covers only the one-time, `OnceCell`-cached
provisioning probe, not this per-call supervisor spawn. With
`RUST_TEST_THREADS=2` in CI's coverage job, any of those ~90 ungated
`temporary()`/`temporary_cold()` calls running on the other thread can
race the exclusive-lock try inside `create_unspawned`'s write guard,
regardless of how the quarantine test itself is gated.

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
  `spawn_gate::spawning()` guard — sits outside those files, is reached by
  ~90 call sites across the crate, and changes suite-wide gate waiting
  behavior (any `temporary()` open would then wait behind a held write
  guard, and WARM-list ordering would need to cover it). That decision is
  recorded in `blocking-changes.md` as a soft blocker for the orchestrating
  session, not made unilaterally here.

## Impact

- Test-only, no product behavior change. No file in
  `packages/kuru-memory/src` is edited by this change.
- `openspec/changes/memory-template-quarantine-gate/` only.
- The goal "eliminate this failure family for every template test" is not
  achieved by this change; it requires a decision at `open_temporary`
  outside this change's scoped files, named here as a follow-on for the
  orchestrating session.
