## Why

`View::new` and `View::refresh` currently read `Harness` directly, so otherwise
deterministic rendering, editor, and scene tests start full Dolt memory merely
to construct presentation state. Extracting one owned projection boundary makes
the view synchronous and plain while keeping the runtime, storage, completion,
and terminal behavior unchanged.

## What Changes

- Add owned `InitialViewData` and `RuntimeSnapshot` presentation inputs in the
  TUI module, and construct or refresh `View` synchronously from those values.
- Move public-history, session, project, reduced-motion environment, topology,
  relationship, focus, and configuration reads into the TUI runtime adapter.
- Preserve the existing part-label tuples, relationship values, transcript,
  completion metadata, activity decoration, command feedback, status text, and
  render output.
- Preserve completion-before-refresh, failed-dispatch refresh and settle,
  cancellation ordering, and generation rejection before any completion or
  snapshot application.
- Convert renderer, editor, and scene cases to plain fixtures without Dolt, and
  retain a separate real-Harness integration target that proves runtime
  relationships, routes, completion facts, and adapter projection reach the
  view correctly.
- Add no streaming, event-stream abstraction, cancellation-policy change,
  public `TurnOutput` change, JSON change, schema change, or runtime ownership
  change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing TUI and chat-harness requirements remain unchanged.

## Impact

The refactor affects `apps/kuru-tui/src/ui.rs`, its scene tests,
`apps/kuru-tui/tests/visual.rs`, and a focused TUI runtime-projection integration
target such as `apps/kuru-tui/tests/ui_runtime.rs`. It changes only
application-local construction and refresh APIs. The real private-dispatch
fixture moves to a separate `src/ui/runtime_tests.rs` module;
`kuru-runtime`, memory,
providers, persisted formats, public command output, dependencies, and bundled
engine compilation remain unchanged. Existing PTY, preferences, trust,
persistence, and native terminal tests stay real and remain in their current
targets.

## Surfaces

- [x] interactive — internal TUI state construction changes while observable terminal behavior remains invariant
- [ ] deploy — deploy/runtime/CI-execution topology
- [ ] integration — a third-party/external contract
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
