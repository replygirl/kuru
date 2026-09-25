## ADDED Requirements

### Requirement: MCP authorization commands share one working registry

Kuru SHALL expose per-alias MCP login, status and logout through one typed command family used by CLI parsing and the TUI command registry. Help, leading-token completion, argument validation and dispatch MUST agree; these operations MUST run without a provider request or tool execution permission and MUST require the same reviewed configured-alias authority before contacting its resource or authorization server. Login MUST offer browser, no-browser and advertised device behavior truthfully; status and logout MUST remain bounded and redact all credential material.

#### Scenario: Login command parity
- **WHEN** a user discovers, completes and runs MCP login for a configured HTTP alias in the CLI or TUI
- **THEN** both surfaces select the same alias-bound connector operation and present the same flow choices without starting a model turn

#### Scenario: Unknown or ineligible alias
- **WHEN** a user requests MCP status, login or logout for an unknown, disabled, STDIO or unapproved automatically configured alias
- **THEN** Kuru refuses before discovery, credential access or process startup with an actionable bounded diagnostic
