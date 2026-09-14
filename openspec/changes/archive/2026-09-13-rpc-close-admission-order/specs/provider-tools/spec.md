## MODIFIED Requirements

### Requirement: Standard protocol adapters

The application SHALL initialize and call MCP tools using stdio or Streamable
HTTP, namespace tool names, and communicate with peers over a documented A2A
JSON-RPC subset. Effective automatic-ancestor MCP transport and external-agent
configuration MUST receive matching workspace approval before discovery,
process creation, connection, advertisement or invocation. An MCP RPC close that
confirms cleanup MUST close command admission before publishing that result, and
later close calls MUST observe the same authoritative confirmed completion.

#### Scenario: Unresponsive protocol peer
- **WHEN** a protocol peer fails to respond
- **THEN** the call fails within its timeout and the harness remains usable.

#### Scenario: Pending configured transport
- **WHEN** automatic ancestor configuration supplies a stdio MCP, HTTP MCP or
  external agent without matching approval
- **THEN** Kuru starts no stdio child, opens no configured connection, and does
  not advertise the external agent.

#### Scenario: Repeated close after confirmed cleanup
- **WHEN** confirmed MCP cleanup publishes its first close result and a caller
  immediately requests close again
- **THEN** command admission is already closed and the later call returns the
  same confirmed completion without waiting for an abandoned command.
