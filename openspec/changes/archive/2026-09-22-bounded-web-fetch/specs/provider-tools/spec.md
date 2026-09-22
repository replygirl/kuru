## MODIFIED Requirements

### Requirement: Provider-neutral inference boundary

The application SHALL own peer context and tool execution independently of the provider. Native OpenAI requests MUST use Kuru's existing supplied context and function tools without executing another harness or importing its agent policy. A built-in web fetch MUST remain a Kuru-owned, permission-gated tool and MUST NOT reuse provider or MCP authentication, credentials, request headers, or configured endpoint authority for its own HTTP(S) requests.

#### Scenario: Tool proposal
- **WHEN** a provider returns a tool request
- **THEN** the runtime validates and executes it through Kuru's configured tool host.

#### Scenario: Web fetch has distinct transport authority
- **WHEN** a provider proposes `web_fetch` while provider or MCP credentials and configured headers are present
- **THEN** the tool host applies its own permission and destination policy and does not forward those credentials or headers to the fetched origin.
