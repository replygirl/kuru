## Why

An owned Dolt supervisor can report Ready within the configured memory startup budget, yet the required client-side authenticated connection has an independent two-second SQLx acquisition deadline. On Windows native CI, that probe timed out before its identity callback began, so Kuru rejected an otherwise ready memory owner before any conversation or undo work started.

## What Changes

Use one startup deadline across owner readiness, the initial authenticated connection, and identity verification. Keep the existing transport margin and bounded failure cleanup; retain the ordinary branch-pool acquisition policy and all identity checks. A past-deadline probe must fail without returning an unverified store or leaving its owned supervisor and lease active.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The correction is confined to `kuru-memory`'s owned-supervisor open and initial connection code, its native startup fixtures, and the fix verification record. It changes no public RPC, database schema, provider route, or ordinary branch-pool acquisition policy.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
