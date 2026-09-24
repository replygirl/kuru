## ADDED Requirements

### Requirement: MCP tool inspection is a working registry entry

The shared TUI command registry SHALL register `/tools` only with its working MCP catalog handler. Help, completion, parsing, and dispatch MUST agree, and the handler MUST project the same filtered tools and per-alias disabled, live, stale, or degraded state as `kuru tools` without making a provider request or treating cached metadata as availability.

#### Scenario: Inspect tools in the terminal
- **WHEN** the user invokes `/tools` with live, stale, degraded, and disabled configured aliases
- **THEN** the registered handler renders their bounded filtered catalog and truthful status without starting a turn or exposing resolved header values
