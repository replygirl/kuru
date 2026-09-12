## Why

The terminal dispatch path discards the runtime's completed `TurnOutput` and
reconstructs the answer from a bounded, lossy activity broadcast. A lagged receiver
can omit the answer or associate it with stale speaker state, and the terminal
never displays the returned token counts or generic limited-result state.

## What Changes

- Carry the completed turn through the dispatch completion channel and render its
  exact text once, with the returned speaker identity, token counts and generic
  limited-result state.
- Keep broadcast events as activity feedback; response events must not append
  answers or overwrite completed-turn facts.
- Preserve slash-command feedback, failures and cancellation, including rejection
  of stale completion generations.
- Add regression and terminal checks for result delivery independent of activity
  delivery, and document the visible completion information.

## Capabilities

### Modified Capabilities

None. The existing chat-harness requirements already require complete responses,
meaningful operation feedback and accurate participant presentation.

## Impact

The application-local dispatch return type, completion handling, view and renderer
in `apps/kuru-tui`, their tests, and terminal documentation. No provider contract,
runtime cognition, memory schema, authentication or retention change is needed.

## Surfaces

- [x] interactive — completed conversation and operation feedback in the TUI
- [ ] deploy — deploy/runtime/CI-execution topology
- [ ] integration — a third-party/external contract
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
