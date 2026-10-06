## MODIFIED Requirements

### Requirement: Isolated durable revisioned memory

Memory SHALL preserve byte-sensitive namespace/key identity, message order, opaque JSON values and existing validation. Mutations SHALL be atomic versioned batches with operation identity for uncertain-response reconciliation. Views MUST remain pinned to their branch, and candidate promotion MUST require its exact proven effective live base and target. Internal reconciliation MAY merge newer live into a clean private candidate before promotion; it MUST NOT merge candidate changes directly into live or relax exact fast-forward proof. A writable app may record one project-scoped first-run notice version only after the notice has been successfully presented; that state records presentation, not reading or consent. Missing or lower versions are pending, current or higher integer versions are settled, and malformed values MUST fail without a new mutation. Memory and conversation history SHALL NOT expire automatically.

#### Scenario: Transaction failure
- **WHEN** a batch fails after one row change
- **THEN** no partial batch is visible and prior history remains readable.

#### Scenario: Lost acknowledgement
- **WHEN** a committed operation loses its response
- **THEN** reconciliation identifies its committed result without duplicating the mutation.

#### Scenario: Candidate divergence
- **WHEN** live memory changes after a candidate captures its base
- **THEN** promotion against the old base fails without overwriting either history; only a separately proven private reconciliation may yield a new exact effective base and target for checked fast-forward

#### Scenario: First-run presentation is durable
- **WHEN** a writable project successfully presents the pending notice
- **THEN** one ordinary committed state revision records its version and a reopened store does not present that version again.

#### Scenario: Presentation fails
- **WHEN** stderr output or the first completed TUI draw fails before the notice is visible
- **THEN** the notice version remains pending and no provider or peer history receives notice text.

## ADDED Requirements

### Requirement: Clean private candidate merge proof

Candidate reconciliation SHALL preserve immutable creation/reconciliation provenance separately from its effective checked promotion base. The owner MUST compare only exact committed revisions, require candidate/live schema parity, and keep existing default conflict/constraint refusal. Any merge failure or explicit overlap MUST leave the original candidate head and live head unchanged and the private working set clean, or retain uncertainty until cleanup is proved. Committed reconciliation MUST retain versions and rows from disjoint write sets, with exact request-bound target/parent evidence sufficient for restart recovery. Historical candidates MUST remain inspectable and SHALL NOT be merged across schema generations. No row conflict may be resolved automatically and no later state version may be reset by reconciliation.

#### Scenario: Clean merge retains both sides
- **WHEN** a current-schema candidate and live contain disjoint committed changes
- **THEN** private reconciliation retains both sets and versions, records exact merge provenance and permits only fast-forward publication of its proven target

#### Scenario: Collision does not become a silent overwrite
- **WHEN** the two branches conflict on membership, message identity, another row or a constraint
- **THEN** conflict remains explicit, both original heads and history remain readable, and the candidate working set is clean before another mutation is admitted

#### Scenario: Historical schema stays historical
- **WHEN** an open candidate predates live schema evolution
- **THEN** reconciliation refuses before merge, preserving its exact branch/history for inspection and explicit checked resolution
