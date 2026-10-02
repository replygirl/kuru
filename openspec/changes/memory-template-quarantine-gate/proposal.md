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

- Under `cfg(test)`, `server.rs`'s default open path
  (`Server::open_inner` → `open_inner_with_probe_delay`) takes
  `spawn_gate::spawning()` exactly across `command.spawn()`, the same
  guard `open_with_initial_probe_delay` already takes, threaded through the
  existing `_test_spawn_guard` parameter instead of always passing `None`.
  This is shape (b) from the prior revision's two candidates, not shape
  (a) (wrapping the whole open in a shared guard): (b) holds the guard only
  across the actual spawn, so `locking_async()` holders (the quarantine
  write-guard tests) queue behind at most one in-flight spawn, not an
  entire open's readiness wait — matching the constraint that every guard
  spans only child creation and spawning tests keep running concurrently
  with one another.
- A full audit of every child-process creation site reachable from the
  `kuru-memory` test binary (`engine.rs::spawn`, `service.rs`'s supervisor
  and project-service launches, `provision.rs`'s probe spawns,
  `test_support/template.rs`'s stage/template builders, and the sites
  `spawn_gate.rs`'s own module doc already names as gated), recording each
  site's treatment — already-gated, newly gated by this change, or
  correctly left ungated with the reason (no `cfg(test)` reachability, or
  already excluded by taking the lock-gate's exclusive guard instead).
- `spawn_gate.rs`'s existing scan (`no_spawn_guard_encloses_a_test_cache_warm_up`)
  extended so a newly introduced spawn site with no guard in scope is
  caught by a test, not just by manual audit.
- No change to the quarantine tests themselves beyond what #155 already
  made (`create_unspawned`, the busy-lock test naming the designed skip);
  this change's assertions are unchanged.

## Impact

- Test-only: the new guard acquisition compiles only under `cfg(test)`
  (`spawn_gate` is already a strict no-op outside `cfg(test)`, including on
  Windows). No product quarantine semantics change: no retry on
  `WouldBlock`, no blocking lock, no deadline change, no change to the
  gate's `RwLock` semantics (shared for spawns, exclusive for lock takers).
- Files: `packages/kuru-memory/src/server.rs` (`cfg(test)` guard threading
  on the default open path), `packages/kuru-memory/src/spawn_gate.rs`
  (scan extension), and any other site the audit finds ungated and
  reachable; `openspec/changes/memory-template-quarantine-gate/` for this
  record.
- Makes the failure family impossible in-process rather than rarer: every
  spawn in the test binary now either takes the shared guard or is proven
  unreachable while a write-guard holder runs.
