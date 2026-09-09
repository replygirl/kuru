# Providers, tools and protocols

Kuru normalizes provider output into text and tool-call proposals. The runtime
owns permission checks, peer routing, memory and tool execution. Providers do
not own the actor pool.

## OpenAI authentication and models

The `codex` provider uses Codex 0.153.4 (the current stable release verified on
2026-09-09), pinned by mise for development, as an app-server over newline-delimited
JSON-RPC. `kuru login` delegates to `codex login`; `kuru login --device` delegates
to device authorization; `kuru auth` reports status; `kuru logout` delegates to
logout. Kuru never copies OAuth tokens or implements a private login endpoint.
Follow the supported [OpenAI authentication documentation](https://learn.chatgpt.com/docs/auth?surface=app)
and [Codex authentication documentation](https://developers.openai.com/codex/auth).

Model discovery calls app-server `model/list`, preserving advertised effort
values. Each completion uses an ephemeral isolated working directory and a
read-only inference thread. Built-in shell, worker spawning, apps, plugins,
memories and other execution features are disabled in the transport config;
Kuru's completion schema carries proposed tool calls for the runtime to execute.
New Codex versions can change this contract: keep the adapter and its protocol
tests in step with supported versions, and report live-smoke evidence separately
from deterministic mock tests.

The 2026-09-09 live smoke used Codex 0.153.4 with existing supported
authentication, discovered eight catalog models (including hidden review/reserve
entries), preserved advertised `max` and `ultra` effort values, and received an
`OK` completion from `gpt-6-astra` at `low` effort through the isolated structured
output transport. This verifies that concrete transport path; catalog access
remains account-specific. Reproduce with `mise exec -- cargo run --locked -p
kuru-connectors --example codex_probe -- --infer` when authenticated.

The `responses` provider uses the [OpenAI Responses API](https://platform.openai.com/docs/api-reference/responses)
and the configured API-key environment variable. It discovers model IDs from
`/models`, submits messages and function tool schemas to `/responses`, and
passes configured reasoning effort. API model catalogs do not necessarily
advertise effort capabilities; an empty effort list means no catalog restriction
was supplied, not that every effort is guaranteed to work. The provider's error
remains authoritative for unsupported model/effort combinations.

The `demo` provider is deterministic and requires no network or credentials.
It is useful for installation, rendering and pool tests. A successful demo
conversation is not evidence of live OpenAI authentication or inference.

## Built-in tools

| Tool | Inputs | Authority |
| --- | --- | --- |
| `file_read` | `path` | Workspace read |
| `file_list` | `path` | Workspace listing |
| `file_write` | `path`, `content` | Requires `allow_write` |
| `file_delete` | `path` | Requires `allow_write` |
| `shell` | `command` | Requires `allow_shell` |

Paths are relative to the opened project capability. Absolute paths, parent
traversal and symlinks are rejected; capability-relative filesystem operations
provide containment beyond string-prefix checks. Sensitive directories such as
`.git`, `.kuru`, `.codex`, `.agents`, `.claude`, `.ssh` and credential/config
files are protected. Instruction files may be read but are protected from
mutation. Files and protocol/output payloads have 2 MiB bounds. Shell execution
defaults to a 30-second timeout; an optional `timeout_ms` argument accepts
1–120000 milliseconds. Stdout and stderr are each bounded to 2 MiB. Unix
process groups are terminated on timeout or cancellation.

The shell uses your process authority, not a sandbox. Configured MCP servers
also bring their own permissions; the built-in file/shell switches do not impose
a sandbox on third-party tools.

## MCP

Both stdio and Streamable HTTP transports initialize the server, send
`notifications/initialized`, discover paginated tools, and call `tools/call`.
The preferred protocol version is `2025-11-25`; the adapter also accepts
`2025-06-18`, `2025-03-26` and `2024-11-05`. HTTP sessions preserve the negotiated
session ID and version headers and are closed when possible. Stdio subprocesses
are terminated during shutdown.

Each tool receives a stable namespaced identifier derived from the configured
server alias and original tool name. Its description retains both names. This
prevents collisions with built-ins and between servers. A server must advertise
tools capability and valid input schemas. Calls are bounded to 60 seconds and
2 MiB; pagination is bounded and repeated cursors are rejected. Tool mutations
are not retried automatically after transport failure.

The adapter is a tools client. It does not implement every optional MCP surface,
such as prompts/resources UI, elicitation, sampling, or a remote OAuth login
manager. Configure supported server credentials outside shared files.

## A2A

Kuru supports a bounded nonstreaming A2A 1.0 JSON-RPC `SendMessage` flow. Messages
use `messageId`, `contextId`, `ROLE_USER` and text parts. Responses contain either
a message or a task; terminal task text is extracted from status messages and
artifacts. The adapter sends `A2A-Version: 1.0`. Protocol errors, malformed
responses, response-size excess and timeouts become observable call errors.

Internal actors use typed peer envelopes and Tokio mailboxes. They do not issue
loopback HTTP calls to each other. A2A at the external boundary preserves peer
message intent and provides a future federation path without imposing a tree of
owners on the pool. Streaming, push notification delivery, remote task polling
and automatic agent discovery are outside the initial subset.

Inbound service is optional:

```sh
# Supply KURU_A2A_TOKEN through your secret/environment manager.
kuru --provider demo serve
```

The default listener is `127.0.0.1:7437`. V1 requires a loopback bind and a bearer
token of at least 16 characters; `--token-env` selects the environment variable.
An authenticated gateway is needed for remote exposure. Outbound endpoints are
explicitly configured under `external_agents`; v1 does not include an outbound
per-agent credential configuration, so such endpoints must accept the supported
transport or sit behind an appropriate local gateway.
