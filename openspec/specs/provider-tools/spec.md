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
filesystem sandbox; workspace approval does not widen tool roots or make
private same-user state inaccessible to a shell.

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

The built-in Unix shell MUST register an independent owner before launch and
retain its root child, stdout/stderr readers, and exact workspace `Directory`
through cleanup. Its public timeout MUST begin at invocation acceptance and its
caller wait MUST end within that operation deadline plus one five-second
cleanup-confirmation allowance. Natural completion MUST require both pipe EOFs
and non-reaping root-exit observation, terminate remaining members of the
original group before reaping the root, preserve that root's original status,
and observe group absence before returning success. Timeout, overflow, read
failure, caller loss, parent-runtime loss, and shutdown MUST enter the same
owned cleanup path without replacing the primary failure.

`ToolHost` shutdown MUST close shell registration, request cancellation, and
await all registered owners within one bounded observation window while still
running MCP cleanup. A bounded unconfirmed result MUST leave the independent
worker holding its process, pipe, and workspace capability until later reap and
absence confirmation; it MUST NOT claim synchronous cleanup. This ownership is
limited to the built-in shell and MUST NOT claim control of escaped processes,
the memory writer lease, or configured MCP lifecycles.

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

#### Scenario: Unix root exits with a silent descendant
- **WHEN** both shell pipes close and the root exits while a same-group descendant remains alive
- **THEN** Kuru terminates the remaining group before root reap and returns the exact root status only after group absence is observed.

#### Scenario: Unix shell caller disappears
- **WHEN** a shell call or its parent Tokio runtime disappears after launch
- **THEN** the independent registered owner retains the child, pipes, and workspace capability through bounded cleanup, and `ToolHost` shutdown observes confirmation or reports that ownership remains unconfirmed.

#### Scenario: Unix worker is delayed before launch
- **WHEN** a registered worker is delayed beyond the accepted operation deadline and cleanup allowance while no child has spawned
- **THEN** the caller returns a fixed bounded cancellation or unconfirmed-cleanup result, and the later worker observes cancellation and starts no child.

#### Scenario: Unix ownership observation remains interrupted
- **WHEN** repeated bounded `EINTR` leaves an owned root anchored beyond the caller's cleanup allowance
- **THEN** the caller receives a fixed unconfirmed result while the registered worker retains ownership, later makes its one destructive transition after a valid observation, and never signals again after that transition starts.

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

### Requirement: Recognizable-secret tool-result projection

Before returning any built-in, shell or MCP result through `ToolHost::execute`,
the connector SHALL project a finite documented set of recognizable credential
patterns to the exact marker `[REDACTED:recognized-secret]`. The projection MUST
cover successful text and typed JSON, useful MCP application-error content and
every outward tool error Display, alternate Display, Debug and source-chain path.
It MUST NOT claim recognition of arbitrary, encoded, split, transformed or future
secret formats.

The finite set MUST include Basic and Bearer Authorization and
Proxy-Authorization values; bounded local OpenAI, GitHub and AWS token heuristics;
supported private-key blocks; and exact contextual sensitive field names. In
typed JSON, an exact contextual or Authorization key MUST replace its complete
associated value even when that value is non-string, while other string keys and
values replace only recognized spans. Typed JSON MUST remain valid and MUST fail
safely rather than overwrite members if projected keys collide. Arbitrary text
which happens to parse as JSON MUST remain text, while bare and matching-quoted
contextual/header assignment syntax is still recognized.

#### Scenario: Successful tool content is projected

- **WHEN** an allowed file read, native shell result or successful stdio or HTTP MCP response contains a supported synthetic credential pattern
- **THEN** `ToolHost::execute` returns the visible marker in place of that pattern and returns no raw match through another error or formatting path

#### Scenario: Sensitive typed field is projected

- **WHEN** a typed JSON tool result contains an exact sensitive or Authorization key with a string, numeric, boolean, null, array or object value
- **THEN** its whole associated value is the marker, all unrelated structure remains semantically unchanged and the serialization is valid JSON

