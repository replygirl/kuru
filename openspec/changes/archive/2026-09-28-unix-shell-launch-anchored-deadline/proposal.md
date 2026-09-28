# Proposal

## Why

Four retained Unix shell tests give a command a 10–30 ms timeout and assert the
post-spawn `cleanup: unconfirmed ownership retained` failure. The deadline starts
at acceptance, so under load it can expire before the shell spawns; the worker
then returns the plain pre-launch `shell timed out` error and the path under test
never runs (main run 36362325789, macOS coverage partition 4).

## What Changes

- `packages/kuru-connectors/src/unix_shell.rs` test support gains a `#[cfg(test)]`
  launch anchor. When a test arms it, the worker's pre-launch checks observe
  cancellation but not the command deadline. At spawn the worker measures the
  command duration from that instant and hands the launch deadline to the
  caller, whose fallback is anchored at the same instant. Release builds
  compile the same `execute`, `worker` and `before_launch` behaviour as before.
  The caller's wait for that launch (or an earlier pre-launch finish) is
  itself bounded by a new `LAUNCH_ANCHOR_BOUND` (10 s). This is a new
  test-only diagnostic bound, not a raised product timeout: no existing
  deadline, budget or backoff changes. If it expires, the caller cancels the
  owner and returns a named `"test launch anchor: …"` error instead of
  hanging; that expiry arm is not exercised by any test in this change
  (uncovered `cfg(test)` code).
- The tests `cleanup_panic_keeps_the_timeout_primary_and_retains_the_owner`,
  `retained_owner_holds_process_wide_admission_across_dropped_registries`,
  `delayed_interruption_retains_the_real_worker_then_confirms_once` and
  `retained_cleanup_uses_capped_exponential_observation_backoff` arm the anchor
  and assert exactly one launch. Their durations, budgets and assertions stay
  unchanged.
- The new `pre_launch_deadline_expiry_fails_without_launching` test holds a
  worker at the start gate until the command deadline has passed. It then
  asserts the plain pre-launch timeout, reclaimed reservation and admission,
  no launch, and no side effect. This gives the pre-launch timeout path its
  own deterministic test.

## Impact

Test-only. The change touches one source file. It adds one unit test of about
20 ms and does not change CI time or the coverage configuration. The Windows
shell (`tools.rs`) starts its timeout only after the process spawns, so it does
not have this pattern.
