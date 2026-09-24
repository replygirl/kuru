# memory-store-lifecycle Specification

## Purpose
Define explicit shared-store shutdown, durable reconciliation of uncertain
operations, and deterministic committed-revision inspection for memory clients.

## Requirements

### Requirement: Explicit shared-store shutdown

`MemoryStore::close` SHALL consume its calling view and await release of that view's client attachment. Dropping a view SHALL release only that view; after an explicit close, retained clones of that attachment SHALL reject reads and writes with the same clear closed-store error. Closing or dropping one attachment SHALL NOT stop a shared service or invalidate another client's attachment. The service owner SHALL await shutdown of its pools and owned supervisor after the last client and accepted operation have drained and the idle interval expires, or after an authenticated maintenance request finds no other attached client. A read-only attachment SHALL not gain authority over an external owner through close or drop.

#### Scenario: A clone is dropped
- **WHEN** a caller drops one writable clone while retaining another
- **THEN** the retained clone can continue to read and write its live store

#### Scenario: A view is explicitly closed
- **WHEN** a caller explicitly closes one view while retaining a clone
- **THEN** reads and writes through the retained clone fail with `memory store is closed`, while an independent attachment remains usable

#### Scenario: Final client closes
- **WHEN** the final client closes after its accepted operations settle
- **THEN** the service waits its idle interval unless authorized maintenance requests early retirement, and must reap Dolt before releasing the lifecycle lease

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

### Requirement: Permanent operational usage branch lifecycle

A writable memory open SHALL establish and validate one project-owned permanent usage branch under the existing lifecycle and writer leases before allowing provider dispatch. Its ledger-owned schema and operation receipts SHALL migrate independently of unrelated main-branch message migrations; it MUST NOT be rebased, fast-forwarded, promoted, or selected by candidate cleanup. Ledger writes SHALL use short serialized transactions, durable commit and the existing uncertain-write reconciliation protocol. Close, purge and reopen MUST account for this branch. Existing `memory export` SHALL remain a live memory snapshot and explicitly disclose that the operational usage ledger is excluded.

#### Scenario: Main migration and ledger reopen
- **WHEN** main's message schema advances while a project already has a usage branch
- **THEN** writable reopen validates or migrates only the ledger-owned contract and retains its records without rebasing to main.

#### Scenario: Uncertain usage commit
- **WHEN** the SQL response to a usage write is lost after commit may have occurred
- **THEN** the exact receipt is reconciled before another ledger mutation or a final outcome is claimed, with no duplicate observation.

#### Scenario: Project export and purge
- **WHEN** a user exports or purges a project containing usage
- **THEN** export clearly states that usage is excluded, and purge removes the owned project ledger with the project rather than leaving a detached branch.
