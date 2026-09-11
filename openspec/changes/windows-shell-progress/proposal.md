## Why

The native Windows shell fixture timed out with a still-running PowerShell process and no output in CI 34602482369. Its first output occurs after file hashing, so the failure cannot distinguish interpreter startup from module import or command execution.

## What Changes

Add private fixed-stage markers to the existing apps/kuru-tui/tests/windows_cli.rs control and Kuru shell invocations. Keep distinct marker files and assert the observed sequence while preserving real hashing, environment retention, file identity, negative module-import control and timeout/cleanup behavior.

## Impact

Only the existing Windows shell acceptance fixture gains diagnostic file writes. No product behavior, timeouts, dependencies, CI topology or runtime evaluation changes.
