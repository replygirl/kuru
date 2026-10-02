# Tasks

Test file: `packages/kuru-delivery/tests/powershell_diagnostics.rs` covers both
wrapper waits (`orchestrator_without_inputs`, shared by the cross-platform
`coverage_orchestrator_refuses_missing_inputs_before_any_effect` and the
`#[cfg(windows)]` `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation`;
and `#[cfg(windows)]` `cmd_mise_launches_published_windows_task_wrapper_before_cargo`)
and the new pin test
`wrapper_waits_take_the_package_launch_budget_inside_the_shard_deadline`.

## 1. Derive one shared bound for both wrapper waits

- [x] 1.1 In `packages/kuru-delivery/tests/powershell_diagnostics.rs`, add one
      named constant whose value is derived from a stated budget, with the
      derivation written beside it, and verify by reading the diff that no
      bare `Duration::from_secs(15)` or `Duration::from_secs(30)` remains at
      either call site and the comment states what it derives from.
      Done: `WRAPPER_LAUNCH_BUDGET` (180 s). No reusable support helper
      exists (the package's per-launch bounds are private constants, and
      importing `support/mise_acceptance.rs` would compile that whole module
      into this test binary), so the constant lives in this file. Its comment
      says truthfully what the value is: the package's 180 s convention for
      mise and PowerShell fixture launches (`DEADLINE` in
      `support/mise_acceptance.rs` and `support/previous_updater.rs`,
      `TIMEOUT` in `bootstrap_windows.rs`), coincident uncommented literals
      that state no derivation of their own; adopting it avoids a fourth,
      different guess, and the pin keeps them equal so a reasoned change to
      any of them forces this bound to be revisited. It describes the actual
      longest shape (mise resolving `verify:published-windows`, the
      `cmd.exe /d /s /c` inline shell, then `pwsh.exe` PowerShell 7
      `-NoProfile` running `support/verify-published-windows.ps1`, which has
      no module prelude and throws on the missing release variables before
      `Get-Command` or `cargo`; the orchestrator shape is a strict subset),
      the measured failure, and that a real stall still fails with
      `bounded_output`'s tree diagnostics at the first expiry in each test.
      What the value must satisfy is checked by the pin test (2.1). Observed
      in the diff.
- [x] 1.2 Replace the `Duration::from_secs(15)` argument to `bounded_output`
      in `cmd_mise_launches_published_windows_task_wrapper_before_cargo`
      with the new bound and verify by reading the diff. Done; its now-unused
      local `use std::time::Duration` was removed.
- [x] 1.3 Replace the `Duration::from_secs(30)` argument to `bounded_output`
      in `orchestrator_without_inputs` with the same bound and verify by
      reading the diff. Done; the orchestrator shape needs no separate value
      because it is a strict subset of the wrapper launch. Its now-unused
      local `use std::time::Duration` was removed.
- [x] 1.4 Confirm neither existing test's assertions on the wrapper's
      diagnostic text changed and verify by reading the diff against the
      pre-change file. Observed: `git diff | grep -E "^[-+].*(Published
      Windows verification requires|MISSING_INPUTS|is not recognized)"`
      printed nothing for the test bodies; the only new `MISSING_INPUTS`
      reference is the `WRAPPER_LAUNCHES` count, which reads its length.
- [x] 1.5 Confirm `bounded_output` and `output_with_limit_and_timeout` in
      `packages/kuru-delivery/src/command.rs` are untouched and verify with
      `git diff`. Observed: `git diff -- packages/kuru-delivery/src` is empty.

## 2. Pin the derived-bound shape with a deterministic test

- [x] 2.1 Add a deterministic, not `cfg(windows)`-gated test that fails if a
      wrapper wait regresses to a flat literal, and verify by running it
      locally and by observing it fail under a mutation. Done:
      `wrapper_waits_take_the_package_launch_budget_inside_the_shard_deadline`
      reads this file's own source (whitespace-normalized) and requires
      exactly two `bounded_output` call sites, both taking
      `WRAPPER_LAUNCH_BUDGET`, and exactly two `orchestrator_without_inputs`
      callers; requires the three sibling package constants to equal the
      budget; and requires `WRAPPER_LAUNCHES` (2 x `MISSING_INPUTS.len()` + 1
      = 7, the Windows launch count; Unix makes a subset) serial budgets,
      7 x 180 = 1260 s, to be strictly below
      `kuru_delivery::coverage::shard_deadline(0, minutes)` (45 min less the
      10 min evidence reserve = 2100 s) for every `KURU_COVERAGE_JOB_MINUTES`
      in `.github/workflows/ci.yml` and `native-tests.yml`. Its comment states
      that the deadline check is a sanity bound for this binary alone, not a
      guarantee, because the shard deadline is shared with every other test
      binary in the partition. Observed locally (darwin): passes. Mutations
      observed, each restored afterward: one call site restored to
      `std::time::Duration::from_secs(15)` failed "a wrapper wait no longer
      takes WRAPPER_LAUNCH_BUDGET, left: 1, right: 2"; the budget set to
      400 s failed "tests/support/mise_acceptance.rs DEADLINE no longer
      matches the package launch convention"; a third
      `orchestrator_without_inputs(task, true)` caller failed "the
      orchestrator launch callers changed; update WRAPPER_LAUNCHES, left: 3,
      right: 2"; `WRAPPER_LAUNCHES` set to 12 failed "a 45-minute coverage
      job leaves 2100 s, within this binary's 12 serial launch budgets
      (2160 s)". No `#[cfg(windows)]` stalled-child test was added:
      `bounded_output`'s expiry diagnostics are unchanged and already covered
      by its own tests in `command.rs`.

