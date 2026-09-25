## Why

Kuru can connect to remote MCP servers with configured static headers, but it cannot complete the MCP OAuth authorization flow or retain refreshable credentials in an operating-system credential store. Users therefore cannot authorize servers that require OAuth without manually supplying a static credential, and Kuru cannot safely refresh, relogin, revoke, or report that authorization state.

The current stable MCP 2026-07-28 authorization specification defines discovery, client registration, resource binding, PKCE, issuer validation, scope challenges, and bearer-token use for HTTP transports. Kuru needs to apply that authorization contract behind the connector boundary while keeping the existing negotiated MCP transport revision, every alias isolated, OpenAI authentication separate, and OAuth material out of plaintext storage. This change does not claim full MCP 2026-07-28 wire-protocol support.

## What Changes

- Add MCP OAuth 2.1 authorization for HTTP aliases: protected-resource and authorization-server discovery, configured/CIMD/DCR client registration, PKCE S256, state and RFC 9207 issuer validation, resource indicators, bounded authoritative scope selection, refresh, step-up/relogin, cancellation, and alias-local failure. DCR remains the advertised compatibility fallback and registers Kuru as a native application.
- Add an optional RFC 8628 device flow only when the discovered authorization server advertises it. Browser authorization uses an owned loopback callback; headless output states whether that callback is reachable and uses device authorization when advertised rather than implying that printing a loopback URL makes a remote callback work.
- Add a native secret-store boundary for Kuru-owned MCP tokens and registered-client secrets. There is no plaintext or session-only fallback; an unavailable native store makes OAuth unavailable with an actionable diagnostic while static-header MCP remains usable.
- Bind every stored credential and outbound bearer header to the exact configured alias, canonical MCP resource, authorization-server issuer, client identity and granted scopes. A changed endpoint, issuer, client registration or workspace authority cannot reuse the old token.
- Add CLI and shared TUI command-registry surfaces for per-alias login, status, and logout/revoke. Logout always deletes Kuru's local credential after bounded ownership is acquired and accurately reports whether an advertised remote revocation endpoint succeeded, refused, was unavailable, or was not advertised.
- Preserve the P12 disabled/live/stale/degraded catalog contract: OAuth affects only the selected HTTP alias, never grants tool execution, never makes stale metadata callable, and never changes a separately configured static-header alias. An OAuth alias may retain validated non-authorization static headers for protected MCP resource requests, but cannot configure `Authorization` or `Proxy-Authorization` through `header_env` and never forwards any static header to OAuth authority endpoints.

## Capabilities

### New Capabilities

- `mcp-oauth`: MCP OAuth discovery, authorization, refresh, token binding, cancellation, status and revoke behavior.
- `native-secret-storage`: OS-backed storage and lifecycle rules for Kuru-owned MCP credentials.

### Modified Capabilities

- `configuration-schema`: add bounded per-alias OAuth client and flow configuration with parser/schema parity and authority provenance.
- `mcp-catalog-controls`: project OAuth authorization state through the existing alias-local catalog without turning credentials or cached metadata into routes or permission grants.
- `command-registry`: add one canonical login/status/logout command family shared by CLI parsing and TUI help/completion/dispatch.
- `public-documentation`: document supported OAuth flows, native-store requirements, headless behavior, static-header separation, status and revocation limits.

## Impact

The change affects MCP configuration in `kuru-core`, the HTTP MCP client and a new connector-owned OAuth manager in `kuru-connectors`, native credential-store APIs in `kuru-platform`, and CLI/TUI command presentation in `kuru-tui`. It adds exact pinned Rust dependencies only where the standard library and current workspace dependencies cannot provide OAuth metadata parsing, secure random generation, or supported operating-system credential-store access; all affected lockfiles change together.

No Dolt schema or MCP JSON-RPC transport revision changes. Existing OpenAI `login`/`logout`/`auth` commands, stores and provider routes remain separate. Existing static-header configurations remain valid.

## Surfaces

- [x] interactive — CLI and TUI login, status, device/headless guidance, cancellation and logout/revoke output
- [x] deploy — loopback callback lifetime and native credential-store availability on supported operating systems
- [x] integration — MCP OAuth 2.1, OAuth/OIDC discovery, CIMD/DCR, RFC 8628 device authorization, RFC 8707 resource binding and RFC 7009 revocation
- [ ] agent-behavior — provider prompts, tool selection and runtime output shapes are unchanged
