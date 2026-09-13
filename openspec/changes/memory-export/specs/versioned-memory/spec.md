## ADDED Requirements

### Requirement: Revision-pinned active memory export

Kuru SHALL expose a provider-free, read-only export of every application
`messages` and `state` row from one captured committed `main` revision. The
memory reader MUST reject a candidate view, capture the active revision once,
open and validate an exact commit-qualified pool, verify that pool resolves to
the captured revision and a supported schema, and retain that pool through all
pages. A concurrent writer MAY advance `main` after capture without changing
any page of the export. The export MUST exclude dirty working data,
candidate-only data, prior revisions, and operational or authority tables.

The reader SHALL preserve a signed message sequence, exact UTF-8 namespace,
role, content, exact state key, and the parsed JSON state value. It MUST page
messages in signed storage-key order and state in binary key order, use a
distinct first-page cursor rather than a numeric sentinel, release SQL
connections between bounded queries, and verify final emitted counts against
the captured committed counts. It MUST fail the complete export on unsupported
schema, invalid stored JSON or identifier, cursor/snapshot mismatch, query
failure, or count mismatch; it MUST NOT silently omit a record or dynamically
dump internal tables.

#### Scenario: Captured main stays coherent while it advances
- **WHEN** an export captures active `main`, a writer then advances `main`, and
  an unpromoted candidate or dirty working state exists
- **THEN** every export page and count comes from the captured committed hash,
  with no later, candidate, or dirty record included

#### Scenario: Signed and unknown application records survive pages
- **WHEN** stored messages include signed sequence boundaries and unknown
  namespaces while state includes unknown JSON fields and keys across page edges
- **THEN** each `messages` and `state` row appears exactly once in stable order
  with its unchanged storage identity and payload

#### Scenario: Export reader cannot cross snapshots
- **WHEN** an application reuses a page cursor with another captured snapshot
  or a later page/schema read fails
- **THEN** the read fails before a successful complete export is reported
