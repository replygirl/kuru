## ADDED Requirements

### Requirement: Truthful versioned context provenance

The main memory schema MUST advance through a forward v6 migration that allows a context summary and its private reasoning records to carry exactly one real provenance kind: a conversation turn or an internal operation. An operation-attributed context record MUST identify its producing actor and invocation. Existing turn-attributed rows, serialized values, deterministic IDs and private state keys MUST retain their exact pre-v6 representation and identity, and the independently versioned usage registry MUST remain at v4.

#### Scenario: Existing turn records survive the forward migration

- **WHEN** a v5 store containing context summaries and private reasoning records is upgraded to v6
- **THEN** every historical row remains readable and exportable with its original turn attribution, serialized record format, deterministic ID and private state key, without an inferred operation or producer actor.

#### Scenario: Internal compaction records use operation provenance

- **WHEN** an accepted internal compaction checkpoints a context summary and private reasoning sidecar
- **THEN** the context record and every sidecar record carry the same real session, producer actor, invocation and operation, carry no fabricated turn, and use domain-separated operation identities.

### Requirement: Atomic context summary and private sidecar checkpoint

The memory owner MUST validate and publish an accepted context summary, its cursor advance and its bounded private reasoning sidecar in one revision-checked SQL transaction under one logical receipt. Any stale revision or cursor, invalid binding, identity conflict, cancellation before dispatch or rejected input MUST leave all three effects absent. An accepted reply loss MUST reconcile the whole checkpoint without redispatching provider work or publishing a prefix.

#### Scenario: Stale checkpoint has no side effects

- **WHEN** a checkpoint carries a stale source revision, moved cursor or source range that no longer matches its pinned view
- **THEN** the owner returns the typed definite stale result and writes no context summary, cursor or private reasoning key.

#### Scenario: Sidecar conflict has no prefix

- **WHEN** any operation-attributed sidecar key already exists with a different settled payload
- **THEN** the owner returns the typed definite reasoning-summary conflict and the transaction publishes none of the checkpoint, cursor or sidecar records.

#### Scenario: Accepted lost reply reconciles one atomic outcome

- **WHEN** the owner accepts and commits the combined checkpoint but the client loses its reply
- **THEN** the existing logical receipt proves the entire transaction committed, an exact retry is idempotent, and no second provider invocation or partial sidecar write occurs.

### Requirement: Combined checkpoint request bounds

The facade MUST validate the complete serialized checkpoint envelope before selecting or acquiring a local or remote mutating backend, and the owner MUST repeat the same validation before mutation. The complete typed payload MUST be limited to 64 MiB, remain below the managed 100 MiB frame after fixed RPC wrapping and reject any one-byte excess; component limits alone MUST NOT authorize a larger combined request.

#### Scenario: Worst-case escaping fits the managed frame

- **WHEN** the maximum accepted record count and aggregate payload use JSON control characters with worst-case escaping
- **THEN** the complete serialized checkpoint request remains within 64 MiB and below the managed frame limit.

#### Scenario: Invalid or oversized request never attaches mutably

- **WHEN** a checkpoint violates provenance binding, record invariants, batch count or the complete serialized size by one byte
- **THEN** the facade returns a definite validation error before backend selection or mutating attachment, and owner-side direct callers receive the same no-effect refusal.
