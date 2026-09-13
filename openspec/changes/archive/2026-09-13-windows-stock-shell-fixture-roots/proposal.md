## Why

The native Windows shell acceptance clears its parent environment but omits stock machine roots that Kuru deliberately preserves, so the fixture demonstrably differs from Kuru's production projection. Repeated cold Windows runs stall inside the real `Get-FileHash` command; restoring the omitted fixture inputs is warranted even though the stall's cause is not yet established.

## What Changes

- Restore the real Windows machine roots already admitted by the built-in shell policy in the isolated CLI fixture.
- Keep hostile module-path input and private per-invocation caches while adding bounded stage evidence that the fixed machine-root inventory reached PowerShell before module discovery.

## Impact

Only `apps/kuru-tui/tests/windows_cli.rs` and its native Windows acceptance evidence change. Production shell behavior, deadlines and retry policy remain unchanged.
