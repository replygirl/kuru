## ADDED Requirements

### Requirement: Honest memory maintenance documentation

The owning memory documentation SHALL explain automatic Dolt GC, mutation
receipt replacement and resolved candidate reclamation. It MUST state that Kuru
does not automatically expire conversations or notes, preserves reachable Dolt
revisions and unresolved candidates, continues to grow with retained history,
and does not provide secure erasure through GC.

#### Scenario: User reviews retention behavior

- **WHEN** a user reads either memory guide to understand storage growth or deletion
- **THEN** they can distinguish operational cleanup from user-history retention and find no automatic-expiry or secure-erasure claim.
