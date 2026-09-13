# MCP servers

Kuru connects to MCP tools over stdio or Streamable HTTP. Configure a named server, then use `kuru tools` to discover its tools alongside the built-ins.

## Configure a transport

For a local process:

```toml
[mcp.local_service]
command = "/absolute/path/to/mcp-server"
args = ["--stdio"]
```

For an HTTP service:

```toml
[mcp.remote_service]
url = "https://example.com/mcp"
```

Replace the command or URL with your server. Each entry must have exactly one transport. Stdio entries can supply an `env` table; keep credentials out of shared configuration. HTTP entries cannot contain process arguments or an environment table.

Aliases namespace tool identifiers. The discovered description retains the original server and tool names. Call the identifier returned by `kuru tools` rather than guessing it.

## Supported operations

Kuru initializes the server, sends `notifications/initialized`, discovers paginated tools, and invokes `tools/call`. The server must advertise tools capability and provide valid input schemas.

The preferred protocol version is `2025-11-25`. The adapter also accepts `2025-06-18`, `2025-03-26`, and `2024-11-05`.

HTTP sessions preserve the negotiated session ID and version headers and close when possible. Each stdio subprocess has an independent native owner which drains stderr and retains the child tree and pipes through bounded cleanup.

Discovery validates each configured alias as a complete unit. If one server cannot start, initialize, or return a valid complete catalog, Kuru reports that alias as unavailable while built-in tools and healthy-server tools remain usable. Previously discovered routes for the unavailable alias dispatch nothing. Running discovery again is the explicit recovery attempt.

## Bounds and permissions

Calls are limited to 60 seconds and 2 MiB. Pagination is bounded; repeated cursors fail. Transport failures and ambiguous cancellation do not automatically retry mutations. An MCP application-error response does not disable an otherwise healthy server.

For direct commands, an unavailable stdio server can include a recent bounded stderr tail on stderr. Kuru replaces its documented finite set of recognizable secret patterns before retention, then escapes terminal controls. This can miss encoded, transformed, new, or deliberately disguised secrets, and ordinary server-authored paths, URLs, or configuration-like text can remain. Runtime events, model input, and memory receive only fixed unavailable metadata, not the captured tail.

Enabling a server grants access to its tools. Its permissions are independent of Kuru's built-in `allow_write` and `allow_shell` switches.

## Current scope

This is a tools client. It does not provide prompts/resources UI, elicitation, sampling, or a remote OAuth login manager. Configure supported server credentials outside shared files. See [configuration](./configuration#mcp-and-external-agents) for layering and server-count limits.
