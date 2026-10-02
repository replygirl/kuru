# Proposal

## Why

`kuru-runtime`'s
`hook_tests::cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`
failed in CI with `Elapsed(())` (PR #133 run 37038146977, head `f8a107fc` on
main `a3de5346`, macos-latest coverage partition 2, job `110941628749`: 53
passed, 1 failed in 53.42s). Measured from the job log: the panic is at
`hook_tests.rs:1765:6`, the `.unwrap()` closing the **marker wait** (lines
1759-1765: a flat `tokio::time::timeout(Duration::from_secs(10), ...)` polling
for the second hook's marker file), not the dream-join wait at 1767-1770. The
test was reported `FAILED` at 17:15:51.79; its earliest possible start under
`RUST_TEST_THREADS=2` was 17:15:41.08 (inference from libtest's name-ordered
scheduling), so it ran about 10.7 s: harness setup plus the whole 10 s wait.
The concurrent slot meanwhile ran a sibling dream fixture
(`dream_tool_rewrites...`) for about 13.9 s (inference, same scheduling); the
same two tests take about 2 s and 6 s locally on an uninstrumented build. The
log records no step timings, so which step was slow is not measured.

The marker appears only after the dream has run `reconcile()`, the dream
lease, `begin_candidate`, the actor's reads, prompt append and usage
admission, the fake provider call, the call-1 tool and summary appends, the
first post-tool hook (bounded by its own `timeout_ms` 5 s plus owned
cleanup), its annotation write, the call-2 tool append and the second hook's
launch. Each memory step is bounded by the store's 30 s statement budget;
the hook by its stated timeout. The test's 10 s is not derived from any of
these: it is a number guessed at runner speed that decides the outcome while
nothing is stuck.

The dream-join wait two lines later is a flat 10 s around the product's own
cancellation wait, `Harness::await_hook_cleanup` -> `HookHost::quiesce()`,
bounded by `QUIESCE` = 10 s (= 2 x `CLEANUP`, stated in `hooks.rs`). A test
bound equal to the product bound it encloses can lose a tie under load while
the product honours its contract. This did not fire in the captured run
(labelled inference: same defect class, latent).

The product cancellation path has no defect: `quiesce()` waits on the event
(`active == 0`) under a stated derived bound and reports `CleanupUnconfirmed`
on expiry instead of killing or abandoning progressing teardown; hook workers
keep signal-before-reap ownership; `resolve_candidate_outcome` does no I/O and
retains the exact candidate. No product behavior changes.

## What Changes

- A platform-neutral `cfg(test)` progress-aware wait in `kuru-runtime`: it
  ends on its event (a condition, or the watched task finishing), fails at
  once if the task finishes before the condition, re-arms on every observed
  progress signal and reports a stall only when one silent gap exceeds its
  bound. Polling is cadence only.
- The gap bound for dream fixtures is derived from stated budgets:
  `max(turn_admission_deadline(), hook timeout_ms + HookHost quiesce bound)`.
  `turn_admission_deadline()` is the existing runtime test helper (memory
  startup budget, from which the Dolt listener derives its statement read
  timeout). The bound is strictly above the product quiesce bound.
- The cancelled-dream test records step timings, subscribes to harness events
  and keeps the hook host before spawning; both its waits use the new wait.
  A stall or early finish panics with step timings, observed events, in-flight
  hooks, marker and descendant file state, and the candidate inventory.
- `StepTimings::completed()` exposes the mark count as a progress signal.
- `HookHost::quiesce_bound()` (`cfg(any(test, feature = "test-support"))`)
  exposes the existing `QUIESCE` value, following `in_flight_hooks()`.
- Paused-clock regression tests show the old flat 10 s shape failing a
  progressing operation that the new wait accepts, and a stalled operation
  reported with its last step.

## Capabilities

### New Capabilities

None — this is a test-reliability fix, not a new product capability.

### Modified Capabilities

None — no capability's specified behavior was wrong; only the regression
test's own wait derivation was wrong, plus a test-only visibility accessor
for an existing, already-correct product constant.

## Impact

- `packages/kuru-runtime/src/hook_tests.rs`: the cancelled-dream test's waits
  and expiry diagnostics.
- `packages/kuru-runtime/src/progress_wait.rs` (new, `cfg(test)`), `lib.rs`
  module declaration, `step_timings.rs` (`completed()`), `tests.rs`
  (`turn_admission_deadline` visibility).
- `packages/kuru-connectors/src/hooks.rs`: test-support accessor only.
- No `kuru-memory` changes; no product behavior change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
