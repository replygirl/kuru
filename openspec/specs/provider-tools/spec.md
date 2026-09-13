# provider-tools Specification

## Purpose
Connect peer actors to supported inference and authentication transports,
workspace tools, MCP servers and A2A peers through extensible interfaces with
explicit authority, bounded execution and protected private state.

## Requirements

### Requirement: Supported OpenAI transport

The application SHALL support native ChatGPT OAuth authentication and direct
OpenAI subscription inference, and Responses API authentication through a
configurable environment variable, with dynamically discovered models and
available effort settings. Login, refresh, status and logout MUST manage only
Kuru's checked private credentials. The application MUST NOT require or embed
the Codex CLI/app-server or switch billing routes after authentication failure.
An automatic-ancestor Responses route MUST receive matching workspace approval
before Kuru reads its named environment variable or contacts its API base;
fixed ChatGPT login and logout MUST NOT read an unrelated workspace-selected
API-key environment variable.

#### Scenario: Future effort setting
- **WHEN** the provider advertises an unfamiliar effort value
- **THEN** the application preserves it and can send it without requiring a code change.

#### Scenario: Standalone authentication
- **WHEN** Kuru is installed without another agent harness
- **THEN** native browser/device login, credential status and direct authenticated requests work without that harness.

#### Scenario: Invalid callback or cancelled login
- **WHEN** a callback has an invalid state or a pending login is cancelled
- **THEN** Kuru preserves its previous credentials and releases the owned callback resources.

#### Scenario: Concurrent refresh and logout
- **WHEN** requests race token rotation or logout
- **THEN** validated rotation is reused without overwriting a newer login or resurrecting a logged-out session.

#### Scenario: Pending workspace API route
- **WHEN** an unapproved automatic ancestor selects a Responses credential route
- **THEN** login and logout still use their fixed ChatGPT route, while an
  active Responses status check requires approval; none reads the selected
  API-key environment value before its applicable preflight permits it.

### Requirement: Provider-neutral inference boundary

The application SHALL own peer context and tool execution independently of the
provider. Native OpenAI requests MUST use Kuru's existing supplied context and
function tools without executing another harness or importing its agent policy.

#### Scenario: Tool proposal
- **WHEN** a provider returns a tool request
- **THEN** the runtime validates and executes it through Kuru's configured tool host.

### Requirement: Explicit bounded tools

Filesystem tools MUST enforce canonical workspace containment and protect
instruction/configuration paths. Mutations and shell execution MUST require
explicit opt-ins and matching workspace approval when automatic ancestor
configuration contributes their effective grant. Shell execution MUST have time
and output bounds and SHALL be described as process authority rather than a
filesystem sandbox; workspace approval does not widen tool roots or make private
same-user state inaccessible to a shell.

The built-in shell MUST receive only this finite inherited compatibility
environment when each entry exists. Unix entries are `PATH`, `HOME`, `USER`,
`LOGNAME`, `TMPDIR`, `TMP`, `TEMP`, `LANG`, `LC_ALL`, `LC_COLLATE`, `LC_CTYPE`,
`LC_MESSAGES`, `LC_MONETARY`, `LC_NUMERIC`, `LC_TIME`, `TZ`, `NO_COLOR`,
`XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` and
`XDG_RUNTIME_DIR`. Windows entries are that same set plus `USERNAME`,
`USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`, `ProgramData`,
`ProgramFiles`, `ProgramFiles(x86)`, `ProgramW6432`, `PROCESSOR_ARCHITECTURE`,
`PROCESSOR_ARCHITEW6432` and `PATHEXT`, using ordinal case-insensitive names.
Windows MUST derive `SystemRoot`, `WINDIR` and `ComSpec` from the native system
directory instead of inheriting their values, and MUST use the conventional
`.COM;.EXE;.BAT;.CMD` `PATHEXT` only when the parent has none. Case-equivalent
duplicates of inherited compatibility names MUST fail deterministically rather
than select an ambiguous value. Duplicates of excluded names and native-derived
OS path names MUST NOT affect the projection.

