# Proposal

## Why

Main run 37036793207 (commit a3de5346, PR #178, which touched only
`kuru-memory` tests), windows-latest coverage partition 1, job 110937162182,
failed `cmd_mise_launches_published_windows_task_wrapper_before_cargo`
(`packages/kuru-delivery/tests/powershell_diagnostics.rs:477-497`):
`bounded_output(&mut child, Duration::from_secs(15), 16 * 1024)` timed out
after `15013 ms` with `stdout_eof=false stderr_eof=false` while the
mise → cmd → pwsh wrapper chain was still starting under coverage
instrumentation (tree at expiry: `mise.exe cpu=328ms`, `cmd.exe`, `pwsh.exe
cpu=671ms working_set=83MB`) — inference, not measured: the instrumented
runner was simply slower to get the wrapper to the point `bounded_output`'s
`capture_until` observes exit, not a hang. The sibling test at line 427 uses
an unrelated flat 30 s for the same cmd → kuru-delivery.exe shape. Neither
literal states what it is bounding against; per the maintainer's standing
principle, a wait must bound a stated budget or observed progress, not a
runner-speed guess, and a CI flake in our own code is a defect, not a rerun
candidate.

## What Changes

- `packages/kuru-delivery/tests/powershell_diagnostics.rs`: replace the two
  flat `Duration::from_secs(15)` / `Duration::from_secs(30)` literals passed
  to `bounded_output` (lines 495 and 427) with one named, derived bound
  shared by both call sites. `bounded_output`'s own wait is already
  event-driven (it ends when both pipes reach EOF and the process tree exits:
  `capture_until` on Unix, `output_with_limit_and_timeout` on Windows, not a
  blind sleep); only the two call-site numbers are unexplained guesses, so
  the derivation lives beside the call sites in this test file, not in
  `command.rs`. The bound, `WRAPPER_LAUNCH_BUDGET`, adopts the package's
  existing 180 s per-launch fixture budget for mise and stock PowerShell
  commands (`DEADLINE` in `support/mise_acceptance.rs` and
  `support/previous_updater.rs`, `TIMEOUT` in `bootstrap_windows.rs`).
- Add one deterministic test, not `cfg(windows)`-gated, that fails if a
  wrapper wait takes anything but that bound, if the bound diverges from the
  package's per-launch budget, or if it no longer expires strictly inside
  the coverage shard's inner deadline (`coverage::shard_deadline` for each
  workflow `KURU_COVERAGE_JOB_MINUTES`).
- No change to `kuru_delivery::command::bounded_output` or
  `output_with_limit_and_timeout` (the "no Unix-side change" non-goal is
  about `command.rs`'s `bounded_unix` implementation specifically), or to
  what either of the two existing `#[cfg(windows)]` tests asserts about the
  wrapper's diagnostic text. The line-427 call site itself does compile and
  run on Unix (it is shared by a non-`cfg(windows)` test), so its bound is
  still in scope for this change, per goal (1) naming both call sites.

## Impact

- Touches only `packages/kuru-delivery/tests/powershell_diagnostics.rs`
  (and, if the derivation needs a shared constant, a `tests/support/`
  module it already uses).
- `docs/development.md` is updated with one sentence only if its existing
  delivery-test documentation turns out to describe these two bounds; initial
  read of the file (searched for `bounded_output`, `powershell_diagnostics`,
  `wrapper`, `coverage partition`, `published-windows`) found no such
  description, so no docs change is expected.
- The line-427 call site (`orchestrator_without_inputs`, used by the
  non-`cfg(windows)` `coverage_orchestrator_refuses_missing_inputs_before_any_effect`
  as well as a `#[cfg(windows)]` sibling) and the new derivation unit test
  on it compile and run on this darwin worktree — their observed pass is
  local evidence. Only the two `#[cfg(windows)]` tests
  (`cmd_mise_launches_published_windows_task_wrapper_before_cargo` and
  `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation`) are
  Windows-only; their evidence is collected from the next native Windows
  coverage run, not reproduced locally.
