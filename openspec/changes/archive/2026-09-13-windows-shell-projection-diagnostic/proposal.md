## Why

Windows native coverage timed out in the existing projected-shell fixture after its first stage, leaving the failing ToolHost launch authoritative but without enough bounded evidence to localize the stall.

## What Changes

- Add failure-only direct stock-PowerShell controls to `packages/kuru-connectors/src/tools.rs`, preserving the existing ToolHost assertion and using the same projected environment and payload construction to distinguish command transport and console selection.
- Record bounded stages around the first `Join-Path` and retain original failure reporting after all controls clean up.

## Impact

Windows-only fixture coverage changes; no production behavior, timeout, or acceptance criterion changes.
