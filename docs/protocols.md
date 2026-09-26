# Providers, tools and protocols

Kuru normalizes provider output into ordered content blocks, including text and
tool-call proposals. Typed tool results retain their call IDs and JSON values;
the native connector sends matching results in the pending request's call
order. The runtime owns permission checks, peer routing, memory and tool
execution. Providers do not own the actor pool.

Native continuation state, including encrypted reasoning, stays private to the
actor and is not reconstructed from stored conversation text. Legacy string
receipts are interpreted only at the existing live pending-call boundary with
matching IDs. An old message that happens to contain JSON remains text, and an
unsupported content block fails before provider dispatch rather than being
silently discarded. The CLI returns a settled text answer; the TUI can display a
provisional preview of the selected speaker's stream. Image input remains
unsupported.

`CompletionRequest.current_message_count` identifies the retained suffix from
the current actor invocation. Native continuation matches only that suffix,
preserving earlier receipts as history even when a bounded projection omits an
assistant record. Current peer messages may appear between receipts and remain
in the native request. Direct callers that omit the boundary get a fresh request;
the connector does not infer a native continuation from historical content.

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
passes configured reasoning effort and requests SSE streaming. API model catalogs do not necessarily
advertise effort capabilities; an empty effort list means no catalog restriction
was supplied, not that every effort is guaranteed to work. Provider failures use
fixed status and supported-code categories; Kuru does not retain or render remote
error messages, unknown codes, parser excerpts, or endpoint queries.

Providers implement `Provider::stream`; `complete` collects the same stream.
Normalized observations include text and refusals, explicitly visible reasoning
summaries, tool-argument fragments, usage and terminal outcomes. Cached-input
and reasoning-output token counts retain the distinction between missing and
explicitly zero values. These counts are components of the reported totals,
not additional tokens.

Only a validated completed response authorizes a tool call or a durable
assistant message. Native item IDs reconcile fragments and completed items with
the native terminal response before it is converted into content blocks; an
item ID is not a tool call ID, and a stream's output index never identifies a
terminal item. A non-empty terminal output is authoritative for ordering and
presence: reasoning or tool-call items it reorders, replaces or omits are a
legitimate completion, while text already shown to the user must still be
present under its item ID and still agree. A terminal envelope that restates no
output leaves the completed items it already delivered authoritative; an empty
one with nothing streamed is an empty completion. A complete terminal response
needs no preceding deltas. A failure or interrupted stream never authorizes a
partial tool call.

A tool call's name and call ID are whatever the stream announced for that item
ID; argument events carry arguments alone, and a backend may state the identity
once rather than repeating it on every event. Announcements that contradict
each other or the terminal response are an error, never a merge. Every tool
call reaching the runtime carries a name, a call ID and complete arguments: one
whose name or call ID was never announced, or whose arguments never settled as
valid JSON, fails the turn instead of dispatching.

Responses completions have a 600-second total operation budget while model
catalog requests remain bounded to 60 seconds. SSE keeps at most 2 MiB of its
retained response; deltas and framing also have finite wire and parser limits.
A truncated, oversized, or failed stream is an error and is not replayed after
partial output. A successful terminal settles the request without waiting for
EOF; conflicting terminal frames already buffered are rejected.

The live preview is separate from semantic events, the turn journal and saved
conversation. Only the selected speaking request exposes text and visible
summaries, including when a relationship is the speaker. Private deliberation,
consultations, dreams, tool fragments and raw or encrypted reasoning do not
appear there. Visible summaries are transient and are not newly saved for replay.

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
| `file_read` | `path`, optional `offset`, `limit` | Workspace read |
| `file_list` | `path` | Workspace listing |
| `grep` | `pattern`, optional `path`, `include_hidden`, `include_ignored` | Checked project search |
| `glob` | `pattern`, optional `path`, `include_hidden`, `include_ignored` | Checked project path discovery |
| `file_write` | `path`, `content` | Requires an effective allow decision or approval |
| `file_edit` | `path`, `hunks` (`before`, `old`, `after`, `replacement`) | Uses the exact-file `file_write` permission; every hunk must match uniquely |
| `file_delete` | `path` | Requires an effective allow decision or approval |
| `shell` | `command` | Requires an effective allow decision or approval |

