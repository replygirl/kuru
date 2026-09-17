## Why

Runtime activity is currently represented as string triplets, which forces the
engine and TUI to parse presentation data before they can understand it. Tool
execution also lacks one redacted, measurable completion observation that can
be replayed safely and used by later Phase 1 work without retaining tool output.

## What Changes

- Add a runtime-owned semantic event enum for activity, selection, peer,
  relationship, state, budget, error, response, legacy and withheld events.
- Preserve the existing `{ kind, actor, detail }` public wire shape through a
  single projected adapter while using typed values inside the runtime and TUI.
- Add one settled tool observation per invocation with bounded projected
  arguments, outcome, receipt byte counts, optional digest, and full
  admission-to-settlement duration.
- Version completed turn journals at format 2 and replay v1 and v2 records
  through explicit version-aware normalization without re-dispatching completed
  work.

## Capabilities

### New Capabilities

- `typed-runtime-events`: Typed, redacted runtime events with compatible wire
  serialization, tool observations, and version-aware completed-turn replay.

### Modified Capabilities

<!-- None. -->

## Impact

- `packages/kuru-runtime`: event contract, engine emission and settlement,
  completed-turn journal adapters, replay, and tests.
- `apps/kuru-tui`: typed event consumption while preserving existing user-facing
  activity and JSON output contracts.
- `docs`: event and journal compatibility guidance, including projected receipt
  measurement semantics.
- No provider streaming, permission policy, accounting ledger, memory schema,
  or new public event JSON fields are introduced.

## Surfaces

- [x] interactive — TUI event rendering consumes the semantic contract.
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — CLI and A2A retain the stable three-key event wire contract.
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
