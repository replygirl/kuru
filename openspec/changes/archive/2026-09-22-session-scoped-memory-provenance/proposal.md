## Why

Kuru's private actor and relationship histories currently share one namespace stream across every conversation in a project. The managed memory service now provides the durable receipt and pinned-view boundaries needed to add physical session provenance without guessing ownership for legacy rows, which is the prerequisite for independent context compaction and later concurrent-session admission.

## What Changes

- Add nullable physical session identity to raw private-history rows while preserving the global sequence allocator and every unattributed legacy row.
- Expose a bounded typed source snapshot bound to one actor namespace, session, pinned live or candidate view, captured revision, and explicit sequence range.
- Publish one strict `context_summary.v1` record and its per-actor/session/source cursor atomically, rejecting a stale view, revision, range, or prior cursor with no partial effect.
- Read only each cursor-selected current summary through a bounded typed projection, retaining its exact identity and provenance while historical summaries remain export/audit data.
- Carry snapshot and checkpoint operations through the managed typed facade and receipt-bearing RPC so an accepted lost reply reconciles against the original view and arguments.
- Build each turn from a coherent topology snapshot, session-filter raw private history, and admit only the cross-session notes or summaries selected by the active mode's existing memory policy.
- Serialize dream inference and promotion through a cancellation-safe owner lease that does not hold a SQL transaction or block ordinary conversation writes; retain the candidate and report on a moved live base and require explicit resolution.
- Keep ordinary conversation-driver admission unchanged. P30 alone enables concurrent process/session drivers.

## Capabilities

### New Capabilities

- `session-scoped-memory-provenance`: Physical session identity, bounded sequenced source snapshots, and atomic conditional summary-cursor publication.

### Modified Capabilities

- `versioned-memory`: Migrate retained private history without attributing ambiguous legacy rows, and preserve session identity through export, recovery, candidates, and historical reads.
- `durable-service-receipts`: Carry the new conditional mutation through authenticated managed views with exact lost-reply reconciliation and definite stale rejection.

## Impact

- `packages/kuru-memory/src/store.rs`, `store/migrations.rs`, `store/export.rs`, `facade.rs`, `service.rs`, and `service/rpc.rs` gain the schema migration, typed DTOs, queries, transaction, and managed operations.
- Existing raw-history append callers gain an explicit session-bearing path; runtime continuity uses session-filtered raw reads while legacy namespace-only reads remain available for inspection/export.
- Runtime turn preparation reloads one coherent topology and holds it for that turn. Dream work uses a separate owner lease, while ordinary chat remains admitted by the existing driver and storage boundaries.
- The current single conversation-driver lease remains authoritative until P30; no SQL transaction or memory-service mutation guard spans provider inference.
- P09's `actor-context-compaction` can consume the real snapshot/checkpoint interface after this change lands.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
