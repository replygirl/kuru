## Context

`Decoder` builds a function call out of three kinds of stream event: the item
announcement, the argument deltas, and the settled arguments. Since the tool
path was written it read the call's `name` off
`response.function_call_arguments.done`, which the OpenAI Responses reference
documents as carrying one. The ChatGPT subscription backend does not.

A live `--debug` capture on that route is the ground truth for this change. Per
function call, keyed throughout by `item_id`:

| order | event | carries |
|---|---|---|
| 1 | `response.output_item.added` | `item.id`, `item.type:"function_call"`, **`item.name`**, **`item.call_id`**, `item.arguments:""` |
| 2..n | `response.function_call_arguments.delta` | `item_id`, `output_index`, `delta` |
| n+1 | `response.function_call_arguments.done` | `item_id`, `output_index`, `arguments` |
| n+2 | `response.output_item.done` | the completed `function_call` item |
| last | `response.completed` | envelope with `"output": []` |

Across one turn's ring: 381 `function_call_arguments.delta` and 7
`function_call_arguments.done` records, every one with `has_name:false` and
`has_call_id:false`; 4 `output_item.added` records for `function_call` items,
every one with `has_name:true`, `has_call_id:true` and `arguments_len:0`; and
**zero** `output_item.done` records, because the turn already died at step n+1.

So the name and call ID are established exactly once, at the announcement, and
are never restated.

## Goals / Non-Goals

**Goals:**
- Tool-calling turns on the `codex` route succeed against the shape the backend
  really sends.
- Keep the honesty guarantee exactly as strong: a call still has a name,
  a call ID and complete arguments before anything dispatches it.
- Keep #49's by-item-ID terminal reconciliation and transient reasoning
  handling untouched.

**Non-Goals:**
- Any change to the request builder, tool declaration, authentication, retry,
  usage accounting or the tool host.
- Inferring a call's name from its arguments, its position, or the declared
  tool set.
- Re-deriving arguments from deltas: deltas remain presentation fragments.

## Decisions

- **Identity is announced, not repeated.** `announce_function_call` retains
  `(name, call_id)` keyed by the item's own `id`, from both
  `response.output_item.added` and `response.output_item.done`. Either may
  establish it; a second announcement may repeat the values but never change
  them, which is an error. Rejected: reading identity only from
  `output_item.done` — it arrives after `function_call_arguments.done`, so a
  decoder that needs the name at the `.done` event still has none, and #49's
  live shape shows `output_item.done` is not always reached.
- **`function_call_arguments.done` carries arguments only.** Its `name` becomes
  optional and, when present, is still checked against the terminal item in
  `reconcile_fragments`. Rejected: keeping the field required and relaxing only
  for the `codex` label — the two routes share this decoder and the shape is a
  property of the backend's event stream, not of the credential used.
- **The hard error moves to dispatch time, on the authoritative output.**
  `settle_function_calls` runs on `response["output"]` after it is settled —
  whether the terminal array supplied it or it was synthesized from streamed
  items — fills a missing `name` or `call_id` from that exact item's own
  announcement, and then refuses anything not dispatchable: no name ever
  announced, no call ID ever announced, an announcement the terminal output
  contradicts, or arguments that are absent or not complete JSON. This is where
  the guarantee belongs: the output array is what becomes actor context and what
  the tool host runs, so checking it covers every provenance at once. Rejected:
  checking only `tool_done` — a terminal item the backend supplies without a
  matching argument event would slip past.
- **Filling identity in is reconciliation, not invention.** The name written
  into an item comes from the provider's own announcement of that same item ID;
  it is never guessed, defaulted, or taken from another item. A conflict between
  the announcement and the terminal output fails the turn rather than picking a
  winner.
- **Trace shape, never content.** `trace_function_event` emits `event_type`,
  `item_id`, `has_name`, `has_call_id` and `arguments_len` at `DEBUG` on
  `kuru.provider` under the existing `responses-stream-reconcile` operation with
  `stage="function-call"`, and `SafeFields` admits exactly those four new names.
  Arguments can carry actor context and file paths, so their content is never
  recorded — only how many bytes arrived.

## Risks / Trade-offs

- [An item's `name` can now be written by the decoder rather than read off the
  terminal output] → Only from the same item ID's own announcement, only when
  the terminal output left it empty, and never over a value the terminal output
  states. A disagreement is an error, not a merge.
- [`response.output_item.added` is now decoded, where it was previously ignored]
  → It grants no authority: it contributes identity for an item the stream must
  still complete, is never retained as output, and never spends the retained
  output budget. Its arguments field is ignored entirely.
- [A call the backend announces and then abandons] → It never appears in the
  authoritative output, so nothing dispatches; the announcement is dropped with
  the decoder.
- [New admitted diagnostics field names] → All four are shape. `has_name` and
  `has_call_id` are booleans and `arguments_len` is a byte count; no argument
  content, tool name or call ID value is admitted by them.

## Integration contract

- Route: OpenAI Responses SSE, both the `codex` subscription transport and the
  `responses` API-key transport, which share this decoder.
- Identity: a function call is keyed on the item `id` shared by
  `response.output_item.added`, `response.output_item.done`, the `item_id` of
  every argument event, and the terminal array. An item ID is still not a call
  ID; `call_id` is a separate announced field and is required for dispatch.
- Event contract: `response.function_call_arguments.done` requires `item_id`,
  `output_index` and `arguments`; `name` and `call_id` are optional there.
  `response.output_item.added` requires nothing — an item it omits is simply
  not announced.
- Dispatch contract: every `function_call` in the returned `response.output`
  has a non-empty `name`, a non-empty `call_id`, and an `arguments` string that
  parses as JSON. Anything else fails the turn.
- Fixture shape:
  `accepts_subscription_function_call_announced_only_on_output_item_added` in
  `packages/kuru-connectors/src/providers/sse.rs` mirrors the captured live
  event sequence, ids, field presence and empty terminal array exactly.
