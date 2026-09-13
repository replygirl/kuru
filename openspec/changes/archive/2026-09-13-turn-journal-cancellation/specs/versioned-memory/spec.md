## ADDED Requirements

### Requirement: Atomic durable turn checkpoints

Each admitted turn SHALL retain a versioned application-state journal value containing its original bounded ID, exact request identity, ordered lifecycle transitions, possible-dispatch state, and any authoritative completed `TurnOutput`. Admission MUST atomically write the started journal state with exactly one early user transcript row. Completion MUST atomically write the ended journal state, exactly one assistant transcript row, and the matching session, topology, and session-index values. Interrupted turns MUST retain their user transcript and journal history without fabricating an assistant message. These records SHALL use the existing message and opaque state schema and remain ordinary durable user content.

#### Scenario: Interrupted admitted prompt
- **WHEN** a turn is cancelled or fails after its admission checkpoint and before completion
- **THEN** its user prompt and started/interrupted journal history remain durable with no assistant transcript row.

#### Scenario: Atomic completed answer
- **WHEN** completion is accepted or its acknowledgement is lost
- **THEN** reconciliation observes either the complete assistant/session/journal checkpoint or its complete absence, without a partial checkpoint or duplicate transcript row.

#### Scenario: Safe pre-dispatch resume
- **WHEN** a matching started or interrupted ID has no possible-dispatch marker
- **THEN** the harness may resume it without appending the existing user transcript again.

### Requirement: Dispatch uncertainty precedes actor work

The journal possible-dispatch marker MUST become durable before work is sent to an actor mailbox or any provider, tool, cognitive, or A2A operation can begin. Recovery MUST treat the marker conservatively and MUST NOT claim whether a particular external call occurred.

#### Scenario: Loss around actor admission
- **WHEN** cancellation or process loss occurs after possible-dispatch persistence but before or during actor work
- **THEN** recovery surfaces an incomplete possibly dispatched turn and does not duplicate private actor history or external effects through automatic replay.
