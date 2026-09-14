## Why

The failure-only PowerShell controls establish that direct launches can complete,
but they do not show whether recurring original stalls occur while PowerShell
imports a module or later while it executes the first stock cmdlet. The original
acceptance must remain authoritative and unchanged.

## What Changes

- Replace each three-arm failure-only matrix with one encoded, inherited-console
  control that imports its fixed stock module by name through the Core-qualified
  cmdlet before the existing first cmdlet.
- Record bounded verbose import text and before/after import stages through the
  existing diagnostic paths in connector and TUI fixtures.

## Impact

- Test-only changes in `packages/kuru-connectors/src/tools.rs` and
  `apps/kuru-tui/tests/windows_cli.rs`.
- No production behavior, deadline, dependency, or CI topology change.
