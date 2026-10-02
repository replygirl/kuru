# Tasks

## 1. Derive one shared bound for both wrapper waits

- [ ] 1.1 In `packages/kuru-delivery/tests/powershell_diagnostics.rs`, add one
      named constant (or a small helper if the derivation needs arithmetic)
      whose value is computed from a stated budget — e.g. the coverage job's
      own `KURU_COVERAGE_JOB_MINUTES` budget referenced in `coverage.rs`, or,
      if that env is absent for these two plain `#[tokio::test]`s, a written
      derivation from the measured 15013 ms failure and the sibling's 30 s
      (for example: worst observed start time × a stated safety factor, with
      the factor and the measured input named in a comment beside the
      constant, in the repo's existing "Bounds and their derivation" style
      used in `docs/development.md`'s usage-scan calibration). Verify by
      reading the diff: no bare `Duration::from_secs(15)` or
      `Duration::from_secs(30)` remains at either call site, and the comment
      states what number it derives from.
- [ ] 1.2 Replace the `Duration::from_secs(15)` argument to `bounded_output`
      in `cmd_mise_launches_published_windows_task_wrapper_before_cargo`
      (line 495) with the new bound. Verify by reading the diff.
- [ ] 1.3 Replace the `Duration::from_secs(30)` argument to `bounded_output`
      in `orchestrator_without_inputs` (line 427, used by both
      `coverage_orchestrator_refuses_missing_inputs_before_any_effect`, a
      plain `#[tokio::test]` with no `cfg(windows)` that also compiles and
      runs on Unix, and the `#[cfg(windows)]`
      `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation`)
      with the same bound, unless its own call shape needs a separately
      derived value — if so, derive it the same way and say why it differs.
      This is the one call site the "no Unix-side change" non-goal does not
      cover: that non-goal is about `command.rs`'s `bounded_unix`
      implementation, not about leaving line 427 untouched, since goal (1)
      names both call sites explicitly. Verify by reading the diff.
- [ ] 1.4 Confirm neither existing test's assertions on the wrapper's
      diagnostic text (`"Published Windows verification requires"`, the
      `MISSING_INPUTS` diagnostics, the absence of
      `"is not recognized as an internal or external command"`) changed.
      Verify by reading the diff against the pre-change file.
- [ ] 1.5 Confirm `bounded_output` (`packages/kuru-delivery/src/command.rs`)
      and `output_with_limit_and_timeout` are untouched. Verify with
      `git diff --stat` showing no change under `src/command.rs`.

## 2. Pin the derived-bound shape with a deterministic test

- [ ] 2.1 Add a unit test on the constant/helper itself (not gated to
      `cfg(windows)`, since the value it computes is also used by the
      Unix-compiled call site at 1.3) that fails if the derivation's inputs
      or arithmetic silently regress to an unexplained flat literal. This
      test is runnable and must actually be run on this darwin worktree
      (`mise run //packages/kuru-delivery:test`, tooling feature) as local
      evidence. Optionally also add a `#[cfg(windows)]` integration test
      with a stalled child proving `bounded_output` still reports its tree
      diagnostics on expiry under the derived bound; that one is Windows-only
      and cannot run here. Verify: the unit test's pass/fail is observed
      locally; the optional Windows integration test, if added, is named as
      not run here with the reason.

## 3. Evidence and docs

- [ ] 3.1 Record in the change's evidence (or PR body) the measured source:
      run 37036793207, job 110937162182, commit a3de5346 (#178), the
      `15013 ms` elapsed with `stdout_eof=false stderr_eof=false`, and the
      tree at expiry (`mise.exe cpu=328ms`, `cmd.exe`, `pwsh.exe
      cpu=671ms working_set=83MB`), each labeled measured; label as
      inference the claim that the wrapper was merely slow to start rather
      than hung. Verify: evidence text quotes these values rather than
      paraphrasing them.
- [ ] 3.2 Re-check `docs/development.md` for any existing description of
      these two bounds before writing anything there. Verify: either one
      sentence is added where such a description exists, or the change
      records that none exists and no docs edit was made.
- [ ] 3.3 Run `coverage_orchestrator_refuses_missing_inputs_before_any_effect`
      (line 427's call site, no `cfg(windows)`) and the new 2.1 unit test
      locally on this darwin worktree and record the observed pass as
      evidence. Collect the next native Windows coverage run's result for
      the two `#[cfg(windows)]` tests —
      `cmd_mise_launches_published_windows_task_wrapper_before_cargo` and
      `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation` —
      separately, naming them explicitly as unrun here (darwin worktree,
      `#[cfg(windows)]`) rather than manufacturing a pass for them.
