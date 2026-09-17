## ADDED Requirements

### Requirement: Canonical bounded provider streaming

Providers SHALL implement a normalized stream with text/refusal deltas, explicitly
visible reasoning-summary deltas, tool-argument fragments, usage observations and
terminal completion or failure. `complete` SHALL collect that same stream through
an async fallible sink, without a separate inference implementation or recursively
defined defaults. Only a validated successful terminal completion SHALL authorize
tool calls or a durable assistant message. A missing success terminal, terminal
failure, multiple delivered terminals, or events delivered after a terminal MUST
fail collection. Transport and sink errors MUST propagate without accepting a
partial completion. Usage MUST preserve missing versus explicit zero values;
terminal reported components take precedence, with earlier observed components
retained when the terminal omits them.

#### Scenario: Completion and streaming agree

- **WHEN** a deterministic provider emits deltas followed by a typed completion
- **THEN** streaming and completion collection produce the same ordered blocks,
  call IDs, usage and stop reason through the same inference path.

#### Scenario: Incomplete stream

- **WHEN** a request ends before a successful terminal, reports terminal failure,
  or its sink fails
- **THEN** no partial tool call or assistant completion is authorized and any usage
  reported before failure has been offered to the awaited observer.

#### Scenario: Partial usage

- **WHEN** cached-input or reasoning-output counts are absent or explicitly zero
- **THEN** collection preserves that distinction without adding those subsets to
  input/output totals or inventing terminal observations.

### Requirement: Native SSE reconciliation and continuation privacy

Both native OpenAI routes SHALL stream bounded SSE while retaining current wire,
event, line, retained-response and time limits. The native parser SHALL reconcile
observed text, refusal, visible-summary and function-argument fragments using raw
item identities and indexes before flattening terminal output into typed blocks.
Raw item IDs MUST NOT be confused with tool call IDs, and raw output indexes MUST
NOT be interpreted as flattened block positions. Terminal-only success SHALL be
valid. Conflicting identities, invalid completed argument JSON and unmatched
observed fragments MUST fail before tool authorization. The parser MUST reject
duplicate terminal frames already buffered before settlement, then settle without
waiting indefinitely for EOF. Native encrypted reasoning and pending continuation
MUST remain actor-private and MUST NOT enter normalized events or public previews.

#### Scenario: Chunked native reply

- **WHEN** either native route splits UTF-8, multiline SSE, text/refusal or tool
  JSON across chunks and includes opaque reasoning before final tool output
- **THEN** visible deltas arrive before terminal completion, native identities
  reconcile correctly, and only final valid typed calls can execute.

#### Scenario: Full terminal without deltas

- **WHEN** a native stream provides a complete terminal response without prior
  fragments or done events for every output item
- **THEN** collection succeeds without imposing unsupported event choreography.

#### Scenario: Conflicting or interrupted wire data

- **WHEN** buffered terminal records conflict, observed fragments disagree with
  final native output, limits are exceeded, or cancellation interrupts streaming
- **THEN** collection fails or cancels with no partial tool dispatch and no native
  continuation leakage.
