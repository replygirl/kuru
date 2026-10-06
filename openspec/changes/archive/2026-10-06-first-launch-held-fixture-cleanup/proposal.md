# Proposal

## Why

The cold first-launch PTY fixture can return an ordinary error before removing its deliberate CreatingDatabase hold. Its Sandbox then invokes ServiceCleanup from Drop, where retirement failure panics and replaces the original frame or assertion error; PR247's Ubuntu5 artifact contains only that cleanup panic.

## What Changes

Capture the named fixture's complete body as a Result, remove its deliberate hold on every ordinary outcome, explicitly close and reap its retained Terminal with the existing bound, and pass the combined outcome through the existing ServiceCleanup::release method. Preserve every original assertion and budget; a failing run must retain the original cause plus any cleanup failure instead of masking it.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds` in `apps/kuru-tui/tests/terminal.rs` and the minimal fix artifacts change. No production, helper, progress, readiness, worker, provider, protocol, deadline or configuration changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
