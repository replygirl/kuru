## Why

The archived MCP catalog controls still need one deterministic application-level proof that CLI and TUI inspection agree on mixed alias state and that one provider response cannot dispatch stale or denied MCP calls.

## What Changes

- Extend the Unix terminal integration fixture with a test-owned stdio MCP peer and exact effect witnesses.
- Exercise real `kuru tools`, registered `/tools`, and one fake-provider live/stale/denied tool-call batch over the same checked configuration.

## Impact

Only `apps/kuru-tui/tests/terminal.rs` and verification artifacts change. The fixture adds one bounded Unix PTY case and no production behavior or paid provider request.
