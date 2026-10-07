# Proposal

## Why

The checked-recovery fixture returned a Dolt early-exit error on Linux ARM in CI run 37665200229, job 112942954144, at head d57f21ea. Its raw initial and successor owner opens omit the existing bounded fixture log capture, so the temporary server log disappears without identifying the failed iteration or startup phase.

## What Changes

Route those two fixture-only opens through the existing startup error capture and label the initial and successor phases. Add the reopen iteration to the checked-recovery helper-call context. Preserve every operation, ownership gate, close/reap ordering, assertion and deadline; this is diagnostic evidence, not a claimed startup cure.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only the relevant fixture in packages/kuru-memory/src/facade.rs and this fix record change. No production, schema, protocol, logging infrastructure, dependency or configuration change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
