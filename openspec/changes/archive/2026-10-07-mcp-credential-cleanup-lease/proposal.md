# Proposal

## Why

Windows coverage job 112865645143 failed the existing shared-issuer MCP fixture
while deleting an isolated fake credential, reporting only that the native
credential changed. Its cleanup releases the read lease before acquiring a delete
lease and can mask the preceding functional result; the native failure stage and
underlying Windows cause remain unproved.

## What Changes

Retain the exact alias lease from reading through generation-checked deletion.
Finish cleanup while preserving the original functional error, and qualify native
vault failures with fixed operation labels without credential contents. Preserve
strict generation guards, native routes, deadlines and all resource assertions;
do not retry or suppress a failed native operation.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `packages/kuru-connectors/src/mcp.rs` fixture cleanup and
`packages/kuru-connectors/src/mcp_credentials.rs` operation diagnostics change.
No API, dependencies, provider authentication, model requests or platform policy
change. Existing fixture execution and scoped static checks establish local
correctness; fresh native Windows CI determines whether the failure recurs.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology
- [x] integration — native MCP credential operation diagnostics
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
