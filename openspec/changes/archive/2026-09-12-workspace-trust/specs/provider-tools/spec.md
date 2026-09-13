## MODIFIED Requirements

### Requirement: Supported OpenAI transport

The application SHALL support native ChatGPT OAuth authentication and direct
OpenAI subscription inference, and Responses API authentication through a
configurable environment variable, with dynamically discovered models and
available effort settings. Login, refresh, status and logout MUST manage only
Kuru's checked private credentials. The application MUST NOT require or embed
the Codex CLI/app-server or switch billing routes after authentication failure.
An automatic-ancestor Responses route MUST receive matching workspace approval
before Kuru reads its named environment variable or contacts its API base;
fixed ChatGPT login and logout MUST NOT read an unrelated workspace-selected
API-key environment variable.

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

#### Scenario: Pending workspace API route
- **WHEN** an unapproved automatic ancestor selects a Responses credential route
- **THEN** login and logout still use their fixed ChatGPT route, while an
  active Responses status check requires approval; none reads the selected
  API-key environment value before its applicable preflight permits it.

### Requirement: Explicit bounded tools

Filesystem tools MUST enforce canonical workspace containment and protect
instruction/configuration paths. Mutations and shell execution MUST require
explicit opt-ins and matching workspace approval when automatic ancestor
configuration contributes their effective grant. Shell execution MUST have time
and output bounds and SHALL be described as process authority rather than a
filesystem sandbox; workspace approval does not widen tool roots or make private
same-user state inaccessible to a shell.

#### Scenario: Symlink escape
- **WHEN** a filesystem call follows a workspace symlink outside the root
- **THEN** the operation fails without modifying the outside file.

#### Scenario: Pending shell or write grant
- **WHEN** an automatic ancestor enables shell or writes without matching approval
- **THEN** the tool host neither exposes nor executes that authority.

### Requirement: Standard protocol adapters

The application SHALL initialize and call MCP tools using stdio or Streamable
HTTP, namespace tool names, and communicate with peers over a documented A2A
JSON-RPC subset. Effective automatic-ancestor MCP transport and external-agent
configuration MUST receive matching workspace approval before discovery,
process creation, connection, advertisement or invocation.

#### Scenario: Unresponsive protocol peer
- **WHEN** a protocol peer fails to respond
- **THEN** the call fails within its timeout and the harness remains usable.

#### Scenario: Pending configured transport
- **WHEN** automatic ancestor configuration supplies a stdio MCP, HTTP MCP or
  external agent without matching approval
- **THEN** Kuru starts no stdio child, opens no configured connection, and does
  not advertise the external agent.
