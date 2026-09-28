# Tasks

## 1. Reproduce before changing

- [x] 1.1 Temporarily gate the worker in `retained_owner_holds_process_wide_admission_across_dropped_registries` until its 25 ms deadline has passed. Verify that the test fails at the CI assertion with `left: "shell timed out; stderr: <pending EOF>"`, save the log, and revert the patch.

## 2. Launch-anchored deadline seam

- [x] 2.1 Add the `#[cfg(test)]` launch anchor to `packages/kuru-connectors/src/unix_shell.rs`: skip the pre-launch deadline check while cancellation checks continue, re-anchor the worker deadline at spawn, and move the caller fallback to the same instant. Verify that `cargo check -p kuru-connectors` for the non-test build and clippy both stay clean.
- [x] 2.2 Arm the anchor in the four exposed post-spawn timeout tests and assert one launch each. Verify that each test passes and that its durations and assertions are unchanged.
- [x] 2.3 Add `pre_launch_deadline_expiry_fails_without_launching`, which gates the worker until the deadline passes. Verify that it asserts the plain pre-launch timeout, a reclaimed owner and admission, no launch, and no marker file.

## 3. Verification

- [x] 3.1 Run the repro patch again on top of the anchored test. Verify that it passes, because the pre-launch delay can no longer expire the deadline.
- [x] 3.2 Run `//packages/kuru-connectors:test`, then loop the `unix_shell` tests 20 times at `--test-threads=2` and 20 times at `--test-threads=8`. Record the observed results.
- [x] 3.3 Run `format:check`, `//packages/kuru-connectors:lint`, `//packages/kuru-connectors:typecheck`, `lint:shell`, `//apps/kuru-docs:lint`, `lint:tooling`, `docs:check` and `cospec validate --strict`, and record the observed results. The root `lint:rust` and `typecheck` aggregates are not run; see the not-run list below.

## Audit

- The four tests at the named exposure use short durations and assert a
  post-spawn `cleanup: unconfirmed ownership retained` outcome: 30, 25, 25 and
  10 ms. All four now arm the anchor.
- The same shape at lower exposure uses 1 s durations and asserts either a
  specific injected pre-launch outcome or a post-spawn outcome. These tests are
  `pre_spawn_panic_reclaims_only_its_registered_reservation`,
  `before_spawn_failures_reclaim_their_reservation_without_launching`,
  `post_spawn_failures_take_the_common_cleanup_path` and
  `successful_capture_never_reports_success_before_unconfirmed_cleanup`. They
  also arm the anchor and assert their launch count: 0 for the two pre-launch
  tests and 1 for the two post-spawn tests.
- Some tests are unchanged on purpose. In
  `caller_fallback_prevents_a_delayed_worker_from_launching` and
  `shutdown_cancels_a_starting_owner_and_closes_registration`, the start gate
  holds the worker, and the pre-launch result is the behaviour under test.
  `confirmed_completion_removes_only_its_reservation_before_waking_receiver`
  launches no worker.
- The shell timeout tests in `tools.rs` either wait for a readiness marker
  written by the running shell or accept any `timed out` text. The Windows
  shell starts `timeout(duration, …)` only after `spawn()`, so it has no
  pre-launch deadline. `unix_shell_outlives_a_destroyed_parent_runtime` uses
  `timeout_ms` 120000 with a readiness marker, so it is not exposed.
  `unix_shell_timeout_projects_a_fixed_failure_without_captured_stderr` and
  `unix_shell_timeout_keeps_eof_complete_redacted_stderr` use `timeout_ms:
  3_000`, with the deadline starting at acceptance and a 5 s bounded wait for
  the readiness marker. These are **residual lower exposure**, not unexposed:
  a pre-launch stall past 3 s still fails them (the readiness marker is never
  written, so `ready_result` expires), the same class of exposure this change
  anchors in `unix_shell.rs`, only lower because it needs a multi-second
  stall instead of a 10–30 ms one. Anchoring them would need a `pub(crate)`
  accessor on `ShellRegistry` analogous to the test-only launch anchor; that
  is follow-on work, out of scope here.
- Known limit, pre-existing and outside this change:
  `retained_cleanup_uses_capped_exponential_observation_backoff`'s caller
  fallback lands about 20 ms (the test's budget) after the launch deadline,
  while the worker records the primary failure category on its next
  read-loop observation, at most about 10 ms after the deadline plus
  scheduling. A stall exceeding roughly that 10 ms window there can still
  make the caller return `shell cleanup unconfirmed; …` instead of the
  primary category. The launch anchor narrows this race (the fallback
  previously counted from acceptance, not from launch) but does not close
  it; closing it is out of scope for this change.

## Observed evidence

Local macOS (aarch64-apple-darwin, 14 CPUs), branch
`test/unix-shell-prelaunch-deadline`, 2026-09-28:

- Red before (1.1): with the temporary start-gate delay and the original test, `unix_shell::tests::retained_owner_holds_process_wide_admission_across_dropped_registries ... FAILED` at the CI assertion, with `left: "shell timed out; stderr: <pending EOF>"` and `right: "shell timed out; cleanup: unconfirmed ownership retained; stderr: <pending EOF>"`. This matches the text of CI job 108741816441.
- Green after (3.1): the same delay on the anchored test gives `... ok` (1 passed).
- `unix_shell::` module: 12 passed. Loops at `--test-threads=2` passed 20 of 20, and at `--test-threads=8` passed 20 of 20. Under a load of 28 `yes` CPU burners on 14 CPUs, `--test-threads=8` passed 10 of 10.
- `mise run //packages/kuru-connectors:test`: 297 lib tests passed, 0 failed, exit 0.
- `mise run //packages/kuru-connectors:lint` (clippy `-D warnings`, all targets and features) exited 0. `mise run //packages/kuru-connectors:typecheck` (all targets, which includes the non-test lib) exited 0.
- `mise run format:check`, `lint:shell`, `//apps/kuru-docs:lint`, `lint:tooling` and `docs:check` each exited 0.
- `mise run cospec -- validate unix-shell-launch-anchored-deadline --strict` passed with 0 errors and 0 warnings.

Not run, by name:

- The root `lint:rust` and `typecheck` aggregates over every other package.
  Disk is tight, the only change is `#[cfg(test)]` code in one package, and
  its dependents never compile that code.
- Coverage and native CI on macOS and Linux. Neither can run without a push,
  and this change makes none. Only repeated CI coverage runs can show that
  this failure is gone from main.