Paths are relative to the opened project capability. Absolute paths, parent
traversal and symlinks are rejected; capability-relative filesystem operations
provide containment beyond string-prefix checks. Sensitive directories such as
`.git`, `.kuru`, `.codex`, `.agents`, `.claude`, `.ssh` and credential/config
files are protected. Instruction files may be read but are protected from
mutation. Built-in UTF-8 file reads and shell streams retain a marked 2 MiB
head-and-tail excerpt after credential projection; shell stdout and stderr keep
independent budgets. MCP protocol records remain hard-bounded at 2 MiB. Shell
execution defaults to a 30-second timeout; an optional `timeout_ms` argument
accepts 1–120000 milliseconds. On Unix,
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
original group. Unix built-in shell ownership is process-wide bounded: at most
16 shell workers or unconfirmed retained process groups are admitted at once,
and capacity exhaustion is rejected immediately. An unconfirmed owner keeps its
slot until cleanup confirms root reap and process-group absence; retained
observations back off from 100 milliseconds to a one-second maximum without
abandoning ownership or using stale-PID signaling.
Windows retains its existing Job-based cleanup.

`grep` and `glob` use Kuru's bundled Rust search components; they never invoke
an installed `rg` executable. They search only regular UTF-8 project files and
exclude hidden and repository-ignored paths by default. Set
`include_hidden` or `include_ignored` independently when that discovery is
intentional. Search never follows links; it caps candidate files at 10,000,
individual file snapshots at 2 MiB, matched lines at 8 KiB, and returned
matches at 2,000. A binary, unreadable, protected, linked, denied, oversized,
or over-limit target is omitted without exposing its path or content; the
result's `omitted` object counts the applicable category, including oversized
matched lines. Each discovered candidate receives its own exact native
permission evaluation. Search does not request or grant the supplied directory
as a subtree, so an allow cannot override a more-specific deny. An unresolved
foreground ask is reported in the result's omission counts and does not disclose
that candidate.

`file_read` keeps its existing text response when called with only `path`.
Supplying either `offset` or `limit` selects a page of one-based logical UTF-8
text lines. The JSON page has `text`, `offset`, `limit`, `line_count`,
`next_offset` when more lines remain, and `omitted_lines`; pass `next_offset`
as the next request's `offset`. A supplied offset or limit must be a positive
integer. Offset zero, a limit outside 1–10,000, an out-of-range page, binary
text, more than 100,000 lines, or a file over 2 MiB fails without returning
partial page metadata.

When one provider response proposes several independent, already-authorized
`file_read`, `file_list`, `grep`, `glob`, or `web_fetch` calls, Kuru may run a
bounded group concurrently. It still evaluates each call's exact permission and
nested-instruction scope in provider order, reports each call with its original
ID, and returns the completed results to the provider in that original order.
`max_parallel` bounds the group. A fresh approval or instruction activation is
handled on the ordinary foreground path before later calls proceed. Writes,
shell, MCP, skill activation, cognition and unknown tools remain serial. A
permission, target, workspace or instruction change detected before a read
returns a refusal/replan result rather than exposing data under stale authority.

If a built-in shell cannot complete its normal capture or cleanup path, Kuru
returns one fixed operational category and a 4 KiB credential-projected stderr
excerpt. A pipe that has not reached EOF is reported as `stderr: <pending EOF>`;
Kuru does not return its partial bytes. These diagnostics never include the
command, stdout, native process observations, or raw error/cleanup chains. A
child that completes with a nonzero status still returns the ordinary structured
shell result with its exit code, stdout, and stderr.

