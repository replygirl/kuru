## Why

The managed memory owner retains a candidate branch pool while a successful promotion or abandonment cleans up that candidate's refs. Dolt can therefore reject the cleanup because the branch remains in use by the owner's own session; hosted Windows runtime coverage observed this during ordinary dream promotion.

The candidate result is already durable at that boundary, so keeping its resolved view open provides no valid authority and prevents the required cleanup. Rejected or conflicting transitions must continue to retain the usable candidate view.

## What Changes

Close the candidate branch pool only after a transition has a committed result and before its ref cleanup. Bound the close, keep the main pool live, propagate close failures without forcing ref deletion, and leave candidate views open after a rejected or conflicting transition.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/store.rs` candidate transition ordering and focused real-Dolt regression coverage. No public API, schema, dependency, migration, timeout, or force-deletion change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
