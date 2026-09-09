# provider-tools Specification

## Purpose
Connect peer actors to supported inference and authentication transports,
workspace tools, MCP servers and A2A peers through extensible interfaces with
explicit authority, bounded execution and protected private state.

## Requirements

### Requirement: Supported OpenAI transport

The application SHALL support Codex-owned authentication through supported commands and app-server transport, and Responses API authentication through a configurable environment variable, with dynamically discovered models and available effort settings.

#### Scenario: Future effort setting
- **WHEN** the provider advertises an unfamiliar effort value
- **THEN** the application preserves it and can send it without requiring a code change.

### Requirement: Provider-neutral inference boundary

The application SHALL own peer context and tool execution independently of the provider; Codex transport MUST disable built-in execution capabilities for inference requests.

#### Scenario: Tool proposal
- **WHEN** a provider returns a tool request
- **THEN** the runtime validates and executes it through Kuru's configured tool host.

### Requirement: Explicit bounded tools

Filesystem tools MUST enforce canonical workspace containment and protect instruction/configuration paths. Mutations and shell execution MUST require explicit opt-ins. Shell execution MUST have time and output bounds and SHALL be described as process authority rather than a filesystem sandbox.

#### Scenario: Symlink escape
- **WHEN** a filesystem call follows a workspace symlink outside the root
- **THEN** the operation fails without modifying the outside file.

### Requirement: Standard protocol adapters

The application SHALL initialize and call MCP tools using stdio or Streamable HTTP, namespace tool names, and communicate with peers over a documented A2A JSON-RPC subset.

#### Scenario: Unresponsive protocol peer
- **WHEN** a protocol peer fails to respond
- **THEN** the call fails within its timeout and the harness remains usable.
