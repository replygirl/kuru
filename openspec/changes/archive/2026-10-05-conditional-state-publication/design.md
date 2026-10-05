# Design

## Context

The concrete facade routes typed operations to one owner. That owner's write
mutex and receipt-bearing transaction already serialize state writes. The current
main registry is schema 8; usage stays schema 4. See proposal and delta specs for
scope and requirements.

## Goals / Non-Goals

Goals: provide a store-owned compare/write primitive using the existing mutation
worker, including exact lost-reply recovery. Avoid touching runtime publication
until its separately reviewed topology split.

Non-goals: SQL advisory locks, storage traits, actor admission, topology migration,
new configuration or new timeouts.

## Decisions

- Add schema 9 with `state.version BIGINT NOT NULL DEFAULT 0`. A stored signed
  value must be nonnegative; public versions are u64 but expectations above
  i64::MAX are rejected. An explicit integrity preflight rejects exhausted versions before any row writes; the transaction also retains SQL overflow rejection. Comparing stored
  JSON was rejected because serialization order is not a version.
- Keep a new `store/versioned_state.rs` module for types, validation, reads and
  conditional mutation preparation. A single SELECT reads a bounded batch and
  the result is reconstructed in request order with explicit missing values.
  Duplicate keys are rejected rather than ambiguously compared or written twice.
- Compare every expected row under the existing owner write guard inside the
  mutation transaction before changing any row. Every expected key must occur in
  the value batch; companion unconditional values are allowed. Row locks are not
  load-bearing. A stale comparison rolls back without a receipt or revision.
- All actual state overwrites, including checkpoint values, advance versions.
  Reasoning-sidecar insert-only paths insert version zero and their idempotent
  no-op does not pretend to be a write. Historical schema paths keep their old
  SQL; versioned APIs require schema 9. Imports insert new rows at zero.
- Candidate promotion only fast-forwards from the unchanged live base, so it
  cannot install an older version over intervening live writes. Candidate schema
  checks, generation-bound attachments and exact transition receipts remain the
  authority; restore to an absent destination does not reuse an old attachment.
  Compensating state writes use ordinary incrementing upserts. No version ABA is
  introduced by comparing values or resetting versions.
- Add explicit RPC read/write variants, values and a typed stale fault carrying
  only bounded key/version metadata. The operation uses the existing logical unit
  receipt digest, fence and outcome query. A definite rejection clears its remote
  pending unit; a lost reply remains uncertain until exact receipt resolution.
- Defer split topology materialization to PR B's migration from the then-current
  legacy blob. Materializing in PR A was rejected because legacy runtime writes
  continue between the two changes and would leave split rows stale.

## Risks / Trade-offs

- [Schema bump] -> retain all historical data, validate the new column on every
  host, and test old-registry writable refusal without mutation.
- [Missed writer] -> audit every state SQL write and exercise unconditional,
  checkpoint, candidate and idempotent-sidecar behavior with real Dolt.
- [Wire drift] -> one minor bump, exact contract fixtures and protocol pin.
- [Loaded fixtures] -> causal barriers and existing operation/startup budgets;
  no guessed positive sleeps and no competing coverage writer.

Conditional requests are bounded to 16 MiB for the exact JSON tuple of expectations and values (including escaping). Batch reads are bounded to 16 MiB cumulative stored JSON, consume rows incrementally inside one read transaction, and suppress a single oversized value in SQL before transfer. Existing scalar and unconditional APIs retain their prior size behavior.

Writer audit: ordinary state writes and every turn/checkpoint state batch use the shared version-aware upsert. Import inserts and absent-only reasoning sidecars use default zero; settled sidecar no-ops preserve the version. Context-summary cursors are separate typed tables, not hidden state overwrites. The private usage ledger is a separate inaccessible schema-4 branch and keeps its own SQL. Candidate promotion requires the unchanged exact live base before fast-forward; compensating runtime writes use the ordinary incrementing API. Whole-directory restore requires an absent destination/new generation, so no old attachment token survives it.

The versioned batch read `get_many_versioned` returns value/version pairs from the same bounded streamed SELECT, with missing rows preserved. It requires schema 9. Its private reader is shared with value-only `get_many`, which selects a literal placeholder version on historical schemas and does not expose that placeholder. This is necessary for the next runtime slice to load membership and its state coherently without per-key RPCs or domain-level duplicate versions.

The batch API supplies coherent storage observations; it does not change which runtime writes need CAS. The existing per-identity last-report policy and single-driver checkpoint/journal contract remain product decisions for PR B. No dummy unchanged membership write is authorized as a substitute for a read-only membership predicate.
