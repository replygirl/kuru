## ADDED Requirements

### Requirement: MCP OAuth and native credential reference

Curated configuration, tools and protocol documentation SHALL explain the MCP 2026-07-28 HTTP authorization contract as applied to Kuru's existing negotiated transport: OAuth discovery, configured/CIMD/deprecated-compatibility-DCR registration order, DCR native application type, PKCE, RFC 9207 issuer validation, authoritative initial and step-up scope selection, resource binding, browser and advertised device login, same-host or forwarded loopback requirements for no-browser use, refresh behavior, native credential-store prerequisites, per-alias status and logout. It MUST avoid claiming full MCP 2026 wire-protocol support; state that Kuru stores only its own MCP credentials, has no plaintext or session-only fallback, keeps OpenAI and static-authorization routes separate, limits non-authorization static headers to protected resource requests, does not replay ambiguous tool or rotating-refresh requests, and deletes local credentials while reporting advertised remote revocation separately and without promising it occurred when absent or failed.

#### Scenario: User prepares a headless MCP login
- **WHEN** a user reads the MCP authorization guide from a host without a usable local browser
- **THEN** they can determine whether loopback forwarding is required, how to select an advertised device flow, what native credential facility is required and how cancellation affects stored state

#### Scenario: User interprets logout
- **WHEN** logout reports local deletion together with no advertised revocation endpoint or a remote refusal
- **THEN** the documentation explains that Kuru's credential is gone without claiming the remote authorization grant was revoked
