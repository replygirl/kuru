## Why

Kuru currently retains actor history but only sends a bounded recent suffix to each request. Older context falls out of the request without a durable, inspectable account of what the actor carried forward. Phase 2 needs each actor to maintain useful context within a model window while preserving every original row and the boundary between sessions, actors and candidate views.

## What Changes

- Add automatic, configurable window-fraction compaction and a working manual `/compact` path. A compaction request has its own output reserve and recorded usage; current input and active tool chains remain mandatory context.
- Persist a bounded, typed summary in the producing actor's private, session-scoped summary namespace with exact source ranges, captured view/revision, and summarizer operation/invocation provenance. Advance per-actor/session source cursors atomically with the summary and any returned producer-private reasoning sidecars; canceled, failed, invalid or stale attempts leave the prior projection usable.
- Assemble later requests from current-session raw rows plus policy-admitted notes and summaries. Preserve raw histories, candidates and legacy rows, and disclose which context was summarized without implying deletion or shared-pool memory.
- Coordinate the physical session-filtered sequenced snapshot and conditional summary checkpoint with P29. Keep provider-supplied reasoning sidecars producer-private and outside compaction/replay context even when an accepted Compact result persists them atomically with its summary.

## Capabilities

### New Capabilities

- `actor-context-compaction`: Per-actor context selection, durable summary provenance, cursor publication and automatic/manual compaction.

### Modified Capabilities

- `context-usage-accounting`: Measure and account for compaction requests and reserves without an extra provider call.
- `command-registry`: Expose `/compact` only when its real backend and notice are available.
- `mode-visibility-memory`: Apply mode visibility and memory admission to source selection and cross-session summaries.

## Impact

- `kuru-runtime` owns policy selection, compaction orchestration, notices and usage; `kuru-core` owns configuration, typed phases and budget contracts; `kuru-connectors` exposes pure estimates of the exact pending request shapes; `kuru-memory` and its typed service boundary own P29's strict summary/provenance records, session-filtered snapshot, cursor-selected projection and atomic publication checked against the captured revision's relevant source and prior summary.
- `apps/kuru-tui` owns manual command dispatch and visible notices; configuration and usage docs explain thresholds, cost and retained originals.
- P29's physical session-identity migration and source query are a hard integration prerequisite. Existing unattributed legacy rows are retained for inspection/export, not guessed into the next session. No automatic deletion, belief rewriting, cross-actor raw-history sharing or provider request for a preview estimate is introduced.

## Surfaces

- [x] interactive — `/compact` and automatic notices
- [ ] deploy — no service topology change in this slice
- [x] integration — typed storage/service and provider estimate contracts
- [x] agent-behavior — selected prompt context and summarization requests change
