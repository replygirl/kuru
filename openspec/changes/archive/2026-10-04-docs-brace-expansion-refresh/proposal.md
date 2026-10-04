# Proposal

## Why

The documentation dependency tree still resolves brace-expansion 5.0.9. Accept
Dependabot PR #153's patch update, which includes upstream parser stack and
rewrite bounds, and verify the existing documentation build against it.

## What Changes

- Update only the brace-expansion entry in `apps/kuru-docs/package-lock.json`
  to 5.0.12 with its registry URL and integrity hash.
- Record the existing dependency PR's review and acceptance evidence.

## Impact

The app-owned documentation installation and build consume the updated transitive
development dependency. No Rust dependency, runtime behavior, tool pin, or
publication workflow changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
