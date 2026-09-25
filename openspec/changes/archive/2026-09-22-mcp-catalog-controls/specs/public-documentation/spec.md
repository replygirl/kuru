## ADDED Requirements

### Requirement: MCP catalog-control reference

Public configuration and tools documentation SHALL describe server enabled state, original-name allow and deny precedence, stale-cache status, explicit live recovery, environment-referenced static headers, and the distinction between catalog metadata, availability, workspace trust, and execution permission. It MUST state that header values stay in their configured environment variables and that OAuth, MCP resources, prompts, and sampling are outside this capability.

#### Scenario: User configures a controlled MCP catalog
- **WHEN** a user reads the MCP configuration and tools reference
- **THEN** they can configure a disabled or filtered server, interpret live/stale/degraded status, supply a static header by environment reference, and understand that a listed tool still requires a current route and permission
