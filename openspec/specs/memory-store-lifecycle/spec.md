# memory-store-lifecycle Specification

## Purpose
Define explicit shared-store shutdown, durable reconciliation of uncertain
operations, and deterministic committed-revision inspection for memory clients.

## Requirements

### Requirement: Explicit shared-store shutdown

`MemoryStore::close` SHALL consume its calling view and await shutdown of the
shared server and its pools. Dropping a view SHALL release only that view; after
an explicit close, retained clones SHALL reject reads and writes with the same
clear closed-store error. A read-only attachment SHALL not gain authority over
an external owner through close or drop.

#### Scenario: A clone is dropped
- **WHEN** a caller drops one writable clone while retaining another
- **THEN** the retained clone can continue to read and write its live store

#### Scenario: A view is explicitly closed
- **WHEN** a caller explicitly closes one view while retaining a clone
- **THEN** reads and writes through the retained clone fail with `memory store is closed`, and a later owner can open after awaited cleanup

### Requirement: Reconciliation outcome

`MemoryStore::reconcile` SHALL return `Result<Option<bool>>`: `None` when no
operation was pending, `Some(true)` when the pending operation is durably
committed, and `Some(false)` when it is not committed. Callers SHALL continue to
use exact durable values before publishing pending in-memory state.

#### Scenario: A durable reply is lost
- **WHEN** an operation receipt is pending after its original SQL session ends
- **THEN** reconciliation reports `Some(true)` only when the durable receipt exists and does not replay the operation

### Requirement: Deterministic revision graph order

`MemoryStore::revisions` SHALL return the newest bounded revisions in the
pinned Dolt graph order and SHALL use commit hash as a stable tie-break. It
SHALL NOT use wall-clock timestamps as the ordering authority.

#### Scenario: Commits share a timestamp
- **WHEN** an ancestor and descendant have tied wall-clock timestamps
- **THEN** the newer graph entry appears first in a bounded revision result

### Requirement: Checked stale endpoint recovery

When a published memory endpoint accepts the bounded raw TCP availability probe but its authenticated SQL connection returns a typed connection-reset I/O error before the identity-verification callback begins, a writable open SHALL treat that endpoint as unavailable and continue through the existing lifecycle lease and owned server startup path. Authentication rejection, project or instance mismatch, data-directory mismatch, SQL protocol or database errors, connection I/O after the verification callback begins, and every other endpoint result MUST remain terminal under their existing diagnostics and ownership rules.

#### Scenario: Reaped endpoint resets before authentication callback
- **WHEN** an owned fixture accepts the raw published-endpoint probe and resets the following SQL authentication connection before the identity callback begins
- **THEN** a writable open acquires the existing lifecycle authority, starts its owned server and reopens the same committed state without weakening any identity or data-directory check

#### Scenario: Endpoint failure crosses the recovery boundary
- **WHEN** endpoint authentication, protocol, SQL, identity or data-directory verification fails, or an I/O error occurs after the identity callback begins
- **THEN** the open returns the original failure and does not classify the endpoint as unavailable or start a replacement server
