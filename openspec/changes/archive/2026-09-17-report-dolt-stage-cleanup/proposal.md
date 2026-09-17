## Why

Windows native CI observed a successful cold Dolt publication with a leftover `.install-*` stage. The disposable stage is currently removed by a destructor that cannot report deletion failure, so activation can return success while the cache inventory remains dirty.

## What Changes

After a verified publication, close candidate and probe handles, explicitly close the private stage, and return a contextual cleanup error if that close fails. Retain the cache lease until cleanup has completed. Publication-failure and cancellation paths keep their existing preservation behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`kuru-memory` private temporary-stage and Dolt activation code and focused native fixtures. No public API, storage schema, dependency, or timeout change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
