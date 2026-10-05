# Spec Delta

## ADDED Requirements

### Requirement: Atomic revision-conditional state publication

Current writable memory SHALL expose state row versions and atomically compare
all requested expectations before publishing any values in a batch. Expectations
SHALL distinguish an absent key from an existing version, each expected key MUST
be written, and duplicate, malformed, oversized or empty conditional requests
MUST fail without mutation or receipt. Every actual overwrite of a state row
SHALL advance its nonnegative version, including unconditional checkpoints;
new rows SHALL start at zero. Overflow MUST fail without partial publication.
A stale expectation SHALL return a typed key/expected/actual refusal and create
neither a durable receipt nor a revision. Accepted work SHALL retain the ordinary
exact receipt and uncertain-write recovery guarantees. Versioned operations MUST
refuse historical schemas without mutating them.

#### Scenario: Concurrent compare and publish
- **WHEN** two writers publish against the same observed version
- **THEN** exactly one succeeds and the other receives a stale refusal; retry
  using the new version can succeed without erasing the winner

#### Scenario: Mixed batch has a stale key
- **WHEN** any expected key is stale while companion keys would otherwise succeed
- **THEN** no key, receipt or revision changes

#### Scenario: Ordinary checkpoint intervenes
- **WHEN** an unconditional write or checkpoint overwrites an observed key
- **THEN** the earlier version expectation is stale even if values are equal

#### Scenario: Accepted response is lost
- **WHEN** a conditional mutation commits but its response is lost
- **THEN** exact unit receipt recovery proves that mutation without replaying it

#### Scenario: Schema migration preserves state
- **WHEN** schema 8 upgrades to the row-version schema
- **THEN** state values and legacy topology bytes are retained with zero versions
  and an older writable registry refuses the result without changing it

### Requirement: Bounded consistent state batch reads

Memory SHALL return one consistent SELECT batch in requested order, including
explicit missing values, for at most 256 distinct validated state keys. Empty
reads SHALL return an empty batch. A current-schema versioned batch SHALL return
each value and its version from that same snapshot. Historical value-only batch
reads SHALL remain supported; versioned batches MUST refuse historical schemas
without effects. Invalid or excessive input MUST fail before
mutation; exports SHALL remain value-only.

#### Scenario: Ordered keys and a missing row
- **WHEN** a bounded batch requests existing keys out of database order and an absent key
- **THEN** results preserve request order and include the absent key with no value

#### Scenario: Invalid batch
- **WHEN** a request has duplicate or more than 256 keys
- **THEN** memory refuses it without changing values, receipts or revisions

Conditional requests are bounded to 16 MiB for the exact JSON tuple of expectations and values (including escaping). Batch reads are bounded to 16 MiB cumulative stored JSON, consume rows incrementally inside one read transaction, and suppress a single oversized value in SQL before transfer. Existing scalar and unconditional APIs retain their prior size behavior.

#### Scenario: Coherent publication tokens
- **WHEN** a writer changes multiple state rows together while a versioned batch reads them
- **THEN** every returned value and version comes from one snapshot, preserving request order and explicit missing rows
