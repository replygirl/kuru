# Proposal

## Why

Stock Windows PowerShell 5.1 launched by tests with a fresh `LOCALAPPDATA`
starts with a cold module-analysis cache, and its first non-Core command can
park in module discovery past the test bound (#98 measured about 3 stalls in 35
runs before it fixed the delivery installer; #162 fixed the source entrypoint).
Test-authored scripts that reach a Utility or Management command before
anything imports those modules still carry the same stall, as the completion
activation step did on PR #158 (run 36888874521, job 110459524521).

## What Changes

- `packages/kuru-delivery/tests/support/stock_powershell.rs` (new, shared by
  `#[path]` with `apps/kuru-tui/tests`): the two exact `$PSHOME` manifest
  imports already used by both `install.ps1` scripts, and a helper that
  prefixes them to a test-authored script.
- `packages/kuru-delivery/tests/powershell_diagnostics.rs`: a portable contract
  test asserting the prelude is byte-identical to the imports of both
  `install.ps1` scripts and, through #98's
  `assert_pshome_imports_precede_discovered_commands`, that a prefixed script
  reaches no discovered command before both imports.
- `apps/kuru-tui/tests/windows_cli.rs`: the source-entrypoint probe
  (`ConvertTo-Json`, `Write-Output`) and the PE-inspection launcher (whose
  support script first calls `Resolve-Path`) start with the prelude.
- `apps/kuru-tui/tests/embedded_runtime.rs`: the stock completion-activation
  command (`Out-String`, `Invoke-Expression`) starts with the prelude.
- `packages/kuru-delivery/tests/bootstrap_windows.rs`: the architecture probe
  that calls `Write-Output` before the bootstrap starts with the prelude.
- Launches that deliberately exercise module discovery or module-path
  reconstruction, timeouts, product launches and installer-first scripts stay
  unchanged.

No deadline, retry or sleep changes. Shipped code is unchanged.

## Impact

Test files only. Each changed Windows launch imports two modules it would
otherwise autoload; CI time is unchanged in expectation. Windows behaviour is
verified only by native CI; locally the Windows-target lint compiles it.
