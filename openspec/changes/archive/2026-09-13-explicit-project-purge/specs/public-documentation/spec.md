## ADDED Requirements

### Requirement: Explicit project-purge boundary

Curated documentation SHALL explain the confirmed project-memory purge command,
that it removes the selected managed Dolt current store and revision history, and
that it does not promise secure physical erasure. It MUST distinguish retained
original/shared legacy SQLite, user exports/backups, engine cache, and other
projects, and state that selected-note forgetting retains historical revisions.

#### Scenario: User evaluates deletion scope
- **WHEN** a user reads memory controls before confirming a purge
- **THEN** they can distinguish managed project-history removal from retained
  shared/original copies and do not receive a secure-erasure or automatic-expiry
  promise.
