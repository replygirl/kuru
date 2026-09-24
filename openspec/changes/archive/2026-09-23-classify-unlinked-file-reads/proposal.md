## Why

A checked reader can retain a regular file handle immediately before another
authorized owner unlinks its name. On Unix that valid race changes the retained
handle's link count to zero, but the platform currently reports the same privacy
failure used for files with additional hardlinks, preventing memory service
endpoint retirement from being observed as ordinary absence.

## What Changes

- Classify an opened regular file with zero remaining links as a vanished named
  object using `io::ErrorKind::NotFound`.
- Continue rejecting files with more than one hardlink as a privacy failure.
- Prove the distinction with deterministic retained-handle and real managed
  memory endpoint retirement regressions.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The change affects the checked regular-file validation boundary in
`packages/kuru-platform` and its native filesystem tests. Existing memory
endpoint reads consume the corrected `NotFound` classification without an API,
protocol, timeout, or retry change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
