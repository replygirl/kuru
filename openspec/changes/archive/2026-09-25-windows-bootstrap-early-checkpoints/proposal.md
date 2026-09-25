## Why

The exact P14 Windows installed-runtime check timed out while stock PowerShell was still live after reporting that `install.ps1` had entered. The existing direct checkpoints cannot distinguish whether it stalled before or during the first `Write-Verbose` call.

## What Changes

- Add flushed, `-Verbose`-gated direct checkpoints after strict-mode setup and after the first verbose phase report in the stock installer.
- Require those checkpoints in order in the existing packaged Windows install/update acceptance fixture, so a successful native run verifies the diagnostic route.

## Impact

Touches `packages/kuru-delivery/support/install.ps1` and `apps/kuru-tui/tests/embedded_runtime.rs`. Ordinary non-Verbose installer output, installation authority, deadlines, and acceptance assertions remain unchanged. The final-head native Windows check remains required.
