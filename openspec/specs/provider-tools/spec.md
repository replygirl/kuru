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

### Requirement: Provider-neutral inference boundary

The application SHALL own peer context and tool execution independently of the
provider. Native OpenAI requests MUST use Kuru's existing supplied context and
function tools without executing another harness or importing its agent policy.

#### Scenario: Tool proposal
- **WHEN** a provider returns a tool request
- **THEN** the runtime validates and executes it through Kuru's configured tool host.

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

### Requirement: Bounded provider failure diagnostics

Responses completion and model-catalog requests, Responses HTTP-success error
envelopes, and native ChatGPT failed stream events SHALL produce one finite,
Kuru-owned diagnostic class. The application MUST render only fixed Kuru text
and an HTTP status where present; it MUST NOT render provider message, parameter,
header, unknown code/type, endpoint URL/query, or other remote diagnostic text.

Provider failed-body inspection MUST be limited to 8 KiB and a two-second total
reader deadline within the existing operation deadline. A malformed, oversized,
stalled, or dribbling body MUST retain its fixed status-derived diagnostic. The
application MUST sanitize every error-chain element exposed to callers under
both Display and Debug formatting, including unexpected HTTP-success status
strings, and MUST
preserve the existing single rejected-401 rotation and no-replay-after-partial
completion rules.

#### Scenario: Known API model error is redacted
- **WHEN** a Responses completion receives HTTP 400 with `error.code` equal to
  `model_not_found` and a body message containing a fake secret
- **THEN** it reports the fixed selected-model availability/access diagnostic
  with HTTP 400 and no error-chain element contains the secret or model text

#### Scenario: HTTP-success error envelope is redacted
- **WHEN** a Responses completion returns HTTP 200 with a non-null error object
  whose message contains a fake secret
- **THEN** it reports a fixed provider protocol failure and no error-chain
  element contains the message

#### Scenario: Known native failed event is classified
- **WHEN** a successful ChatGPT SSE connection emits a terminal
  `response.failed` event with the observed `server_is_overloaded` code
- **THEN** it reports the fixed overloaded-service diagnostic without accepting
  or replaying the completion

#### Scenario: Unknown native failed event remains bounded
- **WHEN** a successful ChatGPT SSE connection emits a terminal failed event
  with an unrecognized code or type and a fake-secret message
- **THEN** it reports the fixed stream/protocol failure without rendering the
  code, type, message, or secret and without replaying the completion

#### Scenario: Dribbling failed body times out within the operation budget
- **WHEN** a non-success provider response dribbles its diagnostic body beyond
  the two-second diagnostic-reader deadline
- **THEN** it returns the fixed status-derived diagnostic before the containing
  catalog or completion deadline expires
