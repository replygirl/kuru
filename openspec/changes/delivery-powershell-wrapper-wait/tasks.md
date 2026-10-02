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
      exists (the package's per-launch budgets are private constants, and
      importing `support/mise_acceptance.rs` would compile that whole module
      into this test binary), so the constant lives in this file and adopts
      the delivery package's existing per-launch fixture budget for mise and
      stock PowerShell commands: `DEADLINE` in `support/mise_acceptance.rs`
      and `support/previous_updater.rs`, `TIMEOUT` in `bootstrap_windows.rs`,
      each 180 s. Its comment names what it covers (mise start and task
      resolution, the `cmd.exe` inline shell, stock PowerShell start with its
      module prelude, the wrapper's diagnostic write; the orchestrator shape
      is a strict subset), the measured failure, and why expiry still yields
      `bounded_output`'s tree diagnostics (one budget sits far inside the
      coverage shard's inner deadline, `shard_deadline` = 45 min job limit
      less the 10 min `EVIDENCE_RESERVE` = 2100 s). Observed in the diff.
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
      printed nothing.
- [x] 1.5 Confirm `bounded_output` and `output_with_limit_and_timeout` in
      `packages/kuru-delivery/src/command.rs` are untouched and verify with
      `git diff`. Observed: `git diff -- packages/kuru-delivery/src` is empty.

## 2. Pin the derived-bound shape with a deterministic test

- [x] 2.1 Add a deterministic, not `cfg(windows)`-gated test that fails if a
      wrapper wait regresses to a flat literal, and verify by running it
      locally and by observing it fail under a mutation. Done:
      `wrapper_waits_take_the_package_launch_budget_inside_the_shard_deadline`
      reads this file's own source (whitespace-normalized) and requires at
      least two `bounded_output` call sites, every one taking
      `WRAPPER_LAUNCH_BUDGET`; requires the three sibling package budgets to
      equal it; and requires it to be strictly below
      `kuru_delivery::coverage::shard_deadline(0, minutes)` for every
      `KURU_COVERAGE_JOB_MINUTES` in `.github/workflows/ci.yml` and
      `native-tests.yml`. Observed locally (darwin): passes. Mutation
      observed: with one call site temporarily restored to
      `std::time::Duration::from_secs(15)` it failed with "a wrapper wait no
      longer takes WRAPPER_LAUNCH_BUDGET, left: 1, right: 2"; the file was
      then restored. No `#[cfg(windows)]` stalled-child test was added:
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
      starting under coverage instrumentation, not hung. Derivation: the
      bound adopts the package's 180 s per-launch fixture budget (12 times
      the expired 15 s) and stays under the 2100 s shard window.
- [x] 3.2 Re-check `docs/development.md` for an existing description of these
      bounds and verify either one sentence is added or none exists. Observed:
      grep for `bounded_output`, `powershell_diagnostics`, `published-windows`,
      `WRAPPER_LAUNCH` and `wrapper` finds only an unrelated build-cache
      wrapper line; no docs edit was made.
- [x] 3.3 Run the cross-platform call site and the pin test locally and name
      the Windows-only tests as unrun, and verify by recorded results.
      Observed on darwin: `cargo test -p kuru-delivery --all-features --locked
      --test powershell_diagnostics` 10 passed, including
      `coverage_orchestrator_refuses_missing_inputs_before_any_effect` and the
      pin test; `mise run //packages/kuru-delivery:test` exit 0;
      `//packages/kuru-delivery:lint` exit 0; `//packages/kuru-delivery:typecheck`
      exit 0; `//packages/kuru-delivery:lint:windows` (clippy for
      x86_64-pc-windows-msvc, which compiles the two `#[cfg(windows)]` tests)
      exit 0; `format:check` exit 0. Not run here:
      `cmd_mise_launches_published_windows_task_wrapper_before_cargo` and
      `cmd_launches_the_exact_coverage_tasks_and_reaches_input_validation`
      are `#[cfg(windows)]` and this worktree is macOS; their evidence comes
      from the PR's native Windows coverage partitions.
