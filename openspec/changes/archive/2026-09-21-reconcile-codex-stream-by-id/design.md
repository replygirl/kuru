## Context

`Decoder` retains one item per `response.output_item.done`, keyed by that
event's `output_index`, and then validates them against the terminal
`response.completed` envelope. Since #36 that validation was positional:
`output.get(stream_index)` plus whole-item equality, with
`reconcile_fragments` locating each text, summary and tool fragment the same
way. Positions in a stream and positions in a terminal array are different
coordinate systems, and nothing in the Responses protocol promises they agree.

A live `--debug` capture on the ChatGPT subscription route (recorded per peer,
every peer of every turn) is the ground truth for this change:

| stage | index | id | type | text_len |
|---|---|---|---|---|
| streamed | 0 | `rs_…` | `reasoning` | 0 |
| streamed | 1 | `msg_…` | `message` | 5 |

and **zero** terminal items — the envelope carries the response id, status and
usage with `"output": []`. So the failure is not a subtle reordering: the
terminal array is empty, the first `output.get(0)` is `None`, and every turn
dies on a completion that actually succeeded.

## Goals / Non-Goals

**Goals:**
- Turns on the `codex` route succeed against the shape the backend really sends.
- Keep every honesty guarantee: streamed text is still reconciled against the
  retained output, and text that genuinely vanishes or changes is still an error.
- Leave a permanent, redaction-safe record of the reconciliation shape so the
  next disagreement is read off the ring instead of guessed at.

**Non-Goals:**
- Any change to the request builder, authentication, retry, usage accounting or
  tool handling.
- Accepting partial or streamed-only text as a substitute for a terminal event.
- Re-deriving content from deltas: deltas remain presentation fragments.

## Decisions

- **Match by item ID, never by stream position.** `final_item(output, id)` finds
  the terminal item; the stream's `output_index` is now only an ordering key for
  the fallback. Rejected: keeping positions and merely tolerating a short array —
  it still mis-pairs a reordered array, silently comparing two different items.
- **A non-empty terminal output is authoritative for ordering and presence.** A
  reasoning item the backend drops, replaces (a bare `summary: []` becoming a
  populated one) or reorders is a legitimate completion. Rejected: whole-item
  equality — it makes any terminal enrichment a client error.
- **User-visible text is the one hard requirement.** `visible_text` collects the
  typed `output_text` / `refusal` parts of an item; a streamed item with any is
  required to exist under its ID in a non-empty terminal output and to still say
  the same thing. This is the honesty guarantee, stated on the axis that matters
  rather than on positions. Rejected: dropping the check with the positions —
  that would let a terminal response silently contradict text already shown.
- **An empty terminal array with items streamed means "no restatement", not "no
  output".** It carries zero reconciliation information, so it is treated exactly
  as an absent `output` already was, and the streamed completed items become the
  response's output. An empty array with *nothing* streamed still means an empty
  completion — a distinction the fix must keep, because an empty completion after
  a tool call is a real and already-tested case.
- **Trace shape, never content.** `trace_item` emits `item_id`, `item_type`,
  `item_index` and `text_len` at `DEBUG` on `kuru.provider`, and
  `SafeFields` admits exactly those four names. `on_event` now drops below-`INFO`
  records unless `--debug`, so ordinary runs are unchanged.

## Risks / Trade-offs

- [A dropped tool call or reasoning summary is now silent] → It was always
  silent in effect: the terminal output is what becomes actor context, so an item
  missing there was never going to run. The ring records every streamed and
  terminal item, so the divergence is visible under `--debug`.
- [Whole-item equality is no longer enforced] → Visible text equality is, which
  is the part a user could be lied to about. Fragment reconciliation still checks
  every delta, summary and tool-argument against the retained item.
- [A backend that streams text and then returns `output: []` cannot be
  distinguished from one that meant to retract it] → Retraction has a
  representation in the protocol (`response.incomplete`, `response.failed`, an
  error envelope), all of which remain hard failures. An empty restatement is not
  one of them.
- [New admitted diagnostics field names] → All four are shape, and `item_id` is a
  provider-assigned opaque item identifier, not a credential; values are
  truncated at 256 characters like every other admitted string.

## Integration contract

- Route: OpenAI Responses SSE, both the `codex` subscription transport and the
  `responses` API-key transport, which share this decoder.
- Terminal event: `response.completed`. `response.output` may be absent, an
  empty array, or a non-empty array in any order; `id`, `status` and `error`
  validation is unchanged, and `usage` is still read from the terminal envelope.
- Identity: reconciliation is keyed on the item `id` shared by
  `response.output_item.done`, the `item_id` of every delta event, and the
  terminal array. Duplicate terminal item IDs remain an error. An item ID is
  still not a tool call ID.
- Fixture shape: `packages/kuru-connectors/src/providers/sse.rs`'s
  `accepts_subscription_terminal_envelope_with_empty_output` mirrors the captured
  live event sequence, ids, indexes and empty terminal array exactly.
