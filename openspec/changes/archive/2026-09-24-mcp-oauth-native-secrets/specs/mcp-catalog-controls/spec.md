## ADDED Requirements

### Requirement: MCP authorization state remains alias local and non-authoritative

The existing MCP catalog projection SHALL add a bounded authorization state for each OAuth-enabled HTTP alias that distinguishes login required, authorized, refresh/relogin required, native-store unavailable and authorization failure without exposing tokens, client secrets, callback codes or raw remote bodies. Authorization state MUST NOT make cached metadata live, grant tool permission, start a provider turn, authorize another alias, or hide the existing disabled/live/stale/degraded availability state. Static-header aliases SHALL keep their existing header/session behavior and MUST NOT inherit OAuth credentials.

#### Scenario: Mixed static and OAuth aliases are inspected
- **WHEN** one HTTP alias is live through static headers, another requires OAuth login, a third has stale metadata and an expired credential, and a fourth is disabled
- **THEN** one catalog/status operation reports each availability and authorization state independently without resolving disabled credentials, creating routes from stale metadata, or printing secret material
