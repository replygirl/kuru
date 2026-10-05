# Spec Delta

## ADDED Requirements

### Requirement: Immutable selected-view state read cuts

Memory SHALL expose a checked committed-revision state cut for the selected live
or candidate view. All versioned point reads and ordered binary-prefix pages
SHALL use that same immutable revision. No SQL transaction or writer lock SHALL
be held between requests. Pages MUST bound row count and encoded reply bytes,
allowing one larger row when it fits the exact existing service envelope; no
small-batch limit SHALL become a new retained graph or scalar limit. Explicit
close MUST release this cut's resources without closing another shared reader.

#### Scenario: Updates occur between pages
- **WHEN** membership, session or reports change after a cut is captured and before later pages
- **THEN** every cut read retains the original revision's complete values and versions while new cuts observe the accepted updates

#### Scenario: Large retained state inventory
- **WHEN** more than 256 report rows or a formerly legal row exceeds the small batch target
- **THEN** bounded pages make progress and preserve every retained identity without imposing a graph or 16 MiB scalar cap

#### Scenario: Reader closes
- **WHEN** one cut is closed while another reader holds the same revision
- **THEN** the closed handle refuses further reads and the other reader remains valid

### Requirement: Compatible topology state materialization

The next schema migration SHALL split the then-latest canonical legacy topology
into membership and every per-identity report inside its existing staged build
transaction before version advancement. Membership SHALL contain parts and
relationships using `membership.v1`; report keys SHALL use lowercase SHA-256 of
exact identity UTF-8 with `state_report.v1` records retaining that identity and
raw report. Migration MUST preserve original topology bytes and raw copied
member/report fields, including archived and extra report identities. Ambiguous
legacy project focus SHALL remain unassigned. Malformed sources, identity/hash
mismatches and conflicting destinations MUST refuse atomically without partial
live publication. Historical branches SHALL remain inspectable without rewrite.

#### Scenario: Legacy extra report identities
- **WHEN** valid legacy state maps contain archived, slash-containing or long extra identities
- **THEN** all reports round-trip with exact identity and payload under bounded hashed row keys and the original topology stays byte-identical

#### Scenario: Split destination conflicts
- **WHEN** an existing membership or hashed report destination disagrees with the legacy materialization
- **THEN** migration refuses before schema advance and leaves active legacy data unchanged

## MODIFIED Requirements

### Requirement: Bounded consistent state batch reads

Memory SHALL return one consistent SELECT batch in requested order, including
explicit missing values, for at most 256 distinct validated state keys. Empty
reads SHALL return an empty batch. A current-schema versioned batch SHALL return
each value and its version from that same snapshot. Historical value-only batch
reads SHALL remain supported; versioned batches MUST refuse historical schemas
without effects. Invalid or excessive input MUST fail before mutation; exports
SHALL remain value-only. Small batch reads SHALL retain their 16 MiB cumulative
stored-JSON bound with streamed consumption and SQL suppression of individually
oversized values. Scalar APIs SHALL retain their existing size behavior.

Conditional publication SHALL retain its key/count/expectation constraints and
validate the exact complete request against the existing 100 MiB managed service
envelope before mutation or receipt; it SHALL NOT inherit the small-read byte
bound as a membership or graph limit.

#### Scenario: Ordered keys and a missing row
- **WHEN** a bounded batch requests existing keys out of database order and an absent key
- **THEN** results preserve request order and include the absent key with no value

#### Scenario: Invalid batch
- **WHEN** a request has duplicate or more than 256 keys
- **THEN** memory refuses it without changing values, receipts or revisions

#### Scenario: Coherent publication tokens
- **WHEN** a writer changes multiple state rows together while a versioned batch reads them
- **THEN** every returned value and version comes from one snapshot, preserving request order and explicit missing rows

#### Scenario: Formerly legal larger conditional membership
- **WHEN** a conditional membership request exceeds 16 MiB but fits the exact complete managed service envelope
- **THEN** the ordinary atomic conditional operation accepts it without a separate domain operation
