# Proposal

## Why

A raw profile can predate the only recorded spawn of its reused PID. The existing diagnostic incorrectly states a positive writer identity from that PID or a sibling signature match.

## What Changes

- `packages/kuru-delivery/src/coverage/spawns.rs`: describe recorded PID and sibling-signature matches as candidates, including missing-row and PID-reuse limits; update existing diagnostic assertions.
- `docs/development.md`: describe the same informational attribution limits.

## Impact

Coverage automation wording only. No profile parsing, timestamp correlation, export filtering, threshold, process lifetime, workflow, secret, tool or pin changes. Previous archives remain immutable.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
