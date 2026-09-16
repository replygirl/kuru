## Context

The existing subscription SSE decoder collects a terminal response; API-key
requests currently read a JSON body. Both map terminal native output to P1 typed
content and retain actor-private pending native tool continuation. Actors receive
owned work over mpsc, and the TUI already separates authoritative turn completion
from decorative semantic events and fences cancelled operations.

## Goals / Non-Goals

Implement the proposal's live preview without a durable delta log, another actor
worker, an unbounded event queue or a second canonical answer. No persisted
reasoning feature, permission change, accounting ledger or mode behavior change.

## Decisions

1. `Provider::stream(request, &mut dyn ProviderSink)` is required; `complete`
   defaults to a shared collector. `ProviderSink::emit` is Send, object-safe,
   async and fallible. A borrowed observer can await future usage persistence;
   synchronous callbacks or a separate worker would obstruct that boundary.
2. Normalized events retain native item/output/content/summary indexes for text,
   refusal, visible summary and tool fragments, plus Usage, Completed and Failed.
   The generic collector validates terminal cardinality and merges usage. The
   native parser reconciles fragments against raw final items before flattening:
   opaque reasoning means raw output indexes are not content-block positions.
   Terminal-only replies remain valid; already-buffered duplicate terminals fail
   without waiting for future bytes or EOF. REST tool fragments carry item ID,
   not an invented call ID. Existing 2 MiB retained-response and wire bounds stay.
   The optional observer receives original terminal usage before the collector
   fills missing result fields from earlier observations. This preserves the
   distinction between terminal-reported usage and partial evidence for P7.
3. Actor Work carries an owned optional progress descriptor/sender. An observer
   is borrowed locally while collecting a selected speaking request. Internal
   requests use silent completion. Native Pending and encrypted reasoning stay
   inside the provider's per-actor mutex, including immediate tool continuations.
4. Runtime publishes latest-value snapshots with turn ID, request round,
   increasing sequence, bounded UTF-8 text/summary tails and activity. Prefer
   watch/coalescing transport over deltas that become corrupt when updates drop.
   Suggested bounds are 8 KiB text, 2 KiB summary and 256 bytes activity. Tool
   fragments have no preview mapping. UI generation adds the operation fence.
5. A small provisional region sits above status/composer and outside cached
   transcript lines. It renders unfinished text and visible summary/activity,
   truncation and provisional labels using the existing palette. A progress wake
   coalesces work at the existing busy paint cadence; final completion clears the
   preview before appending its authoritative answer. Plain terminal-safe text
   avoids speculative Markdown interpretation.

Runtime/UI agreement: `Harness::subscribe_progress(&self)` returns
`watch::Receiver<Option<FacingProgress>>`. The snapshot has `turn_id: String`,
`request_round: u32`, `seq: u64`, `text_tail: String`, `text_truncated: bool`,
`summary_tail: String`, `summary_truncated: bool`, `activity: String` and
`activity_truncated: bool`. Round starts at 1; sequence increases within the turn.
Every speaking round begins with an empty reset snapshot. A transport `None`
has no turn identity and is not completion authority. The UI mints the ordinary
turn ID at submission and clears previews using its own operation lifecycle;
slash commands and exact retries do not replay a preview.

## Risks / Trade-offs

- Partial native streams can disagree with final output → reconcile observed
  raw fragments/items before accepting the terminal; retain final-only tool use.
- Fast output can starve input or lose deltas → bounded latest snapshots, existing
  scheduler cadence and real burst/cancellation PTY tests.
- Internal text can leak through a broad observer → attach only at selected
  speak-and-act; prove part and relationship speakers versus silent consultation.
- Cancellation can leave late work → turn/round/sequence plus UI generation fence,
  while retaining existing durable-checkpoint precedence.
- Temporary summaries can accidentally become history → keep progress separate
  from semantic events/journal and retain actor durable-block filtering.

## Operational surface

The existing local terminal process owns rendering and runtime progress. There
is no new listener, port, container, credential, binary, target or release
topology. Native authentication and configured endpoints remain unchanged.
Preview state is bounded in process and never part of project storage.

## Integration contract

Connectors own native routes, SSE framing and raw response reconciliation;
runtime owns selected-speaker admission; the TUI owns its operation generation.
HTTP fixtures use native REST item IDs/indexes separately from completed tool
call IDs and retain actor-private encrypted continuation. Both existing routes
must pass actual delayed/chunked fixture requests, with only the subscription
route retaining its existing missing-media-type compatibility exception.

The official Responses streaming reference defines indexed text/refusal,
visible-summary and function-argument deltas, usage fields and terminal outcomes:
https://platform.openai.com/docs/api-reference/responses-streaming/response/refusal?lang=python
Protocol research was verified on 2026-09-16. Public API SSE header validation
does not expand the existing subscription missing-header compatibility exception
to arbitrary API-key endpoints.
