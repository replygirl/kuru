## Context

Providers already expose typed reasoning-summary deltas and the runtime has the
actual controlled turn and allocated provider invocation at admission. Those
values must reach a private memory record without treating the long-lived
Harness operation as a turn identity. P04 is introducing the common typed
runtime-to-ToolHost provenance tuple; P08 consumes that stable session, turn,
actor, and invocation identity and adds only summary-specific settled-item
coordinates.

## Goals / Non-Goals

**Goals:**

- Retain only provider-delivered reasoning summaries with exact settled-turn
  provenance.
- Make privacy boundaries structural: bounded presentation is a projection,
  while transcript, session export, fork presentation, and continuity never
  consume private records. The invoking owner's full-memory export and backup
  restore retain them.
- Make repeated completion/recovery idempotent against the admitted item
  identity.

**Non-Goals:**

- Reconstructing opaque provider reasoning, creating summaries from text, or
  changing provider request/continuation protocols.
- Cross-session recall, prompt injection, or a new general-purpose memory
  framework.
- Altering P27 candidate publication recovery or its pending-dream state.

## Decisions

### Persist the provider item, not a transcript-derived summary

The writer accepts only the normalized typed reasoning-summary item after a
successful terminal completion. Deriving text from the transcript was rejected
because it confuses model-visible content with provider-delivered metadata and
would create a new privacy source.

### Use admitted identity plus final-item coordinates for idempotency

The record key includes P04's session, turn, actor, and invocation identity and
the final settled item's output and summary coordinates. Harness operation IDs
and plain provider indexes alone were rejected because they can span turns or
collide across outputs.

### Keep display data as a one-way bounded projection

The writer stores producer-private data separately from public event and
transcript values. A dedicated projection supplies any visible summary state.
Embedding the private record in completed-turn output or generic runtime events
was rejected because those paths feed replay and consumers that must not receive
it. The existing active-store export is a full-fidelity private archival surface,
so it retains the record for backup and restore rather than silently dropping it.

### Producer-owned reuse is policy-selected, never ambient

The record is excluded from peer and public context assembly. Its producing
actor may use a policy-selected private replay or compaction path, but P08 does
not inject it into every request, concatenate it with another actor's history,
or make it available to cross-session continuity. This keeps reuse an explicit
producer-local decision.

### Depend on P04's provenance contract rather than introduce a second tuple

P08 will use `ToolInvocationContext` once P04 lands, extending it only with
summary/item coordinates local to this capability. A parallel runtime identity
type was rejected because it could silently diverge at provider admission.

## Integration contract

P04 owns `kuru_connectors::ToolInvocationContext` and creates it at the
admitted provider/tool boundary from the runtime session, controlled turn,
speaker actor, allocated `InvocationStart.invocation_id`, and concrete call
identity. P08 SHALL accept its session, turn, actor, and invocation fields as
inputs without reformatting or allocating substitutes. P08's memory writer adds
the reconciled successful final item's output and reasoning-summary coordinates
and returns only a bounded display projection. Provider fixtures must use the
same normalized completion/item shape as the runtime path; no raw provider wire
record or transcript string is an input to the writer.

## Risks / Trade-offs

- [Risk] A provider emits partial or conflicting summary coordinates before
  completion. → Persist only the reconciled successful terminal item and reject
  invalid identity combinations before writing.
- [Risk] A future public display feature accidentally reads the private record.
  → Keep projection APIs narrow and cover transcript, session-export, and fork
  exclusion with integration fixtures, while separately proving that the
  invoking owner's full-memory export/restore retains the record.
- [Risk] P04's uncommitted provenance implementation changes shape. → Keep P08
  blocked on that committed contract and avoid runtime source edits until it is
  available.
