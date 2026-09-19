## Why

`apps/kuru-tui/tests/embedded_runtime.rs::packaged_install_and_update_preserve_complete_offline_memory`
stalls intermittently on Windows CI (3/35 application-shard runs examined,
~8.6%; see `plan-flake-windows-installer.md`). All three failures are
byte-identical modulo timestamps: the very first `powershell.exe`
invocation (`install_packaged`) times out after the full 100 s
`COMMAND_TIMEOUT` with zero stdout/stderr bytes — the script's own first
checkpoint line, gated only on `-Verbose` (always passed), never executes.
The evidence rules out PSModulePath/module-autoload and mid-script hangs;
`elapsed` in the timeout error is measured after `spawn().await` already
returned, so the stall is inside the already-created process before it runs
any of its own script — a PowerShell 5.1 engine/host cold-start cost
(loading `System.Management.Automation.dll`, `types.ps1xml`/`format.ps1xml`,
building the runspace) that `-NoProfile`/`-NonInteractive` do not skip. The
install step's 100 s bound is meant to bound the install, not also absorb
that cold start, and on any future occurrence there is currently no
attribution evidence (CPU time, working set) to tell a genuine engine stall
apart from the script/engine spinning.

## What Changes

Give the stock-PowerShell test harness its own distinctly labelled,
generously bounded engine warm-up step (a trivial inline script) ahead of
the timed install invocation, using the exact same launch configuration
(`env_clear` + explicit env set, `-NoProfile -NonInteractive`, the same
absent-PSModulePath handling AGENTS.md documents) so a slow-but-successful
cold start no longer competes with the install step's own 100 s budget, and
a stall there produces a distinct, attributable failure message instead of
the ambiguous install-step one. The install step's own `COMMAND_TIMEOUT` is
unchanged. Separately, instrument `output_with_limit_and_timeout` in
`packages/kuru-delivery/src/command.rs` to sample the child's CPU time and
working set roughly every 10 s while it is within its timeout window (via
new diagnostics-only primitives in `kuru-platform`), and include the
samples in the existing timeout error text next to `stdout_eof`/`stderr_eof`
— purely additive, on the already-slow/failing path only; the success path
and every existing assertion are unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — this corrects the test harness and adds failure-path diagnostics; no
capability spec was wrong.

## Impact

- `apps/kuru-tui/tests/embedded_runtime.rs`: new `warm_up_powershell_engine`
  helper and `ENGINE_WARM_UP_TIMEOUT` constant (Windows-gated), called from
  `install_packaged` before the timed install command. No change to
  `COMMAND_TIMEOUT` or the install assertions.
- `packages/kuru-delivery/src/command.rs` (Windows module): the timeout path
  of `output_with_limit_and_timeout` gains a background diagnostic sampler
  (aborted as soon as the timed future settles, on both success and
  failure) and its error text gains a `samples=[...]` segment. No change to
  the success path or to any timeout value.
- `packages/kuru-platform/src/windows/process.rs`: new
  `NativeChild::duplicate_diagnostic_handle` (query + `PROCESS_VM_READ`
  rights only, no terminate/suspend/write authority) and a new
  `sample_process`/`ProcessSample` pair wrapping `GetProcessTimes` /
  `GetProcessMemoryInfo`. `duplicate_process` is refactored into a shared
  `duplicate_process_with_access` helper with no behavior change for
  existing callers.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
