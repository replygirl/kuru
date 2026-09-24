## ADDED Requirements

### Requirement: File checkpoint commands are working registry entries

The command registry SHALL expose bounded file checkpoint inspection, one selected file undo and explicit pruning only when their backing handlers are active. TUI and CLI file recovery paths SHALL describe an unresolved receipt honestly, keep file undo distinct from dream undo, and never advertise a placeholder command. A file undo attempt MUST use the same normal file authority checks as a native target mutation.

#### Scenario: Inspect and undo from a terminal
- **WHEN** a user inspects a settled checkpoint and selects its undo in a real terminal
- **THEN** the displayed ID/status/path identify that one edit and the handler reports either a verified restoration or a current-file conflict.
