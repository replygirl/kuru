## Why

The retained Unix shell worker can send its caller result before its confirmed
owner reservation is removed. A receiver can therefore resume while the
completed owner remains in the registry, even though cleanup succeeded. The
same ordering exists when a worker panics before spawn: it reports the error
before releasing the reservation.

## What Changes

Remove a confirmed or pre-spawn-terminated worker's own registry entry before
publishing its result. Keep a spawned worker registered when cleanup remains
unconfirmed. Add a deterministic wake-time regression that proves a completed
worker does not remove a concurrent owner's reservation.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `provider-tools`: Unix shell completion publishes only after its own confirmed
  owner reservation has been removed.

## Impact

- `packages/kuru-connectors/src/unix_shell.rs`: retained-worker completion and
  its Unix-only regression.
- `openspec/specs/provider-tools/spec.md`: completion ordering requirement.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
