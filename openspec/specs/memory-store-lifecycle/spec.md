# memory-store-lifecycle Specification

## Purpose
Define explicit shared-store shutdown, durable reconciliation of uncertain
operations, and deterministic committed-revision inspection for memory clients.

## Requirements

### Requirement: Explicit shared-store shutdown

`MemoryStore::close` SHALL consume its calling view and await release of that view's client attachment. Dropping a view SHALL release only that view; after an explicit close, retained clones of that attachment SHALL reject reads and writes with the same clear closed-store error. Closing or dropping one attachment SHALL NOT stop a shared service or invalidate another client's attachment. The service owner SHALL await shutdown of its pools and owned supervisor as soon as the last client and accepted operation have drained, or after an authenticated maintenance request finds no other attached client. A read-only attachment SHALL not gain authority over an external owner through close or drop.

#### Scenario: A clone is dropped
- **WHEN** a caller drops one writable clone while retaining another
- **THEN** the retained clone can continue to read and write its live store

#### Scenario: A view is explicitly closed
- **WHEN** a caller explicitly closes one view while retaining a clone
- **THEN** reads and writes through the retained clone fail with `memory store is closed`, while an independent attachment remains usable

#### Scenario: Final client closes
- **WHEN** the final client closes after its accepted operations settle
- **THEN** the service shuts down without an idle interval and must reap Dolt before releasing the lifecycle lease

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

A writable memory open SHALL establish and validate one project-owned permanent usage branch under the existing lifecycle and writer leases before allowing provider dispatch. Validation of ledger-owned state SHALL be bound to the exact content of the branch's state table and to the identity of the validator that checked it. A writable open MAY rely on a durable validation record only when the record is on the branch head, names this binary's validator exactly and matches the live state content; otherwise it SHALL validate every ledger-owned row before allowing provider dispatch, and SHALL record the result only after that validation succeeds. Every ledger write SHALL validate each ledger-owned row it writes with the same row validator before commit, SHALL record the resulting state content in the same commit, and SHALL refuse to write if the branch's state no longer matches the content validated for this open. A missing, foreign or mismatched record MUST NOT be treated as evidence of validity. Its ledger-owned schema and operation receipts SHALL migrate independently of unrelated main-branch message migrations; it MUST NOT be rebased, fast-forwarded, promoted, or selected by candidate cleanup. Ledger writes SHALL use short serialized transactions, durable commit and the existing uncertain-write reconciliation protocol. Close, purge and reopen MUST account for this branch. Existing `memory export` SHALL remain a live memory snapshot and explicitly disclose that the operational usage ledger is excluded.

#### Scenario: Main migration and ledger reopen
- **WHEN** main's message schema advances while a project already has a usage branch
- **THEN** writable reopen validates or migrates only the ledger-owned contract and retains its records without rebasing to main.

#### Scenario: Uncertain usage commit
- **WHEN** the SQL response to a usage write is lost after commit may have occurred
- **THEN** the exact receipt is reconciled before another ledger mutation or a final outcome is claimed, with no duplicate observation.

#### Scenario: Project export and purge
- **WHEN** a user exports or purges a project containing usage
- **THEN** export clearly states that usage is excluded, and purge removes the owned project ledger with the project rather than leaving a detached branch.

#### Scenario: Recorded ledger reopens without re-decoding
- **WHEN** a writable open finds a head record naming this validator and matching the live state content
- **THEN** it activates the ledger without decoding ledger-owned rows.

#### Scenario: Unrecorded or mismatched ledger
- **WHEN** the head has no record, another validator's record, or a record whose content no longer matches
- **THEN** every ledger-owned row is validated before dispatch, the open is refused if any row is invalid, and a successful validation is recorded once.

#### Scenario: State changed under the writer
- **WHEN** a ledger write finds state content different from what this open validated
- **THEN** it commits nothing and further ledger writes refuse until a reopen revalidates.

#### Scenario: Committed write with a lost reply keeps the ledger writable
- **WHEN** the reply to a ledger write or a validation record commit is lost and reconciliation proves the commit happened
- **THEN** the validated content is re-derived from the head record and the next ledger write succeeds without a reopen.

