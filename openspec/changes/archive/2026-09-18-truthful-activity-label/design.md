## Context

The facing preview carries three regions: a visible-summary line
(`"thinking · …"`, driven by `FacingProgress.summary_tail`), a text tail, and
an activity line (`"activity · …"`, driven by `FacingProgress.activity`).
`ProgressDescriptor::round` publishes `"Responding"` once per request round and
no code path ever replaces it, while `ProgressObserver`'s `ProviderSink` impl
matches `ProviderEvent::ToolCallDelta` into an empty arm. The two facts are the
same bug seen from each end: nothing observes the tool call, so nothing can say
anything truer than the constant.

The identity of the tool is not available where the constant is published.
`ProviderEvent::ToolCallDelta { item_id, output_index, arguments_fragment }`
has no name field, and `providers/sse.rs` only learns the name at
`response.function_call_arguments.done`, which it stashes privately for the
final `Completion` and never emits as a `ProviderEvent`. The name *is* already
available one layer up: `kuru-runtime::engine` emits
`Event::ToolStarted { actor, name }` at dispatch with the raw catalog name, and
`apps/kuru-tui/src/ui.rs` already renders that same string verbatim into its
scrolling activity log.

## Goals / Non-Goals

**Goals:**
- The activity line reflects the real state during a facing stream: responding
  on text deltas, a tool call while one is in flight, the existing visible
  summary preserved.
- `ProviderEvent::ToolCallDelta` is observed rather than dropped.
- Prove it in a rendered frame from a real spawned `kuru` process.

**Non-Goals:**
- No new `ProviderEvent` variant and no SSE parsing change.
- No permission-flow change; the label never uses the permission module's
  redaction path.
- No provider work, no docs change (no user-facing command, configuration,
  protocol or workflow changes).

## Decisions

- **Two layers, each with the truest signal it actually has.** The runtime
  observer publishes the generic `"Calling tool"` the moment a `ToolCallDelta`
  arrives, because that is all the provider stream knows while arguments are
  still streaming. The TUI overrides it with `"Calling {name}"` once
  `Event::ToolStarted` delivers the resolved catalog name at dispatch. Rejected
  alternative: emit a new name-bearing `ProviderEvent` from
  `response.function_call_arguments.done`. That event fires only *after* all
  arguments have streamed — it would buy nothing during the window the label is
  wrong, at the cost of a new connector event type.
- **The raw catalog name, not the permission module's prettified
  `native_label`.** `Event::ToolStarted.name` is already displayed verbatim in
  this same TUI's activity log, so it is already established as safe to render;
  using it needs no new plumbing and keeps the two regions consistent. The
  prettified form would require exporting a helper from
  `kuru-connectors::permissions` or duplicating its match arms. This is a
  product-facing wording choice and is recorded here as such.
- **Facing-only scope.** `calling_tool` is set only when the event's actor is
  `view.speaker_id`. The preview region belongs to the one facing stream the
  user is watching; a peer consulted mid-loop must not rewrite it.
- **Display-only state in the view, not a new `FacingProgress` field.** The
  name never crosses the runtime boundary, so nothing new is published,
  persisted or fenced. `calling_tool` is cleared everywhere `preview` is
  already cleared (`begin_operation`, `settle`, `complete_turn`, `/cancel`,
  `/quit`), so no turn inherits a stale label.
- **Argument fragments are never read.** `ProgressObserver::tool_call` takes no
  arguments and the `ToolCallDelta` arm destructures with `{ .. }`; the fixture
  test asserts a sentinel embedded in the streamed arguments never reaches the
  screen or the terminal's byte output.

## Operational surface

No bind address, container topology or secret is introduced. The interactive
surface is the existing `kuru` binary spawned as a local child under a real PTY
by `apps/kuru-tui/tests/terminal.rs`, against a local mock `responses` server,
exactly as the neighbouring streaming fixtures already do. The fixture holds
its SSE body open on a `watch` channel so the in-flight window is deterministic
rather than timing-dependent.

## Risks / Trade-offs

- [Risk] A tool call that streams no `ToolCallDelta` at all (a provider that
  only sends the completed call) leaves the label on `"Responding"` until
  dispatch. → `Event::ToolStarted` still corrects it at dispatch, which is the
  same instant the call becomes real; the window is strictly smaller than
  today's.
- [Risk] `"Calling {name}"` shows an MCP tool id (`alias/tool`) unprettified.
  → Consistent with the existing activity log, which already shows exactly that
  string; a prettified form can be layered later without changing this seam.
- [Trade-off] Two labels for one conceptual state (`"Calling tool"` then
  `"Calling state_report"`). The alternative — suppressing the generic form
  until the name is known — would leave the known-wrong `"Responding"` on
  screen for the whole argument stream, which is the defect being fixed.
