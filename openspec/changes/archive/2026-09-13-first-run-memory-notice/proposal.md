## Why

A newly initialized project has durable memory controls, but it does not tell a
person where that memory lives or how to inspect, export, selectively forget,
or explicitly purge it. The first successful interactive or headless runtime
command is the narrow point where Kuru can state those controls without making
the notice part of a peer conversation or provider prompt.

## What Changes

- Add one project-scoped, versioned informational notice using existing memory
  state after actual writable memory is available.
- Deliver the notice through stderr before headless work or a completed first
  TUI frame before input, then record the shown version in one ordinary Dolt
  revision.
- Keep inspection, help, authentication, configuration, and missing-store paths
  storage-free; retain unchanged machine stdout and public UI entrypoints.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `versioned-memory`: persist the displayed first-run notice version only after
  its app presentation succeeds.
- `chat-harness`: present a non-model first-run memory notice before interactive
  input or headless runtime work.
- `public-documentation`: explain first-run notice content and the distinct
  selective-forget versus confirmed-project-purge boundaries.

## Impact

Adds a private app memory-notice adapter and narrow CLI/UI wiring, with focused
TUI, CLI, and persistence fixtures. It uses existing `MemoryStore` state APIs,
adds no schema, runtime API, provider call, dependency, migration, or breaking
public interface.

## Surfaces

- [x] interactive — the first completed TUI frame and headless stderr gain an
  informational notice while stdout remains unchanged.
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