### Requirement: Linear usage ledger validation

Validating ledger-owned usage state at writable open SHALL read each ledger-owned row a bounded number of times, so that its cost grows at most linearly with the number of ledger-owned rows. Owned-state paging SHALL select by an exact byte range on the state key and preserve byte-wise key order.

#### Scenario: Aged ledger opens within linear cost
- **WHEN** a writable open validates a ledger holding N ledger-owned rows
- **THEN** each page reads only its own key range, and validation cost grows at most linearly with N.

#### Scenario: Range paging preserves the owned set and order
- **WHEN** the state table holds keys inside the owned prefix and keys that only resemble it (the prefix without its final separator, keys that sort immediately before or after the range, and keys with high or NUL bytes)
- **THEN** owned-state paging visits exactly the keys that begin with the owned prefix, each once, in byte-wise order.

### Requirement: Usage validation record convention

The usage branch's validation record SHALL be one trailer line in a commit message on the branch head, `Kuru-Usage-State: <validator id> <state hash>`, where the validator id is ASCII of at most 128 bytes naming the release that validated, and the state hash is exactly 32 characters of `[0-9a-v]` naming the content hash of the branch's state table. Every ledger write commit SHALL carry the record for the content that commit produced. When a validated, non-empty ledger's head lacks the record for its own content, an open SHALL add one empty commit carrying it, and SHALL add none for a ledger holding no ledger-owned row. A reader SHALL accept a record only from a message of at most 512 bytes holding exactly one such line, on the commit that is the branch head; any other message, including a malformed, duplicated, oversize or non-head record, SHALL read as a missing record. A missing, foreign or mismatched record SHALL cause a full validation and a new record, and MUST NOT by itself refuse an open. The record SHALL add no schema, table or schema version, so a binary that does not know it is unaffected.

#### Scenario: Older binary's write leaves no record
- **WHEN** a binary that does not write the record commits to a recorded ledger and a newer binary then opens it
- **THEN** the newer binary validates every ledger-owned row once, records the result, and the following open decodes no rows.

#### Scenario: Release boundary
- **WHEN** the head record names a validator id from another release
- **THEN** the open validates every ledger-owned row, records the new validator id on success, and refuses only if a row is invalid.

#### Scenario: Forged or malformed record
- **WHEN** the head message carries a record with a wrong content hash, a non-head origin, a duplicated line, an oversize message or a malformed hash
- **THEN** it reads as a missing record and the open validates every ledger-owned row before dispatch.

#### Scenario: Unproven validation record commit
- **WHEN** the reply to an open's validation record commit is lost and reconciliation, after the original SQL session ended, cannot prove that the branch head carries that record
- **THEN** the record is treated as missing rather than refused: no later writer is blocked, the open keeps the content its full validation checked, and the next open validates every ledger-owned row and records again.

#### Scenario: Empty ledger writes no record
- **WHEN** a writable open finds a usage branch holding no ledger-owned row
- **THEN** it adds no commit to the branch and the first ledger write carries the record.

### Requirement: Canceled pooled-operation retirement

An abnormally dropped pooled memory session SHALL close its owned connection through a bounded disposal path without waiting for the canceled operation's pending SQL response. The connection SHALL retain its client pool permit until disposal completes or its existing close bound expires. Successful explicit release SHALL continue to return the connection inline for reuse. Candidate retirement SHALL retain its admission fence, existing caller deadline and exact server-session absence proof before renaming or deleting a branch; local pool counts alone MUST NOT authorize that transition. Uncertain writes SHALL retain their existing exact-session reconciliation fence.

#### Scenario: Return task already started after query cancellation

- **WHEN** a transaction is canceled after a real query executes but before its response reaches the client, and disposal begins while the pool remains open
- **THEN** pool retirement can finish without delivering the held query response, and branch transition still waits for exact server-session absence

#### Scenario: Successful pooled statement releases inline

- **WHEN** a successful statement or explicit transaction completion releases a session
- **THEN** the next statement can reuse the same idle connection before another connection is opened
