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