Every other inherited entry, including provider/authentication values, proxy
configuration, SSH-agent handles, arbitrary `KURU_*` entries and shell startup
inputs, MUST be absent. `PSModulePath` MUST remain absent so the selected stock
Windows PowerShell reconstructs its standard modules. This variable reduction
MUST NOT be described as containing the shell's filesystem, process or network
authority, and an allowlisted name MUST NOT be treated as proof that its value
is nonsecret. Configured stdio MCP inheritance and `McpConfig.env` overrides
MUST remain unchanged.

#### Scenario: Symlink escape

- **WHEN** a filesystem call follows a workspace symlink outside the root
- **THEN** the operation fails without modifying the outside file.

#### Scenario: Pending shell or write grant

- **WHEN** an automatic ancestor enables shell or writes without matching approval
- **THEN** the tool host neither exposes nor executes that authority.

#### Scenario: Authorized shell receives finite compatibility environment

- **WHEN** an authorized built-in shell starts from a parent containing the documented compatibility entries plus fake provider, proxy, SSH-agent, Kuru, loader and startup-injection values
- **THEN** the real child observes the exact applicable compatibility values and none of the other parent entries, while its ordinary cwd, command discovery, output, timeout and cleanup behavior remains available.

#### Scenario: Stock Windows shell uses native baseline

- **WHEN** the built-in shell starts native Windows PowerShell with case-varied environment names, an inherited `PATHEXT` or no `PATHEXT`, and hostile replacements for OS shell paths and `PSModulePath`
- **THEN** it uses the native-derived system paths, preserves the sole inherited `PATHEXT` or the conventional fallback, rejects case-equivalent ambiguity, reconstructs stock modules and receives no `PSModulePath`.

#### Scenario: Configured stdio MCP retains its environment contract

- **WHEN** a configured stdio MCP starts after built-in shell minimization with one fake inherited non-allowlisted sentinel and one explicit `McpConfig.env` override
- **THEN** the MCP child receives both under its existing contract rather than the built-in shell projection.

### Requirement: Standard protocol adapters

The application SHALL initialize and call MCP tools using stdio or Streamable
HTTP, namespace tool names, and communicate with peers over a documented A2A
JSON-RPC subset. Effective automatic-ancestor MCP transport and external-agent
configuration MUST receive matching workspace approval before discovery,
process creation, connection, advertisement or invocation.

#### Scenario: Unresponsive protocol peer
- **WHEN** a protocol peer fails to respond
- **THEN** the call fails within its timeout and the harness remains usable.

#### Scenario: Pending configured transport
- **WHEN** automatic ancestor configuration supplies a stdio MCP, HTTP MCP or
  external agent without matching approval
- **THEN** Kuru starts no stdio child, opens no configured connection, and does
  not advertise the external agent.

### Requirement: Bounded provider failure diagnostics

Responses completion and model-catalog requests, Responses HTTP-success error
envelopes, and native ChatGPT failed stream events SHALL produce one finite,
Kuru-owned diagnostic class. The application MUST render only fixed Kuru text
and an HTTP status where present; it MUST NOT render provider message, parameter,
header, unknown code/type, endpoint URL/query, or other remote diagnostic text.

Provider failed-body inspection MUST be limited to 8 KiB and a two-second total
reader deadline within the existing operation deadline. A malformed, oversized,
stalled, or dribbling body MUST retain its fixed status-derived diagnostic. The
application MUST sanitize every error-chain element exposed to callers under
both Display and Debug formatting, including unexpected HTTP-success status
strings, and MUST
preserve the existing single rejected-401 rotation and no-replay-after-partial
completion rules.

#### Scenario: Known API model error is redacted
- **WHEN** a Responses completion receives HTTP 400 with `error.code` equal to
  `model_not_found` and a body message containing a fake secret
- **THEN** it reports the fixed selected-model availability/access diagnostic
  with HTTP 400 and no error-chain element contains the secret or model text

