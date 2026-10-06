# Proposal

## Why

The production lost-commit-reply migration fixture returns early while its
opening task, proxy or memory store may still own a live engine. Its temporary
root then panics during Drop, masking the original Result error as observed in
macOS coverage job 112225031478; the historical underlying error is unknown.

## What Changes

- Keep fixture task and store ownership through every error and assertion path,
  await their cleanup and exact store quiescence before releasing the root.
- Use the existing root release accessor to retain the original Result error,
  and preserve original panic payloads after cleanup.
- Exercise a deliberate early error plus the original successful real-memory
  fixture with unchanged assertions and observation budgets.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None; migration and ownership specifications remain correct.

## Impact

Only the one recovery fixture, a deliberate-error acceptance case and reuse of
existing test-support directory enumeration change. No production migration,
runtime, auth, dependencies, pins or coverage policy change; no claim that the
missing historical migration error is diagnosed or corrected.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
