## MODIFIED Requirements

### Requirement: Atomic durable turn checkpoints

Each admitted turn SHALL retain a versioned application-state journal value
containing its original bounded ID, exact request identity, ordered lifecycle
transitions, possible-dispatch state, and any authoritative completed
`TurnOutput`. A new local CLI/TUI submission MUST atomically write the started
journal state, exactly one early user transcript row and the session-scoped
single last-submission tuple. Completion MUST atomically write the ended journal
state, exactly one assistant transcript row, and the matching session, topology,
and session-index values. Interrupted turns MUST atomically retain their user
transcript and journal history with exactly one fixed interruption transcript
marker and no fabricated assistant message. A completed-answer reconciliation
MUST add no marker. These records SHALL use the existing message and opaque state
schema and remain ordinary durable user content.

Turn-journal rows SHALL be durable no-expiry safety and idempotency history.
Their aggregate storage grows in proportion to admitted turns; completed result
data remains retained for indefinite exact retry even though the response event
does not duplicate the answer body. Kuru MUST NOT cap, expire, fold into
transcript metadata, rewrite or automatically delete these rows.

#### Scenario: Interrupted admitted prompt
- **WHEN** a turn is cancelled or fails after its admission checkpoint and before completion
- **THEN** its user prompt, one interruption marker and started/interrupted journal history remain durable with no assistant transcript row.

#### Scenario: Atomic completed answer
- **WHEN** completion is accepted or its acknowledgement is lost
- **THEN** reconciliation observes either the complete assistant/session/journal checkpoint or its complete absence, without a marker, partial checkpoint or duplicate transcript row.

#### Scenario: Safe pre-dispatch resume
- **WHEN** a matching started or interrupted ID has no possible-dispatch marker
- **THEN** the harness may resume it without appending the existing user transcript or replacing the last-submission tuple.

#### Scenario: Retained completed history
- **WHEN** many turns complete and later exact retries are requested
- **THEN** each session-scoped journal row and completed output remains available without expiry or aggregate-cap deletion.