#### Scenario: HTTP-success error envelope is redacted
- **WHEN** a Responses completion returns HTTP 200 with a non-null error object
  whose message contains a fake secret
- **THEN** it reports a fixed provider protocol failure and no error-chain
  element contains the message

#### Scenario: Known native failed event is classified
- **WHEN** a successful ChatGPT SSE connection emits a terminal
  `response.failed` event with the observed `server_is_overloaded` code
- **THEN** it reports the fixed overloaded-service diagnostic without accepting
  or replaying the completion

#### Scenario: Unknown native failed event remains bounded
- **WHEN** a successful ChatGPT SSE connection emits a terminal failed event
  with an unrecognized code or type and a fake-secret message
- **THEN** it reports the fixed stream/protocol failure without rendering the
  code, type, message, or secret and without replaying the completion

#### Scenario: Dribbling failed body times out within the operation budget
- **WHEN** a non-success provider response dribbles its diagnostic body beyond
  the two-second diagnostic-reader deadline
- **THEN** it returns the fixed status-derived diagnostic before the containing
  catalog or completion deadline expires

### Requirement: Bounded provider retry and credential rotation

Responses API-key and native ChatGPT subscription catalog and completion
operations SHALL use one private absolute operation budget across provider
retries, rejected-401 credential rotation, bounded diagnostics, and backoff.
The catalog deadline MUST remain 60 seconds and the completion deadline MUST
remain 600 seconds. An operation MUST execute at most four application HTTP
sends in total, at most three provider endpoint sends, at most two retry
delays, at most one logical credential rotation, and at most two refresh HTTP
sends; the second refresh send is permitted only after the first is proved not
dispatched. A provider retry MUST reserve sufficient remaining send capacity
for any started rotation and provider replay.

The application MAY replay only an explicit rejected provider HTTP response
with status 429, 500, or 503 before a successful JSON body or SSE stream is
accepted. Responses API quota or billing codes, all other statuses, ambiguous
transport outcomes, malformed or incomplete bodies, accepted streams, partial
completion output, and any provider error after response acceptance MUST remain
terminal and MUST NOT be replayed. A received native subscription 401 MAY use
the one logical rotation and replay only within the same operation budget; a
second 401 is terminal. The route, account, session, request body, actor-local
pending state, and tool-execution ordering MUST remain unchanged through retries
until the ordinary update after one fully validated completion. The request body
and actor-local protocol input MUST be built once and cloned for each provider
send; API-key environment resolution MUST occur once per operation.

The implementation MUST inspect at most one bounded Retry-After value without
rendering it. It MAY accept delta-seconds or an exact canonical IMF-fixdate and
otherwise uses a bounded jittered fallback. It MUST NOT shorten a valid server
minimum, sleep more than 30 seconds per retry or 60 seconds total, reset the
operation deadline, or send after the chosen delay cannot fit the remaining
budget. An all-digit delay that overflows the represented duration is an unfit
server minimum and MUST terminate without a fallback replay. Exhaustion MUST
return fixed redacted text and the application attempt count, without remote
body, header, URL, token, or raw transport-chain values.

The fallback bases MUST be 500 milliseconds then one second with equal jitter
in each `[base / 2, base]` interval. The implementation MUST seed once per
operation from the existing OS entropy source and use the upper bound if entropy
is unavailable. It MUST use the greater of a valid header minimum and jittered
fallback, round a positive date-derived fractional wait up, and supply only
private fixed clock/seed controls to tests.

