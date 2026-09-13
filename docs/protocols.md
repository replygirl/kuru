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

The subscription route is a fixed compatibility dependency: client
`app_EMoamEEZ73f0CkXaXp7hrann`, issuer `https://auth.openai.com`, backend
`https://chatgpt.com/backend-api/codex`, and catalog `client_version=0.154.0`.
Its browser/token and device paths are fixed connector literals. Kuru's
loopback auth fixtures and subscription fixtures independently assert those
wire values; update the literal and the applicable fixture together. This is
not a public OpenAI support promise for Kuru, and it cannot detect an upstream
service change. `kuru logout` removes only Kuru's local credentials; no remote
revocation endpoint is established by this contract.

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

Within that same deadline, Kuru may retry an explicitly rejected provider
response only for HTTP 429, 500, or 503, with at most three provider sends and
two delays. One operation makes at most four application HTTP sends in total,
including at most one subscription credential rotation. Fallback delays use
equal jitter in the ranges 250–500 milliseconds and then 500–1000 milliseconds. One
`Retry-After` delta-seconds or canonical IMF-fixdate value may raise the delay,
but never past 30 seconds for one delay, 60 seconds in aggregate, or the
operation deadline. A valid value that does not fit those bounds prevents a
retry instead of being clamped. API quota and billing responses remain terminal.

For provider failures, Kuru reads at most 8 KiB of a failed HTTP body for up to
two seconds, within the existing catalog or completion budget. Known API
statuses and supported provider codes can produce a fixed diagnostic such as a
model-access, quota, rate-limit, or service failure; unknown response details
fall back to a fixed status or stream failure. After a response or stream is
accepted, malformed, partial, idle, canceled, disconnected, and decoding
outcomes are terminal and are never replayed. Ambiguous transport failures are
also never replayed.

Credential refresh remains under Kuru's private credential lease. A refresh
POST may repeat once only when the pinned native HTTP/1 transport reports its
audited connection-acquisition failure before the token request was dispatched;
a timeout can be part of that audited class, but timeout status alone does not
establish it. Proxy CONNECT, response, or other send failure also does not.
Possibly dispatched refreshes are reconciled against the exact durable
credential generation or pending record rather than replayed. Kuru does not
claim that the remote provider implements request idempotency.

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
1–120000 milliseconds. Stdout and stderr are each bounded to 2 MiB. On Unix,
a registered owner retains the standard root, its fresh process group,
both pipes, and the checked workspace capability through cleanup. It signals the
remaining original group before reaping that root, then confirms group absence
before reporting normal completion. A timeout, cancellation, pipe/output
failure, or shutdown keeps its primary error; if bounded confirmation remains
unavailable, Kuru reports that state while the owner remains retained for later
observation. Selected execution/startup timeouts may be followed by one
five-second cleanup-confirmation allowance; a delayed startup caller returns at
that deadline plus the allowance, and host shutdown shares one five-second
window across registered shells. This does not control processes that leave the
original group.
Windows retains its existing Job-based cleanup.

The shell uses your process authority, not a sandbox. Configured MCP servers
also bring their own permissions; the built-in file/shell switches do not impose
a sandbox on third-party tools.

The built-in shell receives a finite compatibility subset of inherited
environment variables for command discovery, home/profile, temporary paths,
locale, time and standard XDG locations. It does not inherit provider or
authentication variables, proxy configuration, agent sockets, arbitrary
`KURU_*` values, or shell-startup controls. Windows additionally derives its
stock system-shell paths and omits inherited `PSModulePath`, allowing stock
PowerShell to reconstruct its standard module paths. This reduces accidental
variable disclosure; it does not contain the shell's filesystem, process, or
network authority. Configured stdio MCP servers keep their own inherited
environment and explicit configuration overrides.

Before a built-in file, shell, or MCP result crosses into the CLI or runtime,
Kuru projects a finite set of recognizable credential forms to
`[REDACTED:recognized-secret]`. It recognizes Basic/Bearer Authorization and
Proxy-Authorization values; OpenAI `sk-svcacct-`, `sk-proj-`, then `sk-`
prefixes, longest first, with at least 16 token bytes after that prefix;
GitHub (`ghp_`, `github_pat_`, `gho_`, `ghu_`, `ghs_`, `ghr_`)
forms with at least 8 following token bytes, and exact-length AWS
(`AKIA`/`ASIA` plus exactly 16 uppercase-alphanumeric bytes) token shapes;
supported PEM private-key blocks; and exact
contextual sensitive field names such as `api_key`, `password`, and
`refresh_token`. Those local OpenAI and GitHub floors reduce accidental matches;
they do not validate a credential. Exact sensitive JSON fields have their whole
value replaced, including non-string values, while other JSON strings and plain
text retain unmatched bytes. AWS matching uses its uppercase-alphanumeric token
alphabet for the trailing boundary, so a following lowercase byte is outside that
token shape.

This projects returned results, actionable MCP application errors and outward
tool-failure details, including error formatting and source chains. It
does not inspect credential stores or enumerate environment values, alter tool
arguments or file writes, rewrite source files or earlier history, or impose a
new combined result limit. Unknown, encoded, split, transformed, or future
credential formats can remain visible, and ordinary source text can be matched
by a documented form; tool results are not byte-exact backups. MCP stderr is
not captured by this projection.

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
