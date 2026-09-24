## Why

The producer-private reasoning change suppressed the existing selected-speaker
thinking preview along with settled private records. That removes a bounded,
transient interactive signal that the earlier runtime and TUI intentionally
rendered, even though it neither persisted nor entered a completed transcript.

The durable record boundary remains necessary: settled summaries and provider
coordinates must not flow into public event detail, peer context, or replay.
The regression is treating the already-approved streaming preview as if it
were a durable settled record.

## What Changes

- Restore the existing bounded text-only reasoning preview for the selected
  speaker while a provider stream is active.
- Keep settled reasoning records and their provider coordinates out of progress
  events, transcripts, peer context, and durable public output.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `private-reasoning-summaries`: distinguish the existing transient preview
  from settled producer-private records.

## Impact

- `packages/kuru-runtime/src/progress.rs` and its focused progress tests.
- The private-reasoning-summary capability specification and verification
  ledger.

## Surfaces

- [x] interactive — the selected speaker's existing TUI preview remains
  available during a live stream.
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
