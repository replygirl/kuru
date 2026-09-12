# Providers, tools and protocols

Kuru normalizes provider output into text and tool-call proposals. The runtime
owns permission checks, peer routing, memory and tool execution. Providers do
not own the actor pool.

## OpenAI authentication and models

The default `codex` provider uses Kuru-owned ChatGPT authentication and direct
HTTPS requests to the subscription service. It does not launch Codex CLI or
app-server. `kuru login` starts browser OAuth with PKCE and a checked local
callback; `kuru login --no-browser` leaves opening the URL to you, and
`kuru login --device` uses device authorization. `kuru auth` prints redacted
local status as JSON; `kuru logout` clears Kuru's stored credentials.

The private auth store is `auth/openai` beneath Kuru's data directory, outside
the project tool root. Only Kuru's store participates in login and refresh;
another application's tokens are never imported. Session revisions prevent a
late login or refresh from overwriting logout or a newer sign-in. An uncertain
refresh outcome requires sign-in again rather than replaying the token exchange.

Subscription requests use the fixed ChatGPT backend with bearer and account
headers. Kuru reads its model catalog, preserving advertised model IDs and
reasoning efforts. Responses stream over SSE with `store` disabled; a truncated
stream or missing successful completion is an error. Kuru supplies the messages
and tool schemas and keeps all tool execution, peer behavior and memory in its
own runtime. Subscription service compatibility and account access are separate
from the public API-key interface; deterministic protocol fixtures do not prove
live account access.

The `responses` provider uses the [OpenAI Responses API](https://platform.openai.com/docs/api-reference/responses)
and the configured API-key environment variable. It discovers model IDs from
`/models`, submits messages and function tool schemas to `/responses`, and
passes configured reasoning effort. API model catalogs do not necessarily
advertise effort capabilities; an empty effort list means no catalog restriction
was supplied, not that every effort is guaranteed to work. Provider failures use
fixed status and supported-code categories; Kuru does not retain or render remote
error messages, unknown codes, parser excerpts, or endpoint queries.

Responses completions have a 600-second total operation budget while model
catalog requests remain bounded to 60 seconds. SSE keeps at most 2 MiB of its
completed response; ignored deltas and framing use separate finite wire and
parser limits. A truncated, oversized, or failed stream is an error and is not
replayed after partial output.

For provider failures, Kuru reads at most 8 KiB of a failed HTTP body for up to
two seconds, within the existing catalog or completion budget. Known API
statuses and supported provider codes can produce a fixed diagnostic such as a
model-access, quota, rate-limit, or service failure; unknown response details
fall back to a fixed status or stream failure. A transport classification never
authorizes a replay.

`api_base` and `api_key_env` apply to the `responses` provider only. ChatGPT
credentials never go to the configurable API endpoint, and authentication
failures never select a different provider automatically. See
[authentication configuration](configuration.md#authentication) for command and
storage details. Live login and inference checks must be recorded separately
from local fixtures, including checks of the former subprocess adapter.

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
