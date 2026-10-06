# Proposal

## Why

The synchronized `/config` PTY fixture leaves the transcript in Reading mode
after inspecting historical captured configuration. It then expects a later
configuration result at the tail without navigating there, contradicting the
stable reading-anchor contract and failing Ubuntu and macOS coverage.

## What Changes

- Explicitly navigate with completed PageDown frames back to follow-tail before
  issuing subsequent mode and configuration commands.
- Preserve every captured snapshot, privacy, live selection and deadline assertion.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None; the existing stable reading-anchor specification is correct.

## Impact

Only `apps/kuru-tui/tests/cli.rs` and this fix's verification artifacts change.
Production navigation, configuration, dependencies and CI policy are unchanged.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
