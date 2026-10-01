# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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

#### Scenario: Empty ledger writes no record
- **WHEN** a writable open finds a usage branch holding no ledger-owned row
- **THEN** it adds no commit to the branch and the first ledger write carries the record.
