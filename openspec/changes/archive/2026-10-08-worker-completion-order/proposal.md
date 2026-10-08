# Proposal

## Why

Release run 37771276789 exposed a completed lifecycle hook whose caller resumed
before the worker released its budget lease. The same result-before-release
ordering exists in Unix and Windows shell operation holds and MCP startup,
request, send and close holds; it can charge finished work or leave a completed
operation briefly retaining a session drain barrier.

## What Changes

- Release confirmed hook leases and worker slots before exposing their result.
- Settle shell operation holds before confirmed completion or registry removal.
- Hand confirmed MCP operation holds to the reply receiver, which releases them
  before returning; keep a failed delivery's hold through native cleanup.
- Preserve early bounded refusal and retained ownership when cleanup cannot yet
  be confirmed, and cover both ordinary completion and retained cleanup.
- Audit other native worker replies for this class without changing intentional
  ownership handoffs or already joined workers.
- Release the session-selection worker's temporary lease clone before its reply,
  leaving the pending/current driver lease as the sole transition owner.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing ownership and budget specifications already require this behavior.

## Impact

Connector lifecycle hooks, Unix and Windows shells, MCP stdio worker completion,
memory session-selection completion,
their native/ownership regression tests, and relevant contributor documentation.
No public API, protocol, dependency, timeout, capability or release change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
