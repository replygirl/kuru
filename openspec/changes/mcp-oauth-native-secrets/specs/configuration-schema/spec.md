## ADDED Requirements

### Requirement: MCP OAuth configuration has strict parser-schema parity

The native parser and published configuration schema SHALL define the same bounded optional OAuth table for HTTP MCP aliases, including explicit enablement, configured client identity, optional environment reference for a preregistered client secret, optional HTTPS Client ID Metadata Document identity and a bounded scope allowlist. A nonempty allowlist MUST constrain rather than override authoritative challenge scopes and MUST restrict protected-resource metadata fallback and later step-up unions; the default empty list adds no configuration ceiling. Both paths MUST reject unknown fields, literal client secrets or tokens, invalid or duplicate scopes, non-HTTPS metadata identities, mutually incompatible registration choices, OAuth on STDIO, and OAuth combined with static `Authorization` or `Proxy-Authorization` header references. Other validated static headers MAY remain configured for protected MCP resource requests but MUST NOT be forwarded to OAuth discovery, registration, authorization, device, token or revocation endpoints. OAuth configuration and its final-leaf provenance SHALL participate in the alias's reviewed workspace authority and catalog context without resolving a secret before trust preflight; a disabled alias MAY retain valid OAuth configuration but MUST NOT resolve its secret reference, discover authority or access the native store.

#### Scenario: Configured public client is valid
- **WHEN** an enabled HTTP MCP alias declares a bounded preregistered public client and OAuth scopes with no literal secret
- **THEN** the native parser and published schema accept the same shape and the authority projection identifies the registration fields without credential values

#### Scenario: Secret or incompatible flow is configured
- **WHEN** an alias contains a literal token/client secret, combines incompatible client-registration choices, requests OAuth on STDIO, combines OAuth with a static authorization header, or uses an unknown or over-limit field
- **THEN** parsing or semantic validation fails before connector construction and does not echo a supplied secret value

#### Scenario: OAuth alias retains a non-authorization resource header
- **WHEN** an HTTP OAuth alias maps a validated non-authorization header to a trusted environment reference
- **THEN** the parser and schema accept it for protected MCP resource requests while the connector withholds it from every OAuth authority endpoint
