## MODIFIED Requirements

### Requirement: Isolated durable revisioned memory

Memory SHALL preserve byte-sensitive namespace/key identity, message order, opaque JSON values and existing validation. Mutations SHALL be atomic versioned batches with operation identity for uncertain-response reconciliation. Views MUST remain pinned to their branch, and candidate promotion MUST require its live base. A writable app may record one project-scoped first-run notice version only after the notice has been successfully presented; that state records presentation, not reading or consent. Missing or lower versions are pending, current or higher integer versions are settled, and malformed values MUST fail without a new mutation. Memory and conversation history SHALL NOT expire automatically.

#### Scenario: Transaction failure
- **WHEN** a batch fails after one row change
- **THEN** no partial batch is visible and prior history remains readable.

#### Scenario: Lost acknowledgement
- **WHEN** a committed operation loses its response
- **THEN** reconciliation identifies its committed result without duplicating the mutation.

#### Scenario: Candidate divergence
- **WHEN** live memory changes after a candidate captures its base
- **THEN** promotion fails without overwriting either history.

#### Scenario: First-run presentation is durable
- **WHEN** a writable project successfully presents the pending notice
- **THEN** one ordinary committed state revision records its version and a reopened store does not present that version again.

#### Scenario: Presentation fails
- **WHEN** stderr output or the first completed TUI draw fails before the notice is visible
- **THEN** the notice version remains pending and no provider or peer history receives notice text.