#### Scenario: Quoted contextual text is projected without JSON coercion

- **WHEN** arbitrary text contains a matching-quoted contextual or Authorization key and value in JSON-like syntax
- **THEN** only its recognized value span is replaced and the surrounding text is not parsed, reordered or reserialized

#### Scenario: Unrecognized ordinary output is preserved

- **WHEN** tool output contains ordinary source text, hashes, UUIDs, model names, generic base64 or JWT-like values, certificates or provider-like strings below the documented local floors
- **THEN** its unmatched bytes remain identical and Kuru makes no claim that an unknown credential shape would be found

### Requirement: Typed tool output and application errors

Tool implementations SHALL retain a private text-versus-JSON result type through
projection while preserving the public `Result<String>` interface. MCP
`isError=true` content MUST remain a useful projected application error and MUST
NOT be reclassified as transport unavailability. Tool transport and protocol
failures MUST retain a typed safe category without parsing formatted text, and
the outward error MUST NOT retain a raw source-chain bypass.

#### Scenario: MCP application error remains useful

- **WHEN** a healthy MCP server returns `isError=true` with ordinary instructions and a supported synthetic credential
- **THEN** the caller receives the useful instructions with the credential replaced and the server is not marked unavailable

#### Scenario: Raw remote failure cannot escape through formatting

- **WHEN** an MCP transport or protocol error contains a supported synthetic credential in its remote detail
- **THEN** Display, alternate Display, Debug and complete source-chain formatting expose only the typed safe category and projected detail

### Requirement: Projection preserves authority and producer bounds

Tool-result projection MUST NOT change tool arguments, file writes, remote
requests, earlier durable history, provider credential stores or environment
selection. It SHALL preserve the existing per-producer limits: 2 MiB file reads,
10,000-entry complete file listings, independently bounded 2 MiB shell stdout and
stderr fields, and bounded MCP messages/responses. It MUST NOT impose a new
global 2 MiB result limit. Projection allocation and runtime MUST remain bounded
relative to already-admitted producer input, and redaction MUST occur before any
truncation or human terminal escaping.

#### Scenario: Write input remains exact

- **WHEN** an authorized file write receives content matching a supported detector
- **THEN** the exact requested bytes are written and only a later returned projection is eligible for filtering

#### Scenario: Combined shell output keeps its current allowance

- **WHEN** shell stdout and stderr each contain an allowed result near their independent existing limits
- **THEN** the complete typed shell result remains successful and projection does not reject it under a new combined 2 MiB limit

#### Scenario: Chunk and EOF boundaries do not leak

- **WHEN** a supported token, contextual value or private-key block crosses any scanner chunk boundary or ends incompletely at EOF
- **THEN** streaming and whole-value projection agree and no recognized fragment is exposed by truncation or finalization

### Requirement: Projected runtime tool authority

Only the projected result or safe projected error SHALL cross from ToolHost into
direct CLI output or runtime tool forwarding. Runtime persistence and subsequent
provider prompts MUST use that same projected value, while prior history and the
tool's source or side-effect data remain unchanged.

Existing context byte limits MAY omit projected content. Tool-specific
truncation MUST keep retained replacement markers whole and MUST NOT bisect an
earlier marker when shortening a prefix to fit one. If a complete marker cannot
fit the available budget, it MUST be wholly omitted with the existing bounded
truncation indication. Generic chat truncation and legacy tool-history parsing
semantics MUST remain unchanged.

#### Scenario: Runtime persists the projection

- **WHEN** a real tool turn returns a supported synthetic credential followed by an ordinary completion
- **THEN** the persisted tool message and next provider-facing context contain the same marker, existing activity remains metadata-only, and none contains the raw credential

#### Scenario: Context limit intersects a replacement marker

- **WHEN** a current tool output or older tool message reaches an existing byte limit inside a replacement marker
- **THEN** the retained marker is complete when it fits, otherwise it is wholly omitted with bounded truncation indication; UTF-8 and existing byte limits remain valid, and arbitrary legacy tool text gains no new parsing requirement
