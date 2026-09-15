## Why

The current main Windows packaged-runtime fixture reached its existing 100-second command deadline while the stock PowerShell bootstrap produced no output, so the bootstrap phase where it stopped is unknown. Bounded, path-free verbose phase markers will make a recurrence actionable without changing installer behavior.

## What Changes

- Add fixed `Write-Verbose` phase markers to the existing Windows PowerShell bootstrap at its major bootstrap boundaries.
- Run the native successful-bootstrap fixture with `-Verbose` and require the fixed markers in order.
- Document that `-Verbose` reports bootstrap stages for troubleshooting.

## Impact

This diagnostic change affects `packages/kuru-delivery/support/install.ps1`, its native Windows bootstrap fixture, and `docs/install.md`. The existing 100-second packaged-runtime deadline, subprocess capture and cleanup, ordinary output, installation semantics, and retry behavior remain unchanged; actual marker execution and ordering require hosted Windows verification.
