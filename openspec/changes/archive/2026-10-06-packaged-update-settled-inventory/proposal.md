# Proposal

## Why

PR247's Linux ARM packaged acceptance completes its direct and updated cold offline conversations, then rejects the retained Unix updater coordination directory as an unexpected companion. The fixture's top-level entry count predates the intentional `.kuru-update/install.lock` settled-state contract and therefore reports a successful update as failure.

The separate Linux running-image fixture registers cleanup at `XDG_DATA_HOME`, although CLI path resolution appends `kuru` to that directory. Its mapped-image assertions pass, but the existing guarded root correctly rejects release without a quiescence record for the actual `data/kuru/memory` store.

## What Changes

Replace the final count with exact platform-specific installation names. On Unix, inspect the existing owner-only updater directory, require its sole checked zero-byte `install.lock`, and verify recovery returns no pending outcome; continue rejecting receipts, draft files and candidate or backup images.

Register the Linux fixture's existing ServiceCleanup at the actual Kuru data directory, retaining its launch environment, mapped-image assertions and unchanged retirement/root guard.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Only `apps/kuru-tui/tests/embedded_runtime.rs`, one cleanup registration line in `apps/kuru-tui/tests/update.rs`, and this fix's artifacts change. Production updater policy, native runtime, Windows installation inventory, offline conversations, authority and cleanup assertions remain unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
