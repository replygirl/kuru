## Why

The interactive TUI blocks its Tokio worker in synchronous crossterm polling for
25 milliseconds while busy and 100 milliseconds while idle. A runtime activity
update or completed provider operation can therefore wait for that unrelated
polling interval, and sustained activity draining can delay the
typed result that authoritatively completes a turn.

The same loop has no explicit asynchronous terminal EOF path. Replacing its sole
reader with crossterm's pinned `EventStream` lets native input participate in the
same async selection as those other sources while
preserving the current view, completion, cancellation, animation and restoration
contracts.

## What Changes

- Enable crossterm 0.29.0's `event-stream` and Unix `use-dev-tty` features only
  in the TUI and consume the stream with the already pinned workspace futures
  package. The level-polled Unix source must preserve terminal input that becomes
  ready in the same native poll cycle as a resize.
- Replace synchronous terminal `poll`/`read` with one async terminal stream and
  select it alongside typed completion, bounded runtime activity and a persistent
  animation deadline.
- Route every queued or newly received completion through one ordered handler,
  bound its pre-completion activity drain, and schedule ready sources fairly.
- Treat terminal EOF and read errors as loop exits that abort and await the active
  dispatch, then use non-dream Harness shutdown to abort and await its nested
  actor/provider work before the existing terminal owner restores native state.
- Preserve the current generation fence, typed result authority, completion
  locking, failed-dispatch refresh, cancellation ordering, command behavior,
  motion cadence and reduced-motion behavior.

## Capabilities

### New Capabilities

### Modified Capabilities

- `chat-harness`: Specify asynchronous terminal wakeups, bounded fair scheduling,
  typed completion ordering, terminal-stream closure and cleanup.

## Impact

The implementation covers `apps/kuru-tui` dependency declarations, `Cargo.lock`,
the private interactive loop in `apps/kuru-tui/src/ui.rs`, the runtime's private
actor-task cleanup and `Harness::shutdown` implementation, and focused scheduler
and native terminal tests. Public `TurnOutput`, JSON, Harness signatures,
provider, memory, authentication, shutdown-dreaming and plain `View` interfaces
do not change. Dependency versions remain pinned; dependency edges add the
app-owned crossterm features, inherited futures, and the existing workspace
async-trait package as a dev dependency for controlled provider fixtures.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