## 3. Evidence and docs

- [x] 3.1 Record the measured source, labeling measurements and inferences,
      and verify the evidence quotes the values. Measured (main run
      37036793207, commit a3de5346, #178, windows-latest coverage partition
      1, job 110937162182): `cmd_mise_launches_published_windows_task_wrapper_before_cargo`
      panicked with "read native stdout/stderr after 15013 ms: tool timed
      out; stdout_eof=false stderr_eof=false", tree before cleanup "root
      pid=5144 mise.exe cpu=328ms; pid=8352 cmd.exe; pid=7108 pwsh.exe
      cpu=671ms working_set=83MB". Inference, not measured: about one CPU
      second in fifteen wall seconds means the wrapper was starved while
      starting on a busy coverage runner, not hung (none of mise.exe,
      cmd.exe or pwsh.exe is itself instrumented; only the `kuru-delivery`
      test binary is). Basis of the bound:
      not a measured derivation; it adopts the package's existing 180 s
      per-launch convention, and the checked constraint is that this
      binary's seven serial launch budgets (1260 s) stay under the 2100 s
      shard window.
- [x] 3.2 Re-check `docs/development.md` for an existing description of these
      bounds and verify either one sentence is added or none exists. Observed:
      grep for `bounded_output`, `powershell_diagnostics`, `published-windows`,
      `WRAPPER_LAUNCH` and `wrapper` finds only an unrelated build-cache
      wrapper line; no docs edit was made.
- [x] 3.3 Run the cross-platform call site and the pin test locally and name
      the Windows-only tests as unrun, and verify by recorded results.
      Observed on darwin after the review follow-up: `cargo test -p
      kuru-delivery --all-features --locked --test powershell_diagnostics`
      10 passed, including
      `coverage_orchestrator_refuses_missing_inputs_before_any_effect` and the
      pin test; `//packages/kuru-delivery:test` exit 0 (every result line
      "ok", none failed); `//packages/kuru-delivery:lint` exit 0;
      `//packages/kuru-delivery:typecheck` exit 0;
      `//packages/kuru-delivery:lint:windows` (clippy for
      x86_64-pc-windows-msvc, which compiles the two `#[cfg(windows)]` tests)
      exit 0; `//:format:check` exit 0. Not run here:
      `cmd_mise_launches_published_windows_task_wrapper_before_cargo` and
      `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation`
      are `#[cfg(windows)]` and this worktree is macOS; their native evidence
      is recorded in 3.4.
- [x] 3.4 Record the native Windows evidence for the two `#[cfg(windows)]`
      tests from the windows-latest coverage partitions of the PR head that
      carries the final code, and verify by the `test ... ok` lines in the
      fetched job logs, not by the job's green check. Observed (CI run
      37043921127, head 4f79aadd, the last commit changing code; each job log
      fetched individually): partition 1, job 110961561344, binary started
      18:04:59.568Z, `wrapper_waits_take_the_package_launch_budget_inside_the_shard_deadline
      ... ok` at 18:04:59.644Z and
      `cmd_mise_launches_published_windows_task_wrapper_before_cargo ... ok`
      at 18:05:04.437Z, so that launch completed within 4.9 s of the binary
      starting; partition 3, job 110961561263,
      `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation
      ... ok` 0.23 s after its binary started; partition 2, job
      110961561159, `coverage_orchestrator_refuses_missing_inputs_before_any_effect
      ... ok`. All eight windows-latest coverage partitions concluded
      success (jobs 110961561159, 110961561178, 110961561212, 110961561263,
      110961561272, 110961561327, 110961561344, 110961561387). The earlier
      run 37042689778 (head a2a862ba) was cancelled by the push before its
      Windows partitions finished, so it supplies no evidence. Inference: the
      archive commit that follows changes only `openspec/`, so this evidence
      applies to the code it ships; one passing run is not evidence about
      the starved-start tail the budget exists for.
