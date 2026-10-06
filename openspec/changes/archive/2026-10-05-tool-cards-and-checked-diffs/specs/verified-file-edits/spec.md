## ADDED Requirements

### Requirement: Read-only checked receipt diff projection

A managed file card SHALL associate its exact admitted checkpoint operation identity with a read-only connector-owned diff projection of the retained, checked before/after receipt snapshots. Only proven applied UTF-8 text snapshots SHALL yield content diffs. Diff input, line/work and output limits SHALL be fixed; projection SHALL redact the complete supported input before truncating or selecting changed lines. Uncertain, binary, oversized, corrupt, missing and explicitly pruned receipts SHALL yield truthful unavailable/refused states. The projection SHALL neither reread current workspace contents nor retain a second full snapshot copy, change a receipt or bypass private project/root validation.

#### Scenario: Workspace changed after the managed edit
- **WHEN** unrelated workspace bytes change before a card is expanded
- **THEN** its diff still uses the checked recorded versions of the exact operation.

#### Scenario: Secret before a later changed line
- **WHEN** a recognized secret opener occurs before the displayed changed region
- **THEN** whole-input redaction prevents its body from leaking through selected diff lines.

#### Scenario: Explicit pruning or unproven effect
- **WHEN** the selected receipt has been pruned or has no proven applied effect
- **THEN** the card retains its known outcome and reports diff unavailability without guessing from the current file.
