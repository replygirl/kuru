## MODIFIED Requirements

### Requirement: Durable session continuity

The application SHALL persist a versioned session catalog, settled public transcript identity and session state outside tool roots and SHALL expose exact inspection, resume, deterministic continue and reversible lifecycle operations. A fresh process MUST reconstruct the selected session's stable public history and mode while resetting session-only grants. A removed session MUST remain retained but unavailable to ordinary resume/continue until restored, and P11 MUST retain the existing single conversation-driver boundary until the separate live-session admission change lands.

#### Scenario: Restart
- **WHEN** a process exits and a new process resumes the same ordinary session
- **THEN** its typed public conversation, settled speaker and turn identities, mode and peer topology remain available while session-only grants do not carry over.

#### Scenario: Continue after another session changes
- **WHEN** a fresh process requests continue after multiple sessions were created, renamed or removed
- **THEN** it selects the durable most recently updated nonremoved session and reports the exact selected identity before accepting input.

#### Scenario: Removed session requires restoration
- **WHEN** a caller tries to resume a removed session
- **THEN** the application reports its retained removed state and the explicit restore path without deleting data, silently restoring it or starting a provider request.
