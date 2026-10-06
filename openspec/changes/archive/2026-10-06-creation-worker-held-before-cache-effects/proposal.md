# Proposal

## Why

The corrected PR249 cold first-launch PTY failed its original assertion that the template cache has no entries before creation is released. Its progress observer waits on the CreatingDatabase marker, but the independently owned creation worker can already run and create cache entries. The official failure preserves the original Result; the historical entry name is not recorded.

## What Changes

Reuse the existing test-support marker budget and removal wait at the creation worker before its first stage or template filesystem effect. Keep observer activity publication and its hold unchanged. Add an isolated task-local hook and causal worker regression that observes entry into that wait, proves the cache and stage remain absent, releases the marker, and awaits the same worker. Preserve worker startup-lock ownership, all original PTY assertions and cleanup, and existing cancellation/reap behavior. Release builds read no marker and acquire no new wait.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This repairs the existing fixture synchronization contract.

## Impact

Scoped to packages/kuru-memory/src/service/activity.rs, store/creation_worker.rs and the existing store/creation_template/hooks.rs test hooks. No public API, dependencies, production readiness behavior, budgets or tool configuration change. The exact existing cold PTY and affected marker, forwarding, cancellation/key-lock reap and fixture guard checks provide acceptance. Native macOS evidence does not establish Linux or Windows execution; fresh PR CI remains required.

## Surfaces

- [ ] interactive — no interactive product change
- [ ] deploy — no execution topology change
- [ ] integration — no external contract change
- [ ] agent-behavior — no agent behavior change
