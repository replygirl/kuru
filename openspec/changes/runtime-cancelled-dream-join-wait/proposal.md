# Proposal

## Why

`kuru-runtime`'s
`hook_tests::cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`
failed in CI with `Elapsed(())` (PR #133 run 37038146977, head `f8a107fc` on
main `a3de5346`, macos-latest coverage partition 2, job `110941628749`: 53
passed, 1 failed in 53.42s). The failing assertion, confirmed by reading the
job's own log output (not inferred from the stack text), is
`hook_tests.rs:1765:6` — the close of the **marker wait** at lines 1759-1765
(`tokio::time::timeout(Duration::from_secs(10), async { while
!marker.exists() { ... } }).await.unwrap()`), not the dream-join wait two
lines later at 1767-1770 as a first reading of the symptom suggests. The exact
commit (`f8a107fc`) carries byte-identical lines 1755-1772, so this is a
measured fact, not a guess from an approximate line number.

That marker wait is a single flat 10-second deadline covering an entire
multi-stage pipeline that has to run before the marker is even written:
`reconcile()`, `acquire_dream_lease()`, `begin_candidate("dream")` (a Dolt
write), the fake provider's dream-proposal completion, dispatch and settle of
the **first** of two tool calls together with its post-tool hook (bounded by
that hook's own stated `timeout_ms: 5_000`, i.e. half the test's total budget
already spent on one component), `finish_post_tool_hooks` (another Dolt
write, the candidate annotation), and only then dispatch of the **second**
tool call whose shell hook writes the marker as its first action. None of
that preceding Dolt/provider work is bounded by any budget the test states or
derives — the 10 seconds is a number guessed at the runner's expected speed,
not a sum of the stated costs it has to clear. On a loaded macOS coverage
partition (an instrumented build, several concurrent test binaries, real Dolt
I/O) that pipeline can exceed 10 seconds even though nothing is actually
stuck: the test fails by construction, independent of whether the product
code is behaving correctly.

Separately — and this is a labelled inference, not something the failing run
exercised — the same file's dream-join wait two lines later
(`tokio::time::timeout(Duration::from_secs(10), running)`, lines 1767-1770)
encloses the product's own cancellation-path hook-cleanup wait
(`Harness::await_hook_cleanup` in `dream.rs:247`, which calls
`HookHost::quiesce()` in `kuru-connectors/src/hooks.rs:421-441`, bounded by
the private `QUIESCE = Duration::from_secs(10)` constant at `hooks.rs:39`,
itself derived as `2 * CLEANUP` with its rationale stated in the adjacent
doc comment). A test wait whose bound is numerically equal to, rather than
strictly larger than, the product wait it encloses will eventually lose that
race under load even when the product is behaving exactly to its own
documented contract (`quiesce()` degrades to a reported `CleanupUnconfirmed`
error on expiry; it does not hang or kill in-progress cleanup). This did not
fire in the captured failure — the panic location rules it out for this
run — but it is the same class of defect (a flat number racing a stated
product budget with no margin) on the same cancellation path, so it is
fixed in the same change rather than left for the next flake to find.

## What Changes

- The marker wait in `cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`
  stops using one flat, undersized guess for a multi-stage pipeline. It
  splits into (a) an event-driven wait, with no flat deadline raced against
  the pipeline's unbounded Dolt/provider work, for the one externally
  observable progress signal that pipeline actually produces before the
  marker — the first post-tool hook's own `"annotated"` observation on the
  harness's event stream — guarded only by a generous deadlock backstop, not
  a contested number; and (b) a short, explicitly derived bound for the one
  remaining step that *is* budgeted — launching the second hook — stated in
  terms of that hook's own configured `timeout_ms` plus a named scheduling
  margin, instead of a bare `10`.
- The dream-join wait in the same test is changed from a flat `10` to a bound
  derived from the product's own `quiesce` cleanup bound (exposed from
  `kuru-connectors` for test use, test-support-gated, following the existing
  `HookHost::in_flight_hooks()` pattern) plus a stated margin, so the test
  can no longer race the product's own documented worst case to a tie.
- Both waits report concrete diagnostics on expiry (in-flight hook count,
  marker/survived-file state, and the harness's step-timing marks captured
  before the dream task was spawned) instead of a bare `Elapsed(())`.
- No product cancellation-path behavior changes: `quiesce()`'s own bound is
  already progress-aware (`wait_for(|state| state.active == 0)`) and
  explicitly derived in its doc comment; it reports uncertainty on expiry
  rather than abandoning in-progress teardown, so it already satisfies the
  "never a flat number that abandons progressing teardown" bar and needs no
  behavioral change — only its bound becomes visible to the test that must
  not race it.

## Capabilities

### New Capabilities

None — this is a test-reliability fix, not a new product capability.

### Modified Capabilities

None — no capability's specified behavior was wrong; only the regression
test's own wait derivation was wrong, plus a test-only visibility accessor
for an existing, already-correct product constant.

## Impact

- `packages/kuru-runtime/src/hook_tests.rs` —
  `cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`:
  both waits re-derived; diagnostics added on expiry.
- `packages/kuru-connectors/src/hooks.rs` — a `#[cfg(any(test, feature =
  "test-support"))]` accessor exposing the `HookHost::quiesce()` cleanup
  bound (currently the private `QUIESCE` constant), following the existing
  `in_flight_hooks()` visibility pattern at `hooks.rs:447`. No change to
  `quiesce()`'s behavior or bound value.
- No `kuru-memory` changes.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