Native tools and MCP calls share execution-time permission evaluation; outbound
`a2a_send` uses the same service before network dispatch. Rules match checked
native targets, configured MCP alias/original tool name, or the outbound A2A
alias. Explicit deny wins over saved grants. Unresolved asks without an attached
foreground approval channel return a typed permission-required refusal and
perform no tool effect. Workspace trust remains a separate prerequisite for
configured authority, including MCP startup. See
[tool permissions](configuration.md#tool-permissions).

The shell uses your process authority, not a sandbox. Configured MCP servers
also bring their own permissions; call approval does not impose a sandbox on
third-party tools.

The built-in shell receives a finite compatibility subset of inherited
environment variables for command discovery, home/profile, temporary paths,
locale, time and standard XDG locations. It does not inherit provider or
authentication variables, proxy configuration, agent sockets, arbitrary
`KURU_*` values, or shell-startup controls. Windows additionally derives its
stock system-shell paths and omits inherited `PSModulePath`, allowing stock
PowerShell to reconstruct its standard module paths. This reduces accidental
variable disclosure; it does not contain the shell's filesystem, process, or
network authority. Before its user command, this built-in ToolHost launch
initializes the shipped `Microsoft.PowerShell.Management` and
`Microsoft.PowerShell.Utility` modules from `$PSHOME`; ordinary module
autoloading remains available. Configured stdio MCP servers keep their own
inherited environment and explicit configuration overrides.

## Lifecycle hook protocol

Kuru writes one JSON object followed by a newline to each lifecycle hook's
stdin, then closes stdin. The version-1 request has `format`, `event`,
`invocation_id`, `actor`, optional `turn_id`, optional `call_id`, and `payload`.
Identifiers are opaque correlation values. The payload is limited to the event:

- `pre_turn`: `{ "input": string }`
- `pre_tool`: `{ "name": string, "arguments": object }`
- `speaker_selected`: `{ "speaker": string, "reason": string }`
- `post_tool`: `{ "name", "arguments", "result", "is_error" }`, using the
  same bounded credential-projected result used by runtime observations
- `post_turn`: `{ "answer": string, "outcome": "text" | "empty" }`

Dream `dream_suggest` calls use `pre_tool` and `post_tool` with their real
actor, invocation, and call IDs and no `turn_id`. Dreams have no `pre_turn`,
`post_turn`, or `speaker_selected` event. A dream rewrite cannot add another
tool class or bypass its proposal cap and candidate validation.

A `pre_turn` rewrite value is `{ "input": string }` (1–131,072 bytes, not blank).
The final rewritten input replaces the original in every provider projection of
that turn, including later turns' public-transcript context, resumed sessions
and forks. A retry that is no longer rewritten projects its original input.
The user-facing transcript and session export keep the original; see
[lifecycle hooks](configuration.md#lifecycle-hooks).
A `pre_tool` rewrite value is `{ "name": string, "arguments": object }`, where
`name` must repeat the proposed tool name exactly. Only `arguments` may change;
a different `name` is an invalid response.

The command must write exactly one JSON response and no trailing value. Pre
events accept `{ "decision": "allow" }`,
`{ "decision": "deny", "reason": string? }`, or
`{ "decision": "rewrite", "value": object }`. Speaker selection accepts
`{ "decision": "observe" }` or `{ "decision": "stop", "reason": string? }`;
neither response can name another actor. Post events accept `observe` or
`annotate` with an annotation string. Denial reasons and annotations redact
recognized credentials and escape terminal controls. Unknown fields, malformed
or trailing
JSON, invalid UTF-8, an event-incompatible decision, a failed exit status, or a
size or time violation fails that hook. Pre and speaker failures stop dispatch;
post failures are recorded separately and the remaining post chain continues.

Hook events expose only event type, configured ordinal, invocation/turn/call
correlation, and outcome classification: `allowed`, `rewritten`, `denied`,
`observed`, `stopped`, `annotated`, `failed`, or `suppressed` (a configured hook
not run below an owned hook launch). They never expose the command, its
arguments, stdin, stdout, stderr, rewritten value, result body, or annotation
body. Recognized credentials are projected before a post-tool result reaches a
hook, but an approved hook still has ordinary process authority and is not
sandboxed. See [lifecycle hooks](configuration.md#lifecycle-hooks) for bounds,
trust, ordering, cancellation, and direct-command scope.

Before a built-in file, shell, or MCP result crosses into the CLI or runtime,
Kuru projects a finite set of recognizable credential forms to
`[REDACTED:recognized-secret]`. It recognizes Basic/Bearer Authorization and
Proxy-Authorization values; OpenAI `sk-svcacct-`, `sk-proj-`, then `sk-`
prefixes, longest first, with at least 16 token bytes after that prefix;
GitHub (`ghp_`, `github_pat_`, `gho_`, `ghu_`, `ghs_`, `ghr_`)
forms with at least 8 following token bytes; Slack `xox*` and GitLab `glpat-`
forms with at least 16 following token bytes; and exact-length AWS
(`AKIA`/`ASIA` plus exactly 16 uppercase-alphanumeric bytes) token shapes;
supported PEM private-key blocks; JWT-shaped values with a base64url JSON-object
header containing a nonempty `alg` field and three base64url segments; and URL
userinfo of the form `scheme://user:password@host` for bounded RFC-style
schemes (`[A-Za-z][A-Za-z0-9+.-]*://`), including custom schemes and mixed-case
spellings. Exact contextual sensitive field names include `api_key`,
`password`, `refresh_token`, bare `token`, and bare `secret`.
Those local heuristic floors reduce accidental matches; they do not validate a
credential. Exact sensitive JSON fields have their whole
value replaced, including non-string values, while other JSON strings and plain
text retain unmatched bytes. AWS matching uses its uppercase-alphanumeric token
alphabet for the trailing boundary, so a following lowercase byte is outside that
token shape. Ordinary dotted text, malformed or short JWT-like values, URLs
without userinfo, and names such as `tokenize` or `secretary` remain unchanged.

This projects returned results, actionable MCP application errors and outward
tool-failure details, including error formatting and source chains. It
does not inspect credential stores or enumerate environment values, alter tool
arguments or file writes, rewrite source files or earlier history, or impose a
new combined result limit. Unknown, encoded, split, transformed, or future
credential formats can remain visible, and ordinary source text can be matched
by a documented form; tool results are not byte-exact backups. Stdio MCP stderr
uses the same finite scanner before Kuru retains a bounded, terminal-escaped
tail for direct human diagnostics. Ordinary server-authored text can remain;
the tail is not sent to models, runtime events, or memory.

## MCP

Both stdio and Streamable HTTP transports initialize the server, send
`notifications/initialized`, discover paginated tools, and call `tools/call`.
The preferred protocol version is `2025-11-25`; the adapter also accepts
`2025-06-18`, `2025-03-26` and `2024-11-05`. HTTP sessions preserve the negotiated
session ID and version headers and are closed when possible. Stdio subprocesses
have an independent native owner which continuously drains stderr and retains
process and pipe ownership through bounded cleanup.

Each tool receives a stable namespaced identifier derived from the configured
server alias and original tool name. Its description retains both names. This
prevents collisions with built-ins and between servers. A server must advertise
tools capability and valid input schemas. Calls are bounded to 60 seconds and
2 MiB; pagination is bounded and repeated cursors are rejected. Tool mutations
are not retried automatically after transport failure. Discovery validates and
publishes each configured alias separately, so one failed server does not hide
built-in tools or tools from healthy servers. Failed aliases report fixed status
and their prior routes dispatch nothing. A later explicit discovery can recover
an alias after its complete catalog validates.

The adapter is a tools client. It does not implement every optional MCP surface,
such as prompts/resources UI, elicitation or sampling. For HTTP authorization,
Kuru applies the MCP 2026-07-28 authorization contract over the existing
negotiated transport without claiming full 2026 stateless-wire support. It
validates protected-resource and authorization-server discovery, resource
binding, PKCE, state, RFC 9207 issuer responses, challenge-authoritative scopes,
configured/CIMD/DCR client registration and advertised device flow.

Kuru stores its own alias-bound OAuth records only in the native secret store.
Refresh rotates the stored generation before use. An invalid-token discovery may
refresh and retry discovery once; tool calls and ambiguous rotating refreshes are
never replayed. Static authorization headers and OAuth are mutually exclusive on
one alias. `mcp login|status|logout` and `/mcp login|status|logout` share the same
connector operations. Logout always deletes local state and reports remote
revocation independently.

## A2A

Kuru supports a bounded nonstreaming A2A 1.0 JSON-RPC `SendMessage` flow. Messages
use `messageId`, `contextId`, `ROLE_USER` and text parts. Responses contain either
a message or a task; terminal task text is extracted from status messages and
artifacts. The adapter sends `A2A-Version: 1.0`. Protocol errors, malformed
responses, response-size excess and timeouts become observable call errors.

For inbound messages, `messageId` is the durable turn ID within the current
project session. Repeating the same ID and exact request returns the stored answer
without another model or tool call. Reusing it with changed text or target fails,
and an interrupted request that may have dispatched external work must use a new
ID. The same ID remains independent in another session. IDs contain 1–256 bytes.
Ingress allows a turn up to 10 minutes. On timeout it signals cancellation and
allows up to 35 more seconds for accepted memory work and the answer race to
settle; that allowance does not prove cleanup of an owned subprocess.

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
