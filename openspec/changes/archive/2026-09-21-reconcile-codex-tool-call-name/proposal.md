## Why

Every turn on the `codex` (ChatGPT subscription) route in which the model calls
a tool fails. The CLI reports `provider errors: completed function arguments
lack name` and then `all peers failed to produce a contribution`, for all seven
peers, while a turn that needs no tool succeeds — so the request, sign-in and
transport are all healthy and each peer really did decide on a call.

`packages/kuru-connectors/src/providers/sse.rs`'s
`response.function_call_arguments.done` handler required a `name` field on that
event (`.context("completed function arguments lack name")`). A live `--debug`
capture shows the backend never puts one there. It announces the call's `name`
and `call_id` exactly once, on `response.output_item.added`, with empty
arguments; every `function_call_arguments.delta` and the
`function_call_arguments.done` that follows carry `item_id` and arguments
alone. The decoder failed at the `.done` event, before `response.output_item.done`
could supply the name and before any of #49's terminal reconciliation ran.

## What Changes

- `response.function_call_arguments.done` is treated as carrying arguments
  only. Its `name` is optional; when absent the call's identity is resolved by
  `item_id` from the item's own announcement.
- `response.output_item.added` is decoded for `function_call` items and retains
  the announced `name` and `call_id` keyed by item ID. `response.output_item.done`
  announces the same way, so either event can establish the identity and they may
  arrive in either order relative to the argument events.
- The hard error moves to dispatch time, on the authoritative output. A
  `function_call` that reaches the runtime must have a non-empty name, a
  non-empty call ID and complete, valid-JSON arguments, or the turn fails. A
  call whose name was never announced, whose announcements contradict each
  other or the terminal output, or whose arguments never became complete JSON is
  still refused — it never reaches the tool host unnamed or truncated.
- The `responses-stream-reconcile` diagnostics trace added in #49 gains a
  `stage="function-call"` record at every function-call stream event: event
  kind, item ID, whether that event carried a name and a call ID, and the
  arguments **length** only — never argument content.
- The by-item-ID terminal reconciliation and transient reasoning handling from
  #49 are unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-connectors/src/providers/sse.rs`: the
  `response.function_call_arguments.done` arm, a new
  `response.output_item.added` arm, new `Decoder::announce_function_call` and
  `Decoder::settle_function_calls`, a new `tool_calls` field, `tool_done`'s name
  becoming optional, the tool-name check in `reconcile_fragments` becoming
  conditional, and a new `trace_function_event` helper. Two new decoder tests
  (`accepts_subscription_function_call_announced_only_on_output_item_added`,
  `resolves_function_call_identity_by_item_id_or_fails_the_turn`).
- `apps/kuru-tui/src/diagnostics.rs`: `SafeFields` admits `event_type`,
  `has_call_id`, `has_name`, `arguments_len`.
- `docs/protocols.md`.
- No configuration, API, dependency or schema change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration
- [ ] agent-behavior
