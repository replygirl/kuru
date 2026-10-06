# Proposal

## Why

Two Windows coverage exports rejected a corrupt raw profile after every assigned
test passed. Existing failure artifacts omit sibling profile names and actual
test-run process IDs, so they cannot identify the recurring module's writer.

## What Changes

- Repair `packages/kuru-delivery/src/coverage.rs` to record actual Windows test
  selection process IDs in the existing runner spawn ledger.
- Repair `packages/kuru-delivery/src/coverage/orchestrate.rs` to retain bounded
  profile names, sizes, modification times, header bytes and existing writer
  attribution in `failure.txt` when an instrumented export fails.
- Cover the failure path and update the owning development documentation.

## Impact

Only coverage automation and its existing failed-run artifact change. Strict
profile merging, the 90% gate, tool pins, jobs, secrets and application process
lifetimes remain unchanged. This repairs missing diagnostic evidence; it does
not establish or fix the underlying profile corruption.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
