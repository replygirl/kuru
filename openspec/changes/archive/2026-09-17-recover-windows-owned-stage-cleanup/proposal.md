## Why

Windows native CI observed OS error 32 after a verified Dolt candidate had already been published: the private `.install-*` stage could remain held while its owner attempted cleanup. A separate fixture also needs to invalidate the exact warmed cache binary under the same transient sharing condition without weakening the production cache checks.

## What Changes

Make Windows private-stage cleanup identity-checked and bounded only for native OS32: disarm the disposable owner before cleanup, remove the exact private child, reconcile absence or the same identity before retrying, and remove only the verified empty outer container. Keep the cache lease through synchronous cleanup and report post-publication cleanup exhaustion without deleting a replacement. Add fixture-only same-identity OS32 recovery for cache corruption and controlled held-handle recovery, exhaustion, and replacement cases.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`kuru-memory` private temporary-stage cleanup and focused native provisioning fixtures. No public API, schema, general deletion helper, retry policy outside native OS32 cleanup, or publication retry changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
