## Why

The compiler-free Windows installer (`packages/kuru-delivery/support/install.ps1`)
calls a bare stock cmdlet (`Write-Verbose`) as its first command outside
`Microsoft.PowerShell.Core`. On a profile whose PowerShell 5.1 module-analysis
cache is cold, that first use enters `CommandDiscovery.TryModuleAutoDiscovery`,
which analyzes every module on the default path before reaching `$PSHOME`.
Native Windows CI shows the installer printing its `script entered` and
`strict mode ready` checkpoints, burning about 20 s of CPU, then waiting with
flat CPU until the 100-180 s fixture bounds expire (#89 job 108226893444; #91
job 108225753971, two tests at once; #89 c325a3ba job 108178594748). Phase 0's
CDB/SOS stack for the same class (run 34801590851) shows discovery blocked in a
nested `Get-Module -ListAvailable` manifest read. The ToolHost stock shell was
corrected for this on 2026-09-14; the installer never adopted that guard, so a
user running the saved script on a fresh profile can see a long silent delay or
the same indefinite stall.

## What Changes

- Immediately after strict mode, the installer imports the exact
  `$PSHOME\Modules\Microsoft.PowerShell.Management` and
  `Microsoft.PowerShell.Utility` manifests through the always-loaded
  `Microsoft.PowerShell.Core\Import-Module`, with `-ErrorAction Stop` and
  `-Verbose:$false` so the documented `-Verbose` stage output is unchanged.
  These two modules supply every non-Core command the script uses.
- Fixed-text, flushed direct checkpoints follow each import under `-Verbose`,
  so any future stall localizes to the import rather than a later cmdlet.
- Other modules keep ordinary autoloading; the installer's native bridge,
  verification, publication and recovery behavior are unchanged.
- A portable source contract rejects any non-Core installer command that runs
  before the imports or that comes from a module the installer does not import.
  A native fixture runs the real installer with module autoloading disabled.
- The isolated `bootstrap_windows` fixture wrapper stops using a bare
  `Join-Path` before invoking the installer.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-delivery/support/install.ps1`; delivery tests
`powershell_diagnostics.rs` and `bootstrap_windows.rs`; the application
installation fixture's checkpoint list in `apps/kuru-tui/tests/embedded_runtime.rs`;
`docs/install.md`, `apps/kuru-docs/guide/installation.md` and `AGENTS.md`.
No dependency, product timeout, fixture timeout, release asset or Rust runtime
behavior changes. Fixtures keep fresh `LOCALAPPDATA`, which models a new profile.

## Surfaces

- [x] interactive — first-run behavior of the documented Windows PowerShell installer
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — binds the installer to the exact modules shipped under stock PowerShell's `$PSHOME`
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
