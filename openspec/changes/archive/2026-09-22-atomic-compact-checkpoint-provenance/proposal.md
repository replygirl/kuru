## Why

Internal context compaction can return provider reasoning summaries, but the current memory contract can only persist those private records in a separate mutation and requires every context summary to claim a conversation turn. A separate private-state write can make the already-captured context revision stale, while substituting an operation ID for a turn invents provenance that never existed.

The memory owner needs one bounded, receipt-bearing checkpoint that truthfully identifies either a real turn or a real internal operation and atomically publishes the context summary, cursor, and accepted private reasoning sidecar. Existing speaking-turn records and identities must remain byte-compatible, and the permanent usage ledger must remain on its independent schema version.

## What Changes

- Advance the main memory schema from v5 to v6 without rewriting existing context summaries or private reasoning records; keep the usage registry at v4.
- Represent context-summary and private-reasoning provenance as an exclusive real turn or real operation, and bind internal-operation checkpoints to the producing actor, session, invocation, and operation.
- Extend the context-summary checkpoint with a bounded private reasoning batch that is validated before a local or remote mutating attachment and repeated at the owner boundary.
- Persist the summary, cursor, and sidecar keys in the existing revision-checked transaction under one logical receipt, including lost-reply reconciliation and typed no-effect failures.
- Preserve the exact legacy record format, IDs, state keys, export/restore behavior, and historical rows for turn-attributed records.

## Capabilities

### New Capabilities

### Modified Capabilities

- `versioned-memory`: Add schema-v6 operation provenance and one atomic context-summary/private-sidecar checkpoint while retaining legacy identities and the separate usage schema.

## Impact

The change affects `packages/kuru-memory` migrations, typed records, export/restore, local and managed facades, service RPC framing and focused real-Dolt tests. It adds a forward main-schema migration and extends public serializable memory DTOs compatibly; it does not change provider execution, runtime compaction policy, public transcript projection, the usage branch schema, or product timeout/lifecycle behavior.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
