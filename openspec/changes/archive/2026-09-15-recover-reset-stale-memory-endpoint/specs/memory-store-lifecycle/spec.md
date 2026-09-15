## ADDED Requirements

### Requirement: Checked stale endpoint recovery

When a published memory endpoint accepts the bounded raw TCP availability probe but its authenticated SQL connection returns a typed connection-reset I/O error before the identity-verification callback begins, a writable open SHALL treat that endpoint as unavailable and continue through the existing lifecycle lease and owned server startup path. Authentication rejection, project or instance mismatch, data-directory mismatch, SQL protocol or database errors, connection I/O after the verification callback begins, and every other endpoint result MUST remain terminal under their existing diagnostics and ownership rules.

#### Scenario: Reaped endpoint resets before authentication callback
- **WHEN** an owned fixture accepts the raw published-endpoint probe and resets the following SQL authentication connection before the identity callback begins
- **THEN** a writable open acquires the existing lifecycle authority, starts its owned server and reopens the same committed state without weakening any identity or data-directory check

#### Scenario: Endpoint failure crosses the recovery boundary
- **WHEN** endpoint authentication, protocol, SQL, identity or data-directory verification fails, or an I/O error occurs after the identity callback begins
- **THEN** the open returns the original failure and does not classify the endpoint as unavailable or start a replacement server
