# Tasks

## 1. Event wait for the provider call

- [x] 1.1 Add no helper to `packages/kuru-runtime/src/progress_wait.rs` (it
  stays unchanged; another change is sweeping its waits). The site adopts the
  existing shared helpers: `until_event` with the notification as its
  `reached` condition (`observed.notified().now_or_never()`, which consumes a
  stored `Notify` permit), `TaskWatch::progress` as the re-arming progress
  signal, `Waited::{Reached, Finished, Stalled}` for the three outcomes and
  `settle` for the stalled task. The poll interval stays cadence only; the
  budget is the caller's `gap`, with its derivation stated at the site. Verify
  by the existing paused-clock tests in `progress_wait::tests` that cover each
  arm: a progressing operation beyond a flat bound reaches its event; a task
  finishing first returns its outcome without waiting a gap; a silent operation
  stalls after one gap naming its last completed step.
  Evidence to collect: test names and pass output.
  Evidence: `git diff origin/main -- packages/kuru-runtime/src/progress_wait.rs`
  is empty. In the full package run below, all 10 `progress_wait::tests`
  passed, including the three that cover the arms this site maps:
  `progressing_operation_beyond_a_flat_ten_seconds_reaches_its_event`
  (Reached, and the old flat shape failing a progressing operation),
  `operation_that_finishes_first_returns_its_outcome_without_waiting_a_bound`
  (Finished) and `silent_operation_stalls_after_one_gap_and_names_its_last_step`
  (Stalled). The site states `gap` as `unhooked_gap_bound`, the memory startup
  budget (`MemoryConfig::default().startup_timeout_secs`, 30 s).
- [x] 1.2 In
  `accounting_tests::abandoned_dream_keeps_usage_after_reopen_without_advancing_main`,
  replace the flat `timeout(5 s, observed.notified())` with the 1.1
  composition under the existing `gap`. If the dream task finishes first,
  panic with its outcome (`Ok`/`Err` chain and cancelled flag); on a stall,
  keep the step-recorder report from `explain_expired_dream_wait` with its
  message naming the gap instead of "5 s". Do not touch any other wait. Verify
  by reading the diff (no remaining `from_secs(5)` in the test) and by the test
  passing.
  Evidence to collect: the diff of the site and `rg "from_secs\(5\)"` over the
  test.
  Evidence: the site now matches on `until_event` over the dream task, with
  `futures::FutureExt::now_or_never(observed.notified()).is_some()` as the
  condition, `watch.progress()` as progress and `gap` as the bound: `Reached`
  continues to the cancellation; `Finished(joined)` panics with "the dream task
  finished before its provider call: " and `describe_dream_outcome(joined)`
  plus the step timings; `Stalled { progress_changes }` calls
  `explain_expired_dream_wait`, which now takes the change count and panics
  with "the dream provider call was not reached: the dream made no observable
  progress for {gap:?} ({n} progress changes seen while waiting); last
  completed step: ..." before its unchanged outcome and step listing.
  `rg -n "from_secs\(5\)" accounting_tests.rs` finds one match, line 3544, in
  `cancellation_settles_admitted_usage_without_a_terminal_report`, which is
  outside this change's scope and untouched; there is none in lines 3573 to
  3673 (the test and its failure helper), and no "5 s" text remains in the
  file. The test passed 10 of 10 times alone (2.2).

## 2. Negative check and local evidence

- [x] 2.1 `CancelAfterUsage` has no seam for a forced slow prelude, so do not
  restore the flat 5 s shape or add test-only machinery. The existing
  `progress_wait` paused-clock tests show a flat bound failing an operation that
  never stops progressing while `until_event` reaches its event, and cover the
  finished and stalled arms. At the site, force the stalled arm once locally:
  temporarily remove `observed.notify_one()` from `CancelAfterUsage::stream`
  (do not commit), run the test alone and observe it fail after one silent gap
  with the step-recorder report naming the gap, the progress-change count and
  the last completed step, then restore exactly.
  Evidence to collect: the failure message, its timing and `git diff` showing
  the restore.
  Evidence (temporary, restored): with `self.observed.notify_one();` commented
  out, the test failed at `accounting_tests.rs:3694` with "the dream provider
  call was not reached: the dream made no observable progress for 30s (13
  progress changes seen while waiting); last completed step: first usage
  observation written at +855.7 ms", then "the dream task was still running at
  the stall; after cancellation it finished: Err(turn cancelled); cancelled:
  true" and the step listing from "setup finished; dream task spawning" at
  +38.6 ms through the three actors' "first usage observation written" marks to
  "(time of this report)" at +30871.2 ms, one 30 s gap after the last mark.
  The test binary reported 32.71 s. The file was restored from a saved copy;
  afterwards `grep -n "self.observed.notify_one();"` finds it at line 3511 and
  `git diff` of `accounting_tests.rs` contains only this change's hunks.
- [x] 2.2 Run
  `mise run //packages/kuru-runtime:test -- --lib accounting_tests::abandoned_dream`
  and `progress_wait::`, then the full `mise run //packages/kuru-runtime:test`,
  plus `//packages/kuru-runtime:lint`, `:lint:windows` and `mise run
  format:check`; record pass counts and exit codes. Do not claim local runs
  reproduce the loaded Windows coverage runner.
  Evidence to collect: pass counts, durations and exit codes.
  Evidence (2026-10-04, local macOS arm64, uninstrumented debug build): the
  test alone, `mise run //packages/kuru-runtime:test --
  accounting_tests::abandoned_dream_keeps_usage_after_reopen_without_advancing_main`
  (without `--lib`; the task's `--all-targets` builds only the lib test
  binary for this package), passed 11 of 11 runs, each exit 0; the ten
  repeated runs took 3.20, 3.04, 3.24, 3.45, 3.25, 3.30, 3.75, 3.14, 3.82 and
  2.49 s by the test harness. The full `mise run //packages/kuru-runtime:test`
  exited 0 with 235 passed and 0 failed in 192.63 s, including the 10
  `progress_wait::` tests. `mise run format:check`,
  `//packages/kuru-runtime:lint` and `//packages/kuru-runtime:lint:windows`
  each exited 0. `mise run cospec -- validate dream-accounting-wait-on-events
  --strict` exited 0 and `apply --json` returned gate `clear`. Local runs do
  not reproduce the loaded Windows coverage runner.
- [ ] 2.3 After the PR's CI run, record whether the windows-latest runs of this
  test passed, naming run and job ids; name unrun checks and reasons.
  Evidence to collect: CI run and job ids.
