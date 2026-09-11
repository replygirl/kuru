## Why

Windows CI at ef72080 failed its first fresh memory open with a generic SQLx pool timeout. The pool discards callback failures while retrying, so the diagnostic cannot identify whether transport, directory verification, or project identity verification failed, nor which startup phase owned the connection.

## What Changes

Preserve bounded, non-secret connection progress and callback failure context alongside the original SQLx error. Label staging, active, and post-ready startup phases and preserve observed cleanup errors. Keep all existing timeouts, authentication checks, ownership and tests intact.

## Capabilities

### Modified Capabilities

None: the existing memory and native-platform contracts remain correct.

## Impact

Only memory server/store diagnostics and their existing real database regression cases. No new dependency, retry, deployment, protocol or user setting.

## Surfaces

- [ ] interactive — no new CLI or UI flow
- [ ] deploy — existing native CI topology unchanged
- [ ] integration — existing SQL contract unchanged
- [ ] agent-behavior — no agent behavior change
