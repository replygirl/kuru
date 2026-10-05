# Spec Delta

## ADDED Requirements

### Requirement: Exact context summary confirmation omits private bodies

The existing memory boundary SHALL provide a validated read-only exact context-summary identity confirmation on the caller's selected live or candidate view. It SHALL inspect retained accepted summary records independently of the current cursor-selected window and return only bounded identity/provenance metadata or absence. It MUST NOT return summary text or reasoning/source bodies, activate a writable open for inspection, change revisions/receipts/cursors, or transfer candidate mutation authority. Existing uncertain-write fences and candidate recovery/inspection ownership SHALL remain authoritative before this confirmation is used for a runtime notice.

#### Scenario: Superseded accepted summary remains confirmable
- **WHEN** an exact accepted summary identity is requested after its cursor advanced to a later summary
- **THEN** the selected view confirms the original identity/provenance without exposing its summary body or mutating storage.

#### Scenario: Candidate isolation and invalid identity
- **WHEN** a candidate-only summary is queried from live main, or malformed identity/provenance is supplied
- **THEN** main does not observe the unpromoted candidate record and invalid input is refused before effects, with unchanged revision and no leaked private body.
