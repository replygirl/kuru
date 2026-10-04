# Proposal

## Why

PR #181 (merged as 5f549b39) showed that a flat
`tokio::time::timeout(Duration::from_secs(10), running)` join after
`cancellation.cancel()` is a number guessed at runner speed: on a loaded runner
the cancelled dream's teardown (the product's own bounded hook cleanup) can
exceed it while nothing is stuck, as in CI run 37038146977. #181 fixed the
hook test only; the same shape remains at the sites below. A wait must end on
the event it waits for, bounded by a stated budget.

Sites, from #181's diagnosis (line numbers measured on main 327f817c; the
implementer re-checks each before editing):

- `packages/kuru-runtime/src/accounting_tests.rs` ~3543 and ~3596: join of a
  spawned `run_controlled`/`dream_controlled` task after `cancellation.cancel()`;
  ~3644: the flat 10 s join inside `explain_expired_dream_wait`, after its own
  `cancellation.cancel()`.
- `packages/kuru-runtime/src/dream.rs` `cancellation_tests` ~790: join after
  `cancellation.cancel()` and `release.send(())`.
- `packages/kuru-runtime/src/dream.rs` ~1211: a flat 10 s join on a conflicting
  dream. Observation, not a finding: no cancellation precedes it in the
  excerpt read, so it may fall outside this change; the implementer decides
  from the code and records the decision.

## What Changes

- Each in-scope site waits with the existing cfg(test)
  `progress_wait::until_event` on the same cancellation events #181 used (the
  watched task finishing, re-armed on step marks, harness events and in-flight
  hook count where the fixture has them), under the fixture's existing derived
  gap bound (`dream_gap_bound`: the larger of the memory startup budget and
  hook timeout plus the host's quiesce bound; for fixtures without hooks, the
  memory startup budget via `turn_admission_deadline()`), with the same
  expiry diagnostics (step timings, observed events, in-flight hooks, task
  state).
- Only those sites and shared test support change; shared helpers are added
  only if two or more sites need the same code.
- Paused-clock regression test(s): a cancelled task whose teardown keeps
  progressing past 10 s makes the old flat shape fail and the new wait
  succeed; a stalled teardown is reported with its last step.
- Not changed: product code, what each test asserts after the join, any
  literal raised, any retry.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-runtime/src/accounting_tests.rs`, `dream.rs` (cfg(test)
  modules only), and `progress_wait.rs` or other cfg(test) support as needed.
- Test only; no runtime, connector, platform or memory change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
