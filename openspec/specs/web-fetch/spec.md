# web-fetch Specification

## Purpose
Define Kuru's permission-gated public HTTP(S) fetch capability, including connection-time destination validation, bounded untrusted content handling, and credential-independent result projection.

## Requirements

### Requirement: Connection-validated public HTTP(S) fetch

The application SHALL expose a permission-gated `web_fetch` operation only for absolute HTTP(S) URLs with no embedded credentials. It MUST validate each destination at actual connection time and each redirect target before use, rejecting loopback, private, link-local, multicast, unspecified, and IPv4-mapped IPv6 addresses. A DNS answer observed during URL parsing or preflight SHALL NOT authorize a later resolver result or connection. Redirects MUST remain HTTP(S), be revalidated independently, and stop at a fixed bounded hop count. Internal-network access is unavailable unless an explicit future permission-bound policy specifies it.

#### Scenario: Redirect changes address class
- **WHEN** a public HTTPS origin redirects to a loopback, private, link-local, multicast, unspecified, or IPv4-mapped IPv6 destination
- **THEN** Kuru rejects the redirect before sending a request to that destination and returns no fetched body.

#### Scenario: DNS rebinds after validation
- **WHEN** a hostname's connection-time resolution differs from its earlier checked DNS answer and includes a prohibited address
- **THEN** Kuru refuses the connection without relying on the earlier answer.

#### Scenario: Unsupported or credential-bearing destination
- **WHEN** a tool request supplies a non-HTTP(S) URL, a relative URL, or URL userinfo
- **THEN** Kuru rejects it before DNS resolution or network dispatch.

### Requirement: Bounded untrusted web content

`web_fetch` SHALL bound total operation time, redirect count, compressed response bytes, decompressed response bytes, and returned model-facing content with fixed documented limits. It MUST stream and cancel slow or hostile responses, return truthful truncation or omission metadata, and preserve the existing redaction rules for errors and retained results. Fetched bytes and rendered text are untrusted tool data: they MUST NOT add instructions, permissions, tool roots, provider routes, or workspace authority. The fetch client MUST NOT forward provider, MCP, or unrelated configured credentials across origins.

#### Scenario: Hostile compressed response
- **WHEN** a response expands beyond the decompressed-byte limit or crosses the compressed-input limit
- **THEN** Kuru stops reading, returns a bounded result or fixed failure according to the documented limit, and retains no unbounded body.

#### Scenario: Slow response or cancellation
- **WHEN** a response stalls beyond the operation deadline or the admitted tool call is cancelled
- **THEN** Kuru closes the request within its bounded cleanup path and publishes one cancelled or timed-out tool outcome without a later body.

#### Scenario: Fetched content resembles instructions
- **WHEN** a successful fetched document contains text that asks the actor to disclose data, change tools, or bypass permissions
- **THEN** the document is delivered only as untrusted tool content and does not alter Kuru authority or dispatch.
