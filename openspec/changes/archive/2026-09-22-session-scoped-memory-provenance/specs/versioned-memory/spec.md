## ADDED Requirements

### Requirement: Session provenance schema preservation

The current Dolt schema SHALL retain nullable session provenance on raw message rows and strict context-summary records and cursors. Migration, candidates, full-memory export and recovery MUST preserve the exact optional session identity, global sequence, summary provenance and cursor state; they MUST NOT derive a session from a namespace, current process or most recent conversation.

#### Scenario: Current schema upgrade preserves unattributed history
- **WHEN** a released schema store containing typed and legacy private rows upgrades
- **THEN** existing rows retain their exact sequences and payloads with null session identity, new attributed rows validate against the current schema, and historical candidate views continue to use their committed schema.

#### Scenario: Full-memory export
- **WHEN** an owner exports a committed revision containing attributed rows, unattributed rows, summaries and cursors
- **THEN** every optional session identity and provenance coordinate is represented exactly without moving a legacy row into session continuity.
