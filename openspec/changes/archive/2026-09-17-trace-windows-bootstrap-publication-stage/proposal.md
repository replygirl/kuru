## Why

A native Windows packaged-runtime acceptance timed out at its existing 100-second command budget after reporting `release archive validated`. Its captured stdout and stderr remained open, so the bootstrap cannot yet distinguish stage construction from durable publication; an earlier passing native invocation does not establish the intermittent timeout's cause.

The bootstrap already has fixed path-free verbose markers through archive validation. Failure-only observations at the next two existing boundaries, plus a bounded fixture-owned inventory after the command wrapper has cleaned up, will make a recurrence actionable without changing installer behavior.

## What Changes

- Add fixed path-free verbose markers after the private installation stage opens and immediately before publication begins.
- On the existing packaged-runtime fixture's failed Windows bootstrap path, append a bounded direct-entry metadata observation for the install directory, at most one matching private stage, its candidate executable, and the installed target after command cleanup.
- Preserve the 100-second deadline, process wrapper, native bridge, publication implementation, retry behavior, success output, and ordinary installation semantics.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Changes are limited to `packages/kuru-delivery/support/install.ps1`, the failure-only Windows path in `apps/kuru-tui/tests/embedded_runtime.rs`, and this test correction's Cospec record. There are no public API, storage, protocol, dependency, or migration changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
