## ADDED Requirements

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
