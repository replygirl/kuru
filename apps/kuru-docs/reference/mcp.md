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

HTTP sessions preserve the negotiated session ID and version headers and close when possible. Stdio subprocesses terminate during shutdown.

## Bounds and permissions

Calls are limited to 60 seconds and 2 MiB. Pagination is bounded; repeated cursors fail. Transport failures do not automatically retry mutations.

Enabling a server grants access to its tools. Its permissions are independent of Kuru's built-in `allow_write` and `allow_shell` switches.

## Current scope

This is a tools client. It does not provide prompts/resources UI, elicitation, sampling, or a remote OAuth login manager. Configure supported server credentials outside shared files. See [configuration](./configuration#mcp-and-external-agents) for layering and server-count limits.
