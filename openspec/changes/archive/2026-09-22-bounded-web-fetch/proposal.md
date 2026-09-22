## Why

Kuru has checked filesystem and shell tools, but it cannot retrieve a bounded public web document through the same reviewed permission surface. Users and peer actors therefore lack a safe, explicit way to obtain current reference material without turning shell authority or provider credentials into an implicit network-fetch mechanism.

A web-fetch tool needs connection-time address enforcement and redirect checks, because a preflight DNS decision cannot safely authorize a later connection. Its returned document is untrusted tool data and must never become instructions or broaden any authority.

## What Changes

- Add a `web_fetch` tool that accepts only validated HTTP(S) destinations and returns bounded textual content with truthful redirect and truncation metadata.
- Enforce destination policy at each actual connection and redirect hop, rejecting loopback, private, link-local, multicast, unspecified, and IPv4-mapped IPv6 addresses unless a future explicit, permission-bound internal-network policy allows them.
- Bound redirect count, total time, compressed input, decompressed output, and model-facing retained content; cancel and redact failures without forwarding provider or MCP credentials across origins.
- Route the tool through Kuru's existing permission evaluation, tool receipts, redaction, documentation, and configuration/schema contracts. Treat every fetched response as untrusted data.

## Capabilities

### New Capabilities
- `web-fetch`: Bounded, connection-validated HTTP(S) retrieval exposed as untrusted tool data.

### Modified Capabilities
- `provider-tools`: Registers and authorizes `web_fetch` through the existing bounded tool host without granting network authority through unrelated tools or workspace trust.

## Impact

Affected areas include `packages/kuru-connectors` HTTP/tool dispatch and permission selectors, `packages/kuru-core` configuration/schema where needed, tool documentation, and deterministic HTTP/DNS/redirect fixture tests. This adds no browser automation, web search, image tooling, provider route changes, MCP credential forwarding, or internal-network exception.

## Surfaces

- [ ] interactive — no new direct TUI control; existing tool receipts may render the result.
- [ ] deploy — no deployment topology, secret, or bind-address change.
- [x] integration — HTTP(S), DNS resolution, redirects, and content decoding are external contracts.
- [x] agent-behavior — adds an untrusted, permission-gated tool and bounded model-facing output.
