# Tasks

## 1. Expose the product's own cleanup bound for test use

- [x] 1.1 Add a `#[cfg(any(test, feature = "test-support"))]` accessor on
  `HookHost` in `packages/kuru-connectors/src/hooks.rs` that returns the
  `quiesce()` cleanup bound (the private `QUIESCE` constant), mirroring the
  existing `in_flight_hooks()` visibility pattern. No change to `quiesce()`'s
  behavior or value. Verify by `mise run //packages/kuru-connectors:typecheck`
  and the runtime tests resolving the accessor.
  Evidence (2026-10-02, local macOS): `//packages/kuru-connectors:typecheck`, `:lint` and `:lint:windows` exit 0; runtime tests use `quiesce_bound()`.

## 2. One progress-aware fixture wait with a derived gap bound

- [x] 2.1 Add a platform-neutral `cfg(test)` module in `kuru-runtime` with a
  wait that ends on its event (a condition such as the marker file, or the
  watched task finishing), fails at once if the task finishes before the
  condition, re-arms its bound on every observed progress signal, and reports
  a stall only when one silent gap exceeds its bound. The poll interval is
  cadence only. Verify by the module's paused-clock tests.
  Evidence: `progress_wait::until_event`; 4 paused-clock tests in `progress_wait::tests` pass.
- [x] 2.2 Derive the gap bound for a dream fixture from stated budgets only:
  `max(turn_admission_deadline() [memory startup budget, from which the Dolt
  listener derives its statement read timeout], hook timeout_ms + HookHost
  quiesce bound)`, read from the fixture's actual `HookCommand` and host, with
  the derivation stated in a comment. Verify by a unit assertion that the
  bound is strictly greater than the quiesce bound it must enclose.
  Evidence: `dream_gap_bound`; `dream_gap_bound_takes_the_larger_stated_budget_above_quiesce` passes and the fixture asserts `gap > quiesce_bound()` (30 s > 10 s locally).
- [x] 2.3 Add a `StepTimings::completed()` count so recorded marks act as
  progress. Verify by its use in the paused-clock tests (all platforms).
  Evidence: used by the paused-clock tests; `//packages/kuru-runtime:lint:windows` exit 0 (no dead code).

## 3. Re-derive both waits in the cancelled-dream hook test

- [x] 3.1 In
  `cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`,
  record step timings on the harness, subscribe to its events and keep its
  hook host before spawning the dream; replace the flat 10 s marker poll with
  the progress-aware wait (condition: marker exists; progress: step marks,
  harness events, in-flight hook count). Verify by the test passing locally.
  Evidence: passes alone and in `hook_tests::` (22 passed); a temporary success report showed the marker at +4640 ms.
- [x] 3.2 Replace the flat 10 s dream-join wait with the same wait
  (condition: the task finishes) under the same derived gap bound, strictly
  above the product quiesce bound it encloses. Verify by reading the diff and
  the unit assertion in 2.2.
  Evidence: join uses `until_event(.., || false, ..)` with the same `gap`; assertion holds.
- [x] 3.3 On a stall or an early finish, panic with what the dream was doing:
  step timings, observed events, in-flight hooks, marker and descendant file
  state, and the candidate inventory (read under the same bound, reported as
  unavailable on expiry); a stalled dream is then cancelled and awaited under
  the same bound, and aborted if it does not finish. Verify by reading the
  panic path and by the stall paused-clock test's message.
  Evidence: temporary forced stall (first hook `sleep 3`, gap 1 s; not committed) panicked with steps, `in-flight hook workers: 1`, file state, candidate inventory, events and "after cancellation the dream finished: Err(turn cancelled)".

## 4. Deterministic regression and local evidence

- [x] 4.1 Paused-clock regression test: a synthetic operation that keeps
  progressing within its gap bound but needs more than 10 s to reach its
  condition. The old shape (`timeout(10 s, poll)`) expires on it; the new
  wait succeeds. A stalled operation fails naming its last completed step.
  Verify by running the test, and by temporarily swapping the old shape in
  and observing the failure (record, do not commit).
  Evidence: test passes; with the old shape swapped in it failed: "old flat 10 s wait expired on a progressing operation" (restored).
- [x] 4.2 Run `mise run //packages/kuru-runtime:test -- --lib hook_tests::`
  and the full `mise run //packages/kuru-runtime:test`; record pass counts.
  Do not claim this reproduces the loaded CI runner.
  Evidence: `hook_tests::` 22 passed in 32.45 s; full `//packages/kuru-runtime:test` 224 passed, 0 failed in 262.88 s. Local runs do not reproduce the loaded macOS coverage runner.
