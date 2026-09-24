## Why

Kuru has no durable, typed record for provider-delivered reasoning summaries tied to the turn and final settled response that produced them. The runtime therefore cannot retain that bounded product signal without risking inference from raw private history, conflating it with a Harness lifetime operation ID, or leaking it through public transcript, session-export, or fork-presentation paths. The invoking owner's existing full-memory export remains a private archival surface and retains every stored row.

## What Changes

- Add producer-private `reasoning_summary.v1` persistence for provider-delivered summaries only.
- Bind every stored summary to the real `run_controlled_inner` turn ID, session, actor, invocation, final item, output, and summary coordinates.
- Project bounded display data separately while excluding private reasoning summaries from transcript, session export, fork presentation, and cross-session continuity; retain them in the invoking owner's full-memory export and backup/restore.

## Capabilities

### New Capabilities

- `private-reasoning-summaries`: Stores and projects typed, provider-delivered reasoning summaries with turn-accurate identity and strict privacy boundaries.

### Modified Capabilities

None.

## Impact

- Runtime actor and engine propagation, private memory records, and typed summary projection APIs.
- Runtime and memory fixtures covering idempotency, session isolation, display projection, exclusion from transcript/session-export/fork paths, and full-memory export/restore retention.
- User documentation describing the bounded provider-summary behavior and its privacy limits.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