Refresh remains an owned rotating-credential operation under the existing
credential lease. The caller MUST grant its RefreshAllowance with an absolute
`std::time::Instant` capped by the caller's then-remaining operation deadline
and the auth 60-second limit; a worker starting later MUST NOT re-anchor or
extend that authorization. The owner MUST check caller closure and this
deadline before its first and each subsequent OAuth POST. It MAY repeat exactly
one refresh POST only for auth-local `reqwest::Error::is_connect` when the
pinned connector graph establishes every applicable `ErrorKind::Connect` site
occurs during connection acquisition before `pooled.try_send_request`, and a
native fixture observes zero OAuth POSTs. Proxy CONNECT traffic is not an OAuth
POST proof. The implementation MUST classify that exact audited auth-local
`is_connect` result first, including when the same connection-acquisition error
also reports a timeout or arises while acquiring direct or proxy transport.
`is_timeout`, proxy use, or observed CONNECT traffic MUST NOT independently
establish that the token POST was absent. Every other final canceled/send error,
timeout, proxy outcome, HTTP response, response loss, or possibly dispatched
outcome MUST NOT be replayed. It MUST instead reconcile the exact durable
new-generation, pending, or unknown state. A lower HTTP/1 layer internally
returning an unaccepted request MUST NOT be inferred from the final reqwest
canceled/send result because auth does not receive that request-object proof.

The owner MUST prepare and clone the secret-bearing refresh request before it
publishes pending, then publish and reread the exact pending record under the
retained lease before any OAuth POST. Local preparation failure makes zero HTTP
calls and leaves the record unchanged. The shared rotation permission is used
by expiry resolution, received 401, or reuse of a durably newer generation;
newer-generation reuse consumes no refresh HTTP send. Once rotation starts,
the RefreshAllowance MUST reserve one provider replay and the caller MUST NOT
reclaim that reservation while the owner is live.

After pending state is published, every return path MUST reconcile an exact
old-token rollback, exact new-generation record, or exact pending record before
releasing its lease; unreadable, missing, or mismatched state MUST be reported
as unknown. Caller loss or caller deadline MAY stop waiting, but MUST NOT cancel
post-dispatch response and publication reconciliation. If no OAuth POST was
dispatched and the owner stops before dispatch, it MUST attempt and reconcile
the exact rollback before lease release; an uncertain rollback publication
remains pending/unknown. The owner MUST remove the outer whole-refresh timeout
that can cancel it, using bounded phase deadlines instead. The provider and
refresh clients MUST explicitly disable library policy retries; this requirement
MUST NOT alter generic MCP, A2A, browser/device login, logout, or
authentication-management behavior.

#### Scenario: Rejected provider response uses the shared budget
- **WHEN** one completion receives a rejected 503, a rejected subscription 401,
  a successful checked rotation, and then a successful provider response
- **THEN** it makes exactly four application sends, at most three provider sends,
  exactly one logical rotation, and returns the completed result with the same
  route, account, session, and tool ordering; its pending state changes only by
  the ordinary update after that validated result

#### Scenario: Accepted or ambiguous provider result is never replayed
- **WHEN** a completion request is accepted and then its JSON or SSE body is
  malformed, partial, idle, canceled, or disconnected, or a raw peer reads the
  full request and closes without a response
- **THEN** it returns one fixed terminal failure with no later provider send or
  public partial completion

#### Scenario: Proved-undispatched refresh can repeat once
- **WHEN** the native HTTP/1 refresh fixture observes the audited auth-local
  `is_connect` connection-acquisition result before the OAuth POST reaches its
  endpoint and the shared budget reserves refresh and provider replay capacity
- **THEN** the owned worker may make one further OAuth POST, the fixture observes
  zero POSTs before that second execution, and the successful rotation publishes
  exactly one newer credential generation

#### Scenario: Possibly dispatched refresh reconciles without replay
- **WHEN** an OAuth POST has an HTTP response, response loss, a timeout outside
  the audited auth-local `is_connect` connection-acquisition class, or any other
  transport result outside that exact class
- **THEN** the owner does not replay it and reconciles an exact durable pending
  or newer-generation record, or reports unknown state, before lease release

#### Scenario: Delay cannot fit the operation deadline
- **WHEN** a retryable rejected response has a valid Retry-After minimum that
  exceeds the remaining operation budget, retry-delay cap, or representable
  duration
- **THEN** the operation returns a fixed exhaustion failure without sleeping or
  sending a replay
