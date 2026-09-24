## ADDED Requirements

### Requirement: Managed session-provenance operations

The authenticated memory service SHALL expose session-bearing append, revision-bound source snapshot and conditional summary-checkpoint operations through the same pinned local or remote view contract. Snapshot requests SHALL remain read-only. Checkpoint requests SHALL carry a bounded canonical fingerprint of the complete record and source proof, use the durable unit-receipt outcome protocol, and distinguish a definite pre-effect stale rejection from transport or storage uncertainty.

#### Scenario: Local and remote views agree
- **WHEN** the same valid snapshot or checkpoint is exercised through local and managed remote stores at the same pinned view
- **THEN** both paths validate the same DTO bounds and return the same typed rows, revision coordinates and stale-domain result.

#### Scenario: Definite stale rejection leaves the attachment usable
- **WHEN** an owner proves before mutation that a checkpoint revision, cursor or range is stale
- **THEN** the service returns the typed complete rejection, clears only that request's pending state and permits a corrected request without an outcome poll.

#### Scenario: Incomplete checkpoint reply fences mutation
- **WHEN** a checkpoint frame may have been accepted but no complete authenticated reply is available
- **THEN** sibling mutations remain fenced until the original receipt outcome is proved, and reconnect alone does not classify the checkpoint as committed or absent.
