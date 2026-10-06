# Proposal

## Why

Pending Unix installation recovery registers a Ctrl-C listener, but its non-Run listener was dropped before command dispatch. When the integrated explicit legacy import later created another listener, a signal arriving between recovery and import admission could be swallowed by the already-installed handler.

## What Changes

Retain the invocation listener through recovery and reuse it for an owned memory operation. Reject a queued signal before polling that operation; after admission, await the same operation and preserve its confirmed receipt or honest error. Update retains its existing separate listener and policy.

## Capabilities

### New Capabilities

### Modified Capabilities

The existing owned cleanup and import settlement contracts are correct; no capability delta is needed.

## Impact

Only the CLI invocation/recovery/memory-operation helper and focused helper tests change beyond the normal integration of archived `legacy-import-reconciliation` and `unix-update-recovery`. No schema, protocol, provider, dependency, exit policy or immutable archive changes.

## Surfaces

- [x] interactive — CLI cancellation ordering
- [ ] deploy
- [ ] integration
- [ ] agent-behavior

The source correction and focused test invocation were drafted/launched during integration before this separate fix gate. The required gate and returned contexts will be recorded accurately before further source changes or archive; no preimplementation gate is claimed.
