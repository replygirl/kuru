## Why

Semantic turn events can currently carry unprojected error, state and peer data
to broadcasts, JSON output and durable journals, while the response event
duplicates the authoritative answer. Exact completed retry is implemented but
is not reachable from the ordinary CLI/TUI, interruptions survive only as
journal state, and `--debug` does not disclose its bounded ring location.

## What Changes

- Project semantic event detail through the shared redaction boundary before
  outward use and on legacy replay, preserving structured state/peer semantics
  while replacing the response body with a completion marker.
- Name tool-call and peer-round limits separately from an empty response while
  retaining `limited` and old-record compatibility.
- Expose exact turn IDs to `kuru run` and a durable last-submission `/retry` path
  without replaying possibly dispatched work or duplicating completed UI rows.
- Commit a minimal deduplicated transcript interruption marker alongside journal
  state, with a completed checkpoint winning races.
- Print the resolved debug-ring directory on stderr and document it separately
  from the durable no-expiry turn journal.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `chat-harness`: Scripted and interactive retry, honest completion metadata and
  durable interruption presentation.
- `peer-cognition`: Semantic events are safely projected and resource outcomes
  are distinct.
- `versioned-memory`: Session-scoped retry state and interruption markers remain
  atomic with durable journal/transcript state and have no expiry.
- `operational-diagnostics`: Debug mode reports the checked bounded ring path on
  stderr without contaminating command output.

## Impact

This changes runtime turn/event output and compatibility decoding, the TUI/CLI
adapter and diagnostics guard, focused runtime/TUI tests, and session/runtime
documentation. Existing journals remain readable and are projected on replay;
no history is rewritten, expired or deleted.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
