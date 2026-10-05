# Proposal

## Why

Windows instrumented main CI `37244540171` failed the deliberately silent-owner CLI fixture after its unchanged 30-second readiness window while also performing unrelated cold engine and first template preparation. The trace does not establish the cause of the historical database interval or subsequent cleanup delay.

## What Changes

- In `apps/kuru-tui/tests/cli.rs`, use the existing prepared engine and store-template cache for `cli_open_is_unchanged_when_the_owner_cannot_publish_its_activity`, keeping its fresh project, real Dolt owner, activity-write failure injection and every assertion.
- Keep the independent cross-platform cold-start CLI fixture unchanged, including its cold engine/template cache, JSON/progress assertions, owner exit and reopen checks.

## Impact

This is a focused CI-fixture correction authorized on exact main `1ac1a1751b28636771ade55893024850cc2f312a`. Only the single test fixture choice and this typed CI record change. Production source, startup deadlines, activity semantics, cleanup classifiers, pools, dependencies, workflows, secrets and required checks remain unchanged. Local focused behavioral/static checks and fresh native instrumented CI are required before archive; no deterministic negative reproduction of the historical runner delay is claimed.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
