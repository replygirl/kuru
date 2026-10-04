# Tasks

## 1. Event wait for the provider call

- [ ] 1.1 In `packages/kuru-runtime/src/progress_wait.rs`, add a `cfg(test)`
  wait that ends on a `Notify` notification, on the watched task finishing
  first (returning its join result), or on one silent gap with no progress
  change, re-armed by `TaskWatch::progress`. Compose it from `until_event` or
  an event select; the poll interval stays cadence only. The budget is the
  caller's `gap`, with its derivation stated at the site. Verify by paused-clock
  tests in `progress_wait::tests`: a progressing operation needing more than 5 s
  reaches the notification; a task finishing first returns its outcome without
  waiting a gap; a silent operation fails naming its last completed step.
  Evidence to collect: test names and pass output.
- [ ] 1.2 In
  `accounting_tests::abandoned_dream_keeps_usage_after_reopen_without_advancing_main`,
  replace the flat `timeout(5 s, observed.notified())` with the wait from 1.1
  under the existing `gap`. If the dream task finishes first, panic with its
  outcome (`Ok`/`Err` chain and cancelled flag); on a stall, keep the
  step-recorder report from `explain_expired_dream_wait` with its message naming
  the gap instead of "5 s". Do not touch any other wait. Verify by reading the
  diff (no remaining `from_secs(5)` in the test) and by the test passing.
  Evidence to collect: the diff of the site and `rg "from_secs\(5\)"` over the
  test.

## 2. Negative check and local evidence

- [ ] 2.1 Swap the old flat 5 s shape back in temporarily (do not commit) and
  observe the paused-clock regression from 1.1 fail on a progressing operation
  that needs more than 5 s; record the failure text, then restore.
  Evidence to collect: the failure message and `git diff` showing the restore.
- [ ] 2.2 Run
  `mise run //packages/kuru-runtime:test -- --lib accounting_tests::abandoned_dream`
  and `progress_wait::`, then the full `mise run //packages/kuru-runtime:test`,
  plus `//packages/kuru-runtime:lint`, `:lint:windows` and `mise run
  format:check`; record pass counts and exit codes. Do not claim local runs
  reproduce the loaded Windows coverage runner.
  Evidence to collect: pass counts, durations and exit codes.
- [ ] 2.3 After the PR's CI run, record whether the windows-latest runs of this
  test passed, naming run and job ids; name unrun checks and reasons.
  Evidence to collect: CI run and job ids.
