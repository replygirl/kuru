# Proposal

## Why

Native reasoning requests currently forward effort and request encrypted continuation without opting into provider-authored reasoning summaries. The isolated native evaluation completed four low-effort requests with positive reasoning usage but no summaries; official Responses documentation requires an explicit summary opt-in, so the existing private-summary and transient-preview paths cannot rely on the present request.

## What Changes

Responses request preparation will request `reasoning.summary = "auto"` when an explicit effort other than `none` is supplied, preserving the exact effort string. Absent effort preserves the provider default without a new reasoning object; explicit `none` preserves its current effort-only request. Both direct subscription and API-key Responses routes use the same request preparation and context measurement. Existing parsing, continuation, private persistence and transient presentation remain unchanged; requesting a summary does not guarantee the provider emits one.

## Capabilities

### New Capabilities

### Modified Capabilities

The existing private-summary contract is correct; only request preparation is incomplete.

## Impact

Connector request preparation and focused captured-HTTP regressions in `packages/kuru-connectors/src/providers.rs`, plus the owning configuration documentation. No authentication, runtime, schema, configuration surface, model registry, dependency or pin changes. The completed evaluation branch and archived evidence remain intact.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
