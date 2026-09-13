## ADDED Requirements

### Requirement: Explicit managed-project purge

Kuru SHALL expose an explicitly confirmed, canonical-project-scoped purge that
removes the verified managed Dolt store and all of its managed revision history.
It MUST acquire and verify the command writer/startup/lifecycle authority before
publishing any destructive intent, MUST NOT provision an engine, import legacy
SQLite, construct a provider, or kill another owner, and MUST retain stable lock
objects. One checked control record MUST retain legacy-import suppression and any
incomplete original/quarantine identity inventory until removal is verified. An
ordinary open MUST reject incomplete purge authority; after completed purge it MAY
create an empty fresh store but MUST NOT automatically import the suppressed
project from legacy SQLite.

#### Scenario: Confirmed project purge
- **WHEN** the user confirms purge for a quiescent managed project
- **THEN** Kuru removes only that project's verified active and retained managed
  recovery trees, verifies their absence, and records completed legacy-import
  suppression without altering another project, shared legacy source, export,
  engine cache, or stable lock.

#### Scenario: Live owner or interrupted removal
- **WHEN** lifecycle authority cannot be acquired or removal is interrupted
- **THEN** no unverified path is removed, a live-owner refusal leaves no new
  purge record, and a retry uses the recorded identities rather than a
  replacement occupying an original pathname.

#### Scenario: Legacy reopen after purge
- **WHEN** a project with importable legacy SQLite is purged and later opened
- **THEN** the project opens as a fresh empty managed store without resurrecting
  the suppressed project's legacy rows while other project imports remain
  available.
