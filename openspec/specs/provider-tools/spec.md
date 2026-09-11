# provider-tools Specification

## Purpose
Connect peer actors to supported inference and authentication transports,
workspace tools, MCP servers and A2A peers through extensible interfaces with
explicit authority, bounded execution and protected private state.

## Requirements

### Requirement: Supported OpenAI transport

The application SHALL support native ChatGPT OAuth authentication and direct
OpenAI subscription inference, and Responses API authentication through a
configurable environment variable, with dynamically discovered models and
available effort settings. Login, refresh, status and logout MUST manage only
Kuru's checked private credentials. The application MUST NOT require or embed
the Codex CLI/app-server or switch billing routes after authentication failure.

#### Scenario: Future effort setting
- **WHEN** the provider advertises an unfamiliar effort value
- **THEN** the application preserves it and can send it without requiring a code change.

#### Scenario: Standalone authentication
- **WHEN** Kuru is installed without another agent harness
- **THEN** native browser/device login, credential status and direct authenticated requests work without that harness.

#### Scenario: Invalid callback or cancelled login
- **WHEN** a callback has an invalid state or a pending login is cancelled
- **THEN** Kuru preserves its previous credentials and releases the owned callback resources.

#### Scenario: Concurrent refresh and logout
- **WHEN** requests race token rotation or logout
- **THEN** validated rotation is reused without overwriting a newer login or resurrecting a logged-out session.

### Requirement: Provider-neutral inference boundary

The application SHALL own peer context and tool execution independently of the
provider. Native OpenAI requests MUST use Kuru's existing supplied context and
function tools without executing another harness or importing its agent policy.

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
