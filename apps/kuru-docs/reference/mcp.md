# MCP servers

Kuru connects to MCP tools over stdio or Streamable HTTP. Configure a named server, then use `kuru tools` to discover its tools alongside the built-ins.

## Configure a transport

For a local process:

```toml
[mcp.local_service]
command = "/absolute/path/to/mcp-server"
args = ["--stdio"]
allow_tools = ["read_*", "search"]
deny_tools = ["*_secret"]
```

For an HTTP service:

```toml
[mcp.remote_service]
url = "https://example.com/mcp"
header_env = { Authorization = "REMOTE_MCP_AUTH" }
```

For an OAuth-protected HTTP service:

```toml
[mcp.team_service]
url = "https://mcp.example.com/tools"

[mcp.team_service.oauth]
enabled = true
client_id = "kuru-native-client"
scopes = ["mcp.read", "mcp.write"]
```

Replace the command or URL with your server. Each entry must have exactly one transport. Stdio entries can supply an `env` table; keep credentials out of shared configuration. HTTP entries cannot contain process arguments or an `env` table. Their `header_env` table maps header names to environment-variable names. Kuru validates the names and resolved values, limits values to 16 KiB each and 64 KiB per server, rejects protocol-owned headers, and sends resolved headers on initialization, discovery, calls, and session close without putting values in configuration or status output.

Entries default to `enabled = true`. Setting `enabled = false` leaves the alias visible as disabled but performs no header resolution, process start, connection, or cache read. `allow_tools` and `deny_tools` match original server names with `*` and `?`; deny wins, and a nonempty allow list omits unmatched tools before Kuru creates routes or provider-facing names.

OAuth aliases require HTTPS. Configure either `client_id`, or a canonical HTTPS
`client_metadata_url`, or rely on an advertised dynamic-registration endpoint.
Client metadata is preferred when advertised; dynamic registration is a
compatibility fallback and registers a native application. Configured scopes are
a ceiling: the protected resource's Bearer challenge controls the initial scope,
and an `insufficient_scope` challenge can add only its advertised scope to the
prior grant. `Authorization` and `Proxy-Authorization` static header references
cannot be combined with OAuth. Other validated static headers go only to the
protected resource, never to authorization-server endpoints.

Sign in with `kuru mcp login ALIAS`, add `--no-browser` to print the URL without
opening a browser, or select `--device` when the server advertises device
authorization. The no-browser callback is loopback: the browser must run on the
same host as Kuru or forward the printed callback port to that host. Inspect
redacted state with `kuru mcp status ALIAS` and sign out with
`kuru mcp logout ALIAS`. The TUI provides the same `/mcp login`, `/mcp status`,
and `/mcp logout` family.

Aliases namespace tool identifiers. The discovered description retains the original server and tool names. Call the identifier returned by `kuru tools` rather than guessing it.

## Supported operations

Kuru initializes the server, sends `notifications/initialized`, discovers paginated tools, and invokes `tools/call`. The server must advertise tools capability and provide valid input schemas.

The preferred protocol version is `2025-11-25`. The adapter also accepts `2025-06-18`, `2025-03-26`, and `2024-11-05`.

HTTP sessions preserve the negotiated session ID and version headers and close when possible. Each stdio subprocess has an independent native owner which drains stderr and retains the child tree and pipes through bounded cleanup.

HTTP authorization follows the [MCP 2026-07-28 authorization
contract](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
over Kuru's existing negotiated MCP transport. This does not claim full support
for the 2026 stateless wire protocol. Kuru validates protected-resource and
authorization-server discovery, exact resource binding, PKCE S256, state, and
the [RFC 9207](https://www.rfc-editor.org/rfc/rfc9207) issuer parameter before
redeeming a code. Advertised [RFC 8628](https://www.rfc-editor.org/rfc/rfc8628)
device flow honors server pacing, expiry, denial, and cancellation.

Credentials live only in Kuru's native operating-system secret store. Kuru does
not read another harness's credentials and has no plaintext or session-only
fallback. Refresh rotates the native generation before a bearer token is used;
an ambiguous rotating refresh is not replayed. An invalid-token discovery
request can refresh once and repeat only discovery. Kuru never replays a tool
call after an authorization or transport failure.

Discovery validates each configured alias as a complete unit. If one server cannot start, initialize, or return a valid complete catalog, Kuru reports that alias as degraded while built-in tools and healthy-server tools remain usable. A valid, context-bound private cache may preserve its filtered metadata as explicitly stale, but stale metadata creates no executable route and grants no permission. Previously discovered routes for the unavailable alias dispatch nothing. Running discovery again is the explicit recovery attempt, and a healthy discovery replaces corrupt cache state.

## Bounds and permissions

Calls are limited to 60 seconds and 2 MiB. Pagination is bounded; repeated cursors fail. Transport failures and ambiguous cancellation do not automatically retry mutations. An MCP application-error response does not disable an otherwise healthy server.

For direct commands, an unavailable stdio server can include a recent bounded stderr tail on stderr. Kuru replaces its documented finite set of recognizable secret patterns before retention, then escapes terminal controls. This can miss encoded, transformed, new, or deliberately disguised secrets, and ordinary server-authored paths, URLs, or configuration-like text can remain. Runtime events, model input, and memory receive only fixed unavailable metadata, not the captured tail.

Enabling a server grants access to its tools. Its own permissions are independent of Kuru's built-in `allow_write` and `allow_shell` switches, though a [`[[permissions]]` rule](./configuration#tool-permissions) can additionally `allow`, `ask`, or `deny` a specific `{ kind = "mcp", alias, tool }` selector — MCP tool arguments have no pattern matching.

`kuru tools` and `/tools` report the same filtered metadata and one state per alias: disabled, live, stale, or degraded. Neither command makes a provider request.

## Current scope

This is a tools client. It does not provide prompts/resources UI, elicitation,
sampling, hot reload, or periodic background refresh. MCP OAuth credentials are
separate from ChatGPT browser credentials and `OPENAI_API_KEY`. Logout always
deletes the selected local MCP credential. It reports advertised remote
revocation separately as revoked, refused, unavailable, or uncertain; local
deletion does not prove that the remote grant was revoked. See
[configuration](./configuration#mcp-and-external-agents) for layering and
server-count limits.
