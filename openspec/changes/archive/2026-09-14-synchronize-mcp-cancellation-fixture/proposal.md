## Why

The macOS coverage run on `9cd8402` observed an empty transcript from an already
spawned MCP peer before cancellation. The test incorrectly assumed that peer
would be killed before creating its transcript, despite correct cleanup and
successful explicit recovery.

## What Changes

- Synchronize the cancellation test with creation of the first empty transcript.
- Require exactly the cancelled empty transcript and the recovered three-request
  transcript, preserving cancellation, availability, shutdown and cleanup checks.

## Impact

Only `packages/kuru-connectors/src/mcp.rs` test code changes. The test retains
bounded waits and real subprocess coverage; production behavior, deadlines and
the workspace coverage gate remain unchanged.
