## Why

The facing preview's activity line (`draw_preview` in
`apps/kuru-tui/src/ui/render.rs`) renders `FacingProgress.activity`, which
`ProgressDescriptor::round` sets to the literal `"Responding"` once per request
round and which nothing ever mutates. At the same time
`ProgressObserver`'s `ProviderSink` impl matches `ProviderEvent::ToolCallDelta`
into an empty arm and drops it. So while a tool call's arguments are streaming
— the one moment the model is demonstrably *not* producing a facing answer —
the region still tells the user "Responding". The single truthful signal the
runtime already emits at dispatch time (`Event::ToolStarted { actor, name }`,
carrying the raw catalog name) reaches the TUI's scrolling activity log but
never the preview's activity line.

This is the second of the two residuals holding Phase 1 open: the activity
state is not truthful, and the dropped `ToolCallDelta` is the mechanical reason
the runtime has nothing truer to publish.

## What Changes

Two layers, each using a signal that already exists:

- `packages/kuru-runtime/src/progress.rs`: `ProgressObserver` observes
  `ProviderEvent::ToolCallDelta` instead of dropping it, moving the round's
  activity from `"Responding"` to the generic `"Calling tool"` and republishing.
  The argument fragment is never read, retained or published — the provider
  stream carries no tool identity until the call is complete, so the label
  stays deliberately generic at this layer. A new request round starts back at
  `"Responding"`.
- `apps/kuru-tui/src/ui.rs` / `ui/render.rs`: `View` gains a display-only
  `calling_tool: Option<String>`, set from `Event::ToolStarted`'s raw catalog
  name when the actor is the facing speaker, cleared on the matching
  `Event::ToolSettled` and everywhere the preview itself is cleared. When set,
  the activity line renders `"Calling {name}"` instead of the runtime's generic
  label; otherwise the runtime's label is rendered unchanged.

The existing reasoning/visible-summary state (`"thinking · …"` from
`summary_tail`) is untouched and remains the truthful third state.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-runtime/src/progress.rs`: `RESPONDING`/`CALLING_TOOL`
  constants, new `ProgressObserver::tool_call`, `ToolCallDelta` arm no longer
  empty. No type, event or public API change.
- `packages/kuru-runtime/src/progress_tests.rs`: the existing
  `tool_loop_replaces_preview_and_partial_call_has_no_authority` gains activity
  assertions (it already drives a `ToolCallDelta`).
- `apps/kuru-tui/src/ui.rs`: new `View::calling_tool` field, set/cleared in the
  `ToolStarted`/`ToolSettled` arms and alongside every existing
  `preview = None`; new unit test.
- `apps/kuru-tui/src/ui/render.rs`: `draw_preview` prefers `calling_tool`.
- `apps/kuru-tui/tests/terminal.rs`: new `ToolActivityState` /
  `tool_activity_complete` fixture and the real-PTY test
  `real_pty_activity_line_tracks_a_streaming_tool_call`.
- No changes to `packages/kuru-connectors`: no new `ProviderEvent` variant, no
  SSE parsing change, no permission-flow change.

## Surfaces

- [x] interactive — the rendered TUI preview activity line.
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
