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
skipped.**

Measured mechanism: the per-key template quarantine is best-effort by
design (`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`
section 3.7) — after a verdict the code drops its shared key lock and tries
the exclusive lock without waiting (`creation_template.rs`'s `quarantine`),
and a sibling spawn holding a duplicate of the just-released shared
`flock` through its `posix_spawn`/exec window makes that try return
`WouldBlock`, skipping the quarantine (the exact observed symptom:
`published(&root)` stays `Some`). This part is read directly from the
code and matches the observed symptom exactly, not inferred.

What *is* inferred, and is labeled as such: `crate::spawn_gate`'s write
guard (`locking_async`) excludes a spawn only when that spawn takes the
gate's own shared guard (`spawning`/`spawning_blocking`) first — it cannot
exclude a spawn that never asks for the gate at all — and
`MemoryStore::temporary()`/`temporary_cold()` (confirmed at 92 call sites
by `grep -rn "::temporary()\|::temporary_cold()" src`) reach exactly such
an ungated spawn: `store.rs::open_temporary` calls `Server::open_with_guard`
unconditionally → `server.rs::open_inner` → `open_inner_with_probe_delay`
with `_test_spawn_guard: None` → `command.spawn()` (confirmed at
`server.rs` lines 542-574, 629-657 unix, 697-730 windows). Only
`Server::open_with_initial_probe_delay` (`#[cfg(test)]`, one startup-probe
test) supplies a real `spawn_gate::spawning()` guard through that same
parameter. `warm_runtime_cache()`'s own gate use
(`test_support.rs::warm_engine`) covers only the one-time, `OnceCell`-cached
provisioning probe, not this per-call supervisor spawn. With
`RUST_TEST_THREADS=2` in CI's coverage job
(`packages/kuru-memory/mise.toml:56`,`:157`), this ungated path is
reachable from the other concurrent test thread. What this change does
*not* claim: that this exact ungated spawn was the one that raced
`template_failing_structure_is_quarantined` in run 36969597416 — no log
evidence pins down which concurrent spawn, if any single one did, held the
flock duplicate at that moment.

A prior scoped attempt (this change's own earlier revision, commits
`80a2662b`/`94186861`) confined itself to
`creation_template/tests.rs`/`open_tests.rs` and concluded no fix was
possible there, raising the chokepoint as a soft blocker for the
orchestrating session to decide. This revision carries that decision: gate
the chokepoint itself, under `cfg(test)` only, using the plumbing the
codebase already has.

## What Changes

- Under `cfg(test)`, every child creation on a product code path that the
  `kuru-memory` test binary drives holds a spawn-gate guard exactly across
  the spawn. The paths are: the lifetime-supervisor launch in
  `server.rs::open_inner_with_probe_delay`, the Dolt engine in
  `engine.rs::spawn` (which covers the in-process supervisor and
  `provision::verify_version`), and the project memory service in
  `service.rs::spawn_service`.
- **Deviation from the brief, with the reason:** the brief's literal remedy,
  `crate::spawn_gate::spawning().await` inside the open path, deadlocks.
  `spawn_gate.rs`'s own `excluding_spawns` doc says the gate is fair, so a
  nested shared acquisition waits behind a queued writer forever. Many
  fixtures already hold `spawning()` across whole opens:
  `test_support::spawn_gated_open` has 76 call sites, and about 100 more
  facade/service/served-owner/activity tests do the same. `excluding_spawns`
  restarts and two service fixtures (`service.rs` ~3086, ~3253) spawn while
  holding the *exclusive* guard. The product paths therefore take a new
  `spawn_gate::child_creation()` (Unix only):
  - It waits only while a lock taker *holds* the exclusive guard, never
    behind one that is only queued, so nesting under any fair shared guard
    cannot deadlock.
  - It counts creations in flight. `locking()`/`locking_async()` return
    only after that count drains, so a creation already in flight when a lock
    taker arrives is still excluded.
  - It does not wait when the current test (thread or tokio task) is the one
    holding the exclusive guard. A libtest test owns its thread and its
    runtime.
  - `spawning()`, `spawning_blocking()`, `excluding_spawns()` and the tokio
    `RwLock` are unchanged: shared for spawns, exclusive for lock takers.
    `locking()`/`locking_async()` return a thin wrapper around the write
    guard that clears the holder mark on drop or downgrade.
- A full audit of every child-process construction compiled into the crate,
  recorded in tasks.md section 4 and in the PR body, gives each site's
  treatment.
- A new scan test, `spawn_gate::tests::every_child_creation_takes_the_gate`,
  fails when a function constructs a child process (`Command::new(`,
  `NativeSpawnSpec::new(`, `isolated_command(`) without any spawn-gate token
  in its body. Windows-only functions and files and the one builder
  (`provision::isolated_command`) are exempt by name. A permanent synthetic
  negative test (`the_child_creation_scan_reports_an_ungated_construction`)
  proves that the scan reports an ungated construction. Besides the three
  product sites above, the scan found one more: the macOS-only
  `/bin/hostname` helper in `store/engine_contract_tests.rs`, now gated
  across creation only.
- Quarantine assertions that run through an ordinary open
  (`spawn_gated_open`, a shared guard only) name the designed skip. Under
  `cfg(test)`, `creation_template::account` records every quarantine outcome
  by template root (`creation_template/hooks.rs`). The helper
  `creation_template::tests::quarantined_or_busy` accepts exactly one
  attempt: the judged template moved, or `Skipped("the store template key
  lock is busy")` with the judged template still published and nothing
  quarantined. Every other outcome fails. Three open-path tests use it:
  `damaged_templates_send_the_opener_cold_and_preserve_copy_remnants`,
  `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template`
  and `adoption_verdict_quarantines_and_leaves_the_stage_in_place`.
  `failed_copy_after_the_build_fails_the_open_without_a_cold_retry` is
  unchanged: its quarantine runs under the exclusive lock its build holds.
  Two `tests.rs` tests check the recorded skip deterministically.
- The `spawn_gate` module doc names the cross-thread hang hazard: version
  probes spawn on their own thread, which the exclusive holder's own child
  creation cannot be matched to. The warm-up scan now also reports a
  warm-up or a `provision(` call under a held `locking` guard, with its own
  synthetic negative test.

## Impact

- Test-only: every new acquisition compiles only under `cfg(test)`, and
  `spawn_gate` is a `cfg(test)` module. Product quarantine semantics are
  unchanged: no retry on `WouldBlock`, no blocking lock, no deadline change.
  Windows behaviour is unchanged: `child_creation` and its bookkeeping exist
  only on Unix, and no Windows arm was edited.
- Files:
  - `packages/kuru-memory/src/spawn_gate.rs`: the gate, its tests and the
    scans.
  - `packages/kuru-memory/src/store/creation_template.rs`: one
    `#[cfg(test)]` record in `account`, plus its doc line.
  - `packages/kuru-memory/src/store/creation_template/{hooks.rs,tests.rs,open_tests.rs}`:
    the quarantine record, `quarantined_or_busy`, and the three
    assertions.
  - `packages/kuru-memory/src/{server.rs,engine.rs,service.rs}`: one
    `#[cfg(test)]` acquisition per spawn.
  - `packages/kuru-memory/src/store/engine_contract_tests.rs`: the hostname
    helper.
  - `openspec/changes/memory-template-quarantine-gate/`: this record.
- Makes the quarantine-verdict failure impossible in-process rather than
  rarer. A test that holds the exclusive guard across its release-to-try
  window has no child creation in flight in the binary, so its quarantine
  cannot be skipped. A test that reaches the verdict through an ordinary
  open holds only a shared guard and accepts the recorded designed skip.
- Residual, not fixed here [read; reachability inferred]: `create_in`'s
  shared-to-exclusive upgrade on an empty private root is a non-waiting try
  as well. A busy upgrade sends the open cold (`creation_worker.rs:165`), so
  an open-path test asserting a build through `spawn_gated_open` is exposed
  to the same mechanism. Accepting cold there would change what those tests
  test; tasks.md 6.3 records the finding for separate routing.
