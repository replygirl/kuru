## Context

`Event.detail` currently mixes plain text and serialized JSON and is copied into
broadcast, `TurnOutput` and the journal without a single outward projection.
The journal already makes exact retry safe, but CLI/TUI adapters discard stable
IDs and the TUI's interruption state is transient. Diagnostics and journal data
also have different lifecycles that the product does not currently disclose.

## Goals / Non-Goals

**Goals:** Project every semantic event before outward use and legacy replay;
name the two budgets and empty response honestly; expose exact local retry;
persist one safe interruption marker; disclose the debug ring; and document
durable no-expiry journal growth.

**Non-Goals:** Add a Phase 1 typed event envelope, remove useful nonsecret state
or peer semantics, browse arbitrary journal history, automatically retry,
replay ambiguous effects, expire history, or merge operational JSONL with
semantic events.

## Decisions

The existing `Event { kind, actor, detail }` shape stays. One constructor accepts
text or structured JSON, projects it through `project_text` or `project_json`,
then stores the projected serialization. Known structured legacy event kinds are
parsed and passed through the same path at replay. Response detail is always a
fixed completion marker.

`TurnOutput` gains additive typed outcome fields. Compatibility decoding maps a
missing old reason plus `limited: true` to `legacy-unspecified`; it never maps
old data to a specific budget. New output tracks `tool-calls` and `peer-rounds`
in a deduplicated set and tracks empty response separately.

A session-scoped opaque state key retains one format-versioned local submission
tuple. New local admission checkpoints it with the started journal and user row;
exact retry leaves it unchanged. An internal controlled-run result reports
whether a completed output was reused so the TUI can avoid display duplication.
A2A continues using its existing controlled method without changing this local
reference.

Interruption reconciliation checkpoints the journal transition and a dedicated
fixed-role transcript marker together, unless that logical turn already has a
marker or an ended output exists. Provider public context filters that internal
role. A later safe retry may add the answer while retaining the truthful earlier
marker.

The diagnostics guard retains its checked normalized directory for a debug-only
stderr notice. The operational ring remains four fixed files; journal rows remain
indefinite and proportional to turns.

## Risks / Trade-offs

- [Legacy journal contains malformed structured detail] → emit a fixed withheld
  marker, never raw fallback text.
- [Completed retry duplicates TUI presentation] → expose reuse in the internal
  result and treat it as status-only in the adapter.
- [Cancellation races an accepted answer] → reconcile durable journal/output
  before deciding to checkpoint a marker.
- [Marker leaks into prompts] → use a dedicated role and filter provider public
  context while retaining it in user transcript projection.
- [Journal grows indefinitely] → document proportional growth and keep it because
  no-expiry exact retry is the selected product contract.

## Operational surface

The change adds no bind address, listener, container/runner distinction, secret,
connection limit, binary version or architecture requirement. Its interactive
surface is the optional `run --turn-id`, TUI `/retry`, one fixed interruption
marker and a debug-only stderr path notice; all use existing local runtime,
memory and diagnostics ownership.
