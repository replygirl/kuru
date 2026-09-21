## Why

Every turn on the `codex` (ChatGPT subscription) route fails. The CLI reports
`provider errors: completed response omits streamed output` and then
`all peers failed to produce a contribution`, for all seven peers, on real
multi-second round trips — the request, sign-in and transport are all healthy
and each peer really did produce its answer.

`packages/kuru-connectors/src/providers/sse.rs`'s `response.completed` handler
reconciled the items it accumulated from `response.output_item.done` against the
terminal `response.output` **by position**: `output.get(stream_output_index)`,
hard-failing when that slot is missing and requiring whole-item equality when it
is. `Decoder::reconcile_fragments` located every text, summary and tool fragment
the same positional way. That dense 1:1 correspondence was introduced by #36 and
no fixture ever exercised a terminal array that disagrees with it. A live
`--debug` capture shows what the backend actually sends: each peer streams a
`reasoning` item at index 0 and a `message` item at index 1 carrying the visible
text, and then closes with an envelope whose `output` is a present but **empty**
array. `output.get(0)` on `[]` is `None`, so the very first streamed item fails
the turn.

## What Changes

- An empty terminal `output` array alongside items already delivered as
  `response.output_item.done` is treated exactly as an absent one: those
  completed items are the authoritative output. An empty array with nothing
  streamed remains an empty completion, unchanged.
- A non-empty terminal `output` is authoritative for ordering and presence.
  Streamed items are matched to it **by item ID**, never by stream position.
  Reasoning items and tool-call scaffolding that the terminal array reorders,
  replaces or omits no longer fail the turn.
- The honesty guarantee is unchanged in substance and now stated precisely: a
  streamed item that carried user-visible text (`output_text` / `refusal`) must
  still be present in a non-empty terminal output under its own item ID, and its
  visible text must still agree, or the turn fails. Text, summary and tool
  fragments are still reconciled against the retained output, now located by
  item ID.
- A permanent `tracing::debug!` records the reconciliation shape into the
  bounded diagnostics ring under `--debug`: for every streamed and terminal item,
  its ID, type, position and text **length** only — never content. `--debug` now
  gates verbose (below-`INFO`) records, which ordinary runs omit.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-connectors/src/providers/sse.rs`: `Decoder::event`'s
  `response.completed` arm, new `reconcile_items` / `final_item` /
  `visible_text` / `trace_item` / `text_len` helpers, and `reconcile_fragments`
  now locating by item ID. Two new decoder tests
  (`accepts_subscription_terminal_envelope_with_empty_output`,
  `terminal_output_owns_order_and_presence_but_never_drops_visible_text`) and
  an extended assertion in an existing decoder test.
- `apps/kuru-tui/src/diagnostics.rs`: `SafeFields` admits `item_index`,
  `item_id`, `item_type`, `text_len`; `on_event` drops below-`INFO` records
  without `--debug`.
- `docs/protocols.md`, `docs/usage.md`, `apps/kuru-docs/reference/commands.md`.
- No configuration, API, dependency or schema change.

## Surfaces

- [ ] interactive
- [ ] deploy
- [x] integration
- [ ] agent-behavior
