# Spec Delta

## MODIFIED Requirements

### Requirement: Candidate-independent operational ledger

Usage SHALL live on a per-project permanent operational Dolt branch outside live `main` and dream candidates. A usage commit MUST NOT advance the live revision, enter candidate content, alter promotion eligibility, or be lost when a candidate is abandoned or rejected. The branch SHALL be opened and validated before provider dispatch and follow the owned memory lifecycle; no general writable branch handle SHALL escape `kuru-memory`.

#### Scenario: Open candidate and usage
- **WHEN** a dream candidate remains open while its provider usage is durably observed
- **THEN** the live revision is unchanged, the candidate can still promote when otherwise valid, and usage appears once whether that candidate promotes or is abandoned.

#### Scenario: Invalid operational branch
- **WHEN** the ledger's owned schema, receipt contract or any ledger-owned row (malformed, non-canonically keyed or of an unrecognized class) cannot be validated or migrated at writable open
- **THEN** no unaccounted new provider work begins, while historical inspection reports its uncertainty rather than substituting live-branch totals.
