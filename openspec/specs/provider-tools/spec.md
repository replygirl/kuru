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
and independent stdout and stderr retained-output bounds and SHALL be described
as process authority rather than a filesystem sandbox; workspace approval does
not widen tool roots or make private same-user state inaccessible to a shell.

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

The built-in Unix shell MUST reserve one fixed process-wide retained-owner
admission slot before it creates an OS worker or process; no registry, dropped
registry, caller, or parent runtime may bypass that shared bound. Capacity
exhaustion MUST fail immediately with fixed Kuru-authored text, without a queued
waiter, worker, or process. That slot MUST follow the independent owner through
its root child, stdout/stderr readers, exact workspace `Directory`, and every
unconfirmed cleanup retry; it MUST release exactly once only after a worker
terminates before spawning a child or confirmed root reap and group absence.

Its public timeout MUST begin at invocation acceptance and its caller wait MUST
end within that operation deadline plus one five-second cleanup-confirmation
allowance. Natural completion MUST require both pipe EOFs and non-reaping
root-exit observation, terminate remaining members of the original group before
reaping the root, preserve that root's original status, and observe group
absence before returning success. Timeout, overflow, read failure, caller loss,
parent-runtime loss, and shutdown MUST enter the same owned cleanup path without
replacing the primary failure.

After confirmed cleanup, or after a worker terminates before spawning a child,
the worker MUST remove only its own registry reservation before publishing its
result to the caller. A spawned worker whose cleanup remains unconfirmed MUST
remain registered and retain its admission slot until later confirmation. Its
indefinite retained cleanup MUST make one phase-safe existing cleanup
observation, then use capped exponential retry intervals before its next actual
platform observation; it MUST never abandon confirmed ownership, introduce a
give-up deadline, or signal a numeric PID after the owned group’s phase no
longer permits it.

`ToolHost` shutdown MUST close shell registration, request cancellation, and
await all registered owners within one bounded observation window while still
running MCP cleanup. A bounded unconfirmed result MUST leave the independent
worker holding its process, pipe, workspace capability, and admission slot until
later reap and absence confirmation; it MUST NOT claim synchronous cleanup.
This ownership is limited to the built-in shell and MUST NOT claim control of
escaped processes, the memory writer lease, or configured MCP lifecycles.

Built-in file-read and shell output that exceeds its retained-output budget MUST
remain a bounded visible head-and-tail excerpt after recognized-secret projection,
rather than fail solely for crossing that retention budget. The excerpt MUST keep
UTF-8 boundaries and every visible recognized-secret marker whole. Model-facing
tool receipts MUST retain the call ID and valid JSON while applying the same
bounded excerpt rule. MCP stdio, HTTP JSON, and SSE framing/parser admission
bounds remain separate protocol limits.

#### Scenario: Unix shell admission is process-wide
- **WHEN** independent shell registries retain owners until their shared fixed admission cap is occupied
- **THEN** a later shell request receives the fixed capacity error before a worker or child starts, and no registry can exceed the shared cap.

#### Scenario: Unconfirmed shell ownership retains its slot
- **WHEN** a caller and its registry are dropped after an owned cleanup remains unconfirmed
- **THEN** the retained worker keeps its admission slot and later releases it exactly once only after phase-safe reap and group-absence confirmation.

#### Scenario: Retained shell observations are paced
- **WHEN** cleanup remains unconfirmed beyond the caller’s five-second cleanup allowance
- **THEN** the worker retains the owned group indefinitely but makes actual phase-safe cleanup observations at capped exponential intervals rather than repeatedly running a short polling window.

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

#### Scenario: Confirmed worker publication has no stale reservation
- **WHEN** a confirmed Unix shell worker wakes its result receiver while another shell owner remains registered
- **THEN** its own reservation is already absent and the other owner remains registered.

#### Scenario: Unix ownership observation remains interrupted
- **WHEN** repeated bounded `EINTR` leaves an owned root anchored beyond the caller's cleanup allowance
- **THEN** the caller receives a fixed unconfirmed result while the registered worker retains ownership, later makes its one destructive transition after a valid observation, and never signals again after that transition starts.

#### Scenario: Oversized built-in shell streams
- **WHEN** shell stdout or stderr exceeds its independent retained-output budget
- **THEN** the completed tool result preserves a marked head and tail for that stream, drains both streams through EOF within the existing operation and cleanup authority, and does not combine their budgets or extend its deadline.

#### Scenario: Tail survives a model receipt boundary
- **WHEN** a projected built-in tool result exceeds the model receipt budget
- **THEN** the receipt remains valid JSON with its original call ID and contains a marked head-and-tail excerpt without a partial recognized-secret marker.

### Requirement: Standard protocol adapters

The application SHALL initialize and call MCP tools using stdio or Streamable
HTTP, namespace tool names, and communicate with peers over a documented A2A
JSON-RPC subset. Effective automatic-ancestor MCP transport and external-agent
configuration MUST receive matching workspace approval before discovery,
process creation, connection, advertisement or invocation. An MCP RPC close that
confirms cleanup MUST close command admission before publishing that result, and
later close calls MUST observe the same authoritative confirmed completion.

#### Scenario: Unresponsive protocol peer
- **WHEN** a protocol peer fails to respond
- **THEN** the call fails within its timeout and the harness remains usable.

#### Scenario: Pending configured transport
- **WHEN** automatic ancestor configuration supplies a stdio MCP, HTTP MCP or
  external agent without matching approval
- **THEN** Kuru starts no stdio child, opens no configured connection, and does
  not advertise the external agent.

#### Scenario: Repeated close after confirmed cleanup
- **WHEN** confirmed MCP cleanup publishes its first close result and a caller
  immediately requests close again
- **THEN** command admission is already closed and the later call returns the
  same confirmed completion without waiting for an abandoned command.

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
Proxy-Authorization values; bounded local OpenAI, GitHub, Slack, GitLab and AWS
token heuristics; JWT-shaped three-segment values; URL userinfo for bounded
RFC-style schemes; supported private-key blocks; and exact contextual sensitive
field names, including bare `token` and `secret`. JWT recognition MUST require a
base64url JSON-object header with a nonempty `alg` field and three bounded
base64url segments; it is a detector, not credential validation. An overlong
pending recognizable candidate MUST project fail-closed rather than return raw
buffered text. In typed JSON, an exact contextual or Authorization
key MUST replace its complete associated value even when that value is non-string,
while other string keys and values replace only recognized spans. Typed JSON
MUST remain valid and MUST fail safely rather than overwrite members if projected
keys collide. Arbitrary text which happens to parse as JSON MUST remain text,
while bare and matching-quoted contextual/header assignment syntax is still
recognized.

The connector SHALL expose its text and JSON projection operations as pure
reusable APIs while preserving `ToolHost::execute`'s existing public
`Result<String>` interface. The projection scanner MUST remain bounded across
arbitrary byte chunking and replace a recognized span before retained truncation
can expose a fragment.

#### Scenario: Successful tool content is projected

- **WHEN** an allowed file read, native shell or successful stdio or HTTP MCP
  response contains a supported synthetic credential pattern
- **THEN** `ToolHost::execute` returns the visible marker in place of that
  pattern and returns no raw match through another error or formatting path

#### Scenario: Sensitive typed field is projected

- **WHEN** a typed JSON tool result contains an exact sensitive or Authorization
  key with a string, numeric, boolean, null, array or object value
- **THEN** its whole associated value is the marker, all unrelated structure
  remains semantically unchanged and the serialization is valid JSON

#### Scenario: Quoted contextual text is projected without JSON coercion

- **WHEN** arbitrary text contains a matching-quoted contextual or Authorization
  key and value in JSON-like syntax
- **THEN** only its recognized value span is replaced and the surrounding text
  is not parsed, reordered or reserialized

#### Scenario: Unrecognized ordinary output is preserved

- **WHEN** tool output contains ordinary source text, hashes, UUIDs, model
  names, generic base64, certificates or provider-like strings below documented
  local floors
- **THEN** its unmatched bytes remain identical and Kuru makes no claim that an
  unknown credential shape would be found

#### Scenario: Expanded recognizable forms are projected

- **WHEN** allowed tool text or typed JSON contains a qualifying Slack `xox*`,
  GitLab `glpat-`, JWT-shaped, URL-userinfo, `token=` or `secret=` synthetic value
- **THEN** the returned projection contains the visible marker and no raw matched
  span

#### Scenario: Ordinary near matches remain exact

- **WHEN** tool output contains ordinary dotted identifiers, non-userinfo URLs,
  `tokenize` or `secretary`, an invalid or short JWT-like value, or a short token
  prefix
- **THEN** unmatched bytes remain identical

#### Scenario: Added forms survive streaming boundaries

- **WHEN** each added recognizable form is split at every byte boundary or read
  one byte at a time
- **THEN** streaming projection equals whole-value projection and contains no
  raw matched fragment

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

### Requirement: Alias-local MCP availability

MCP discovery SHALL validate and publish each configured server alias as one complete unit. A start, initialization, catalog transport, pagination, size, or schema failure MUST mark only that alias unavailable, omit all of its tools from the returned catalog, and report a fixed alias-specific status while built-in tools and completely validated healthy aliases remain usable. Previously known routes for an unavailable alias MUST reject without dispatch until a later explicit catalog operation completely validates and republishes that alias.

Initialization, every catalog page, tool calls, close, and alias-state publication MUST be serialized as logical operations per alias without serializing independent aliases. A call transport or protocol failure after possible dispatch, including caller cancellation, MUST make that alias unavailable and MUST NOT automatically resend the call. An MCP `isError=true` response MUST remain a projected application result and MUST NOT degrade a healthy session.

#### Scenario: One failed alias does not abort the turn

- **WHEN** catalog discovery visits one unavailable server and one healthy server in either configured order
- **THEN** the provider receives built-in and healthy-server tools, receives none from the failed alias, and the runtime records fixed unavailable metadata for that alias outside provider prompts, conversation messages, and authored notes

#### Scenario: Invalid partial catalog is not published

- **WHEN** one alias returns valid pages followed by a malformed, repeated, oversized, or otherwise invalid page
- **THEN** none of that alias's candidate tools becomes advertised or callable and another alias's published routes remain intact

#### Scenario: Ambiguous call is not replayed

- **WHEN** a server receives a tool call and the response fails or the caller disappears before completion
- **THEN** Kuru sends that call at most once, marks the alias unavailable, and rejects another call on its prior route without network or stdin dispatch until explicit successful rediscovery

#### Scenario: Application error keeps the alias healthy

- **WHEN** a server completes a tool call with `isError=true`
- **THEN** Kuru returns its recognizable-secret-projected useful content and later calls may use the same healthy session

### Requirement: Owned MCP stderr diagnostics

A configured stdio MCP session MUST keep one independent connector owner for the native child tree, stdin, stdout, stderr, retained workspace capability, request framing, and bounded cleanup. The owner MUST revalidate the original retained workspace immediately before process creation, continuously drain stderr until EOF or cleanup, and survive loss of a request future or its parent asynchronous runtime. Unix cleanup MUST consume group and root signal authority before exact root reap and perform no destructive numeric signal afterward; Windows cleanup MUST retain Job ownership through tree quiescence. Shutdown MUST reject new sessions before attempting every existing client within one aggregate bound, and any unconfirmed result MUST leave the independent owner retaining authority until later confirmation.

Recognizable-secret scanning MUST occur on stderr byte chunks before bounded retention. Only a bounded, terminal-escaped human diagnostic tail with saturating byte-count and truncation metadata MAY reach direct CLI stderr. Runtime events, tool errors, provider prompts, memory, serialization, and ordinary error chains MUST contain fixed Kuru-owned status metadata and MUST NOT contain captured stderr, command, arguments, environment, endpoint, response, or raw process error text.

#### Scenario: Stderr is drained and projected before retention

- **WHEN** a stdio server emits more than the diagnostic limit including a recognizable fake credential split across read boundaries, invalid UTF-8, and terminal controls
- **THEN** the server does not block, the human diagnostic is bounded and escaped with the credential replaced, and no raw captured byte reaches runtime metadata, prompts, memory, errors, or debug formatting

#### Scenario: Owner survives caller and runtime loss

- **WHEN** a request caller or its parent runtime disappears after possible stdin dispatch while a same-group or Job descendant retains stderr
- **THEN** the independent owner disables the session, drains or closes its pipes, performs bounded native cleanup without stale PID authority, and either confirms quiescence or honestly retains ownership

#### Scenario: Shutdown closes admission once

- **WHEN** shutdown races starting, active, and idle aliases
- **THEN** no new alias is admitted after closure, every already-admitted client receives a cleanup attempt, successful shutdown confirms their cleanup, and the aggregate wait is bounded once rather than once per configured server

### Requirement: Turn cancellation stops dispatch admission

Provider completion, built-in tool, MCP, and outbound A2A dispatch initiated by a turn MUST observe the same explicit cancellation signal before admission and while awaiting a cancellable response. Cancellation MUST NOT replay an ambiguous request. It MUST release logical runtime permits and locks while connector-owned subprocess workers retain responsibility for bounded cleanup and honest unconfirmed state.

#### Scenario: Cancel before dispatch
- **WHEN** cancellation becomes visible before an actor, provider, tool, MCP alias, or A2A request is admitted
- **THEN** that operation sends no request or process input and the next valid operation can proceed.

#### Scenario: Cancel after observed dispatch
- **WHEN** a local fixture observes one provider, shell, MCP, or A2A dispatch before cancellation
- **THEN** Kuru sends it at most once, records the turn as interrupted and possibly dispatched, and leaves any process cleanup with its existing retained owner.

#### Scenario: Accepted memory operation during cancellation
- **WHEN** a cognitive state or note mutation is accepted before cancellation is observed
- **THEN** the mutation is completed or reconciled before interruption is recorded and a later mutation begins.

### Requirement: Typed retry observation

Provider retry handling SHALL emit only literal operation category, attempt, safe status classification, delay or exhaustion outcome, and elapsed time to the application's operational tracing target. It MUST NOT emit request payloads, headers, endpoints, provider diagnostic text, or error chains.

#### Scenario: Retryable rejection
- **WHEN** a retryable provider rejection schedules a later attempt
- **THEN** the diagnostic observation identifies the typed retry outcome without admitting remote text.

### Requirement: Bounded built-in shell operational diagnostics

When an authorized built-in shell cannot complete its normal capture and
cleanup flow, the connector SHALL return one finite Kuru-authored operational
category and a bounded recognizable-secret-projected stderr diagnostic on both
Unix and Windows.  The public diagnostic MUST contain no command, arguments,
stdout, native process observation, raw error/cleanup text, or source chain.
It MUST expose an EOF-complete stderr stream only after streaming projection and
the existing 4 KiB diagnostic bound; a stream that has not reached EOF MUST be
rendered as the fixed explicit pending-EOF state and MUST NOT expose its partial
bytes.  The outward error MUST retain no raw source-chain bypass under Display,
alternate Display, or Debug formatting.

A child whose root status and both output streams have completed successfully
MUST retain the existing structured shell JSON result, including its original
nonzero exit status, stdout, and stderr.  Shell operational diagnostics MUST
NOT change Unix retained-owner admission, process-group cleanup/reap authority,
or Windows process/pipe ownership.

#### Scenario: Incomplete stderr after timeout

- **WHEN** an authorized built-in shell times out while its stderr pipe remains open after emitting a fake recognizable credential
- **THEN** its public error has the fixed timeout category and pending-EOF state, and no rendering or source-chain element contains the command, credential, partial stderr, stdout, or raw cleanup detail.

#### Scenario: Complete stderr before operational failure

- **WHEN** an authorized built-in shell reaches stderr EOF containing a fake recognizable credential before a later operational failure
- **THEN** its public error contains the same cross-platform category grammar and a 4 KiB-bounded redacted stderr excerpt with no raw matched credential.

#### Scenario: Completed nonzero shell exit

- **WHEN** an authorized built-in shell exits nonzero after both output streams reach EOF and cleanup is confirmed
- **THEN** it returns the existing structured JSON result with `success` false and its original exit code rather than an operational diagnostic.

### Requirement: Typed native tool continuations

The connector SHALL translate typed tool-use and tool-result blocks without
re-parsing new receipt prose. Live native continuation MUST validate call IDs,
retain pending call order and preserve current string versus JSON result encoding.
Legacy tool receipt strings MAY be adapted only at the established pending-call
boundary with matching call identity; arbitrary historical JSON-looking text
MUST NOT become executable tool structure. Native opaque reasoning continuation
MUST remain transient and scoped to its producing actor, never reconstructed
from durable history or shared with another actor. Completed text projection and
tool ordering MUST retain existing behavior across both native OpenAI routes.

#### Scenario: Multiple current receipts

- **WHEN** a completion requests multiple tool calls and matching typed receipts arrive in a different order
- **THEN** the next native request sends each result once in pending call order and retains that actor's exact native continuation.

#### Scenario: Receipt mismatch or actor isolation

- **WHEN** a receipt lacks a pending call, duplicates an ID, or belongs to another actor
- **THEN** it cannot complete a different actor's native continuation or bypass validation.

#### Scenario: Legacy text after a new turn

- **WHEN** a new user turn follows a historical JSON-looking tool record without a live pending continuation
- **THEN** the old record remains historical data and does not reconstruct opaque reasoning or authorize tool execution.

### Requirement: Canonical bounded provider streaming

Providers SHALL implement a normalized stream with text/refusal deltas, explicitly
visible reasoning-summary deltas, tool-argument fragments, usage observations and
terminal completion or failure. `complete` SHALL collect that same stream through
an async fallible sink, without a separate inference implementation or recursively
defined defaults. Only a validated successful terminal completion SHALL authorize
tool calls or a durable assistant message. A missing success terminal, terminal
failure, multiple delivered terminals, or events delivered after a terminal MUST
fail collection. Transport and sink errors MUST propagate without accepting a
partial completion. Usage MUST preserve missing versus explicit zero values;
terminal reported components take precedence, with earlier observed components
retained when the terminal omits them.

#### Scenario: Completion and streaming agree

- **WHEN** a deterministic provider emits deltas followed by a typed completion
- **THEN** streaming and completion collection produce the same ordered blocks,
  call IDs, usage and stop reason through the same inference path.

#### Scenario: Incomplete stream

- **WHEN** a request ends before a successful terminal, reports terminal failure,
  or its sink fails
- **THEN** no partial tool call or assistant completion is authorized and any usage
  reported before failure has been offered to the awaited observer.

#### Scenario: Partial usage

- **WHEN** cached-input or reasoning-output counts are absent or explicitly zero
- **THEN** collection preserves that distinction without adding those subsets to
  input/output totals or inventing terminal observations.

### Requirement: Native SSE reconciliation and continuation privacy

Both native OpenAI routes SHALL stream bounded SSE while retaining current wire,
event, line, retained-response and time limits. The native parser SHALL reconcile
observed text, refusal, visible-summary and function-argument fragments using raw
item identities and indexes before flattening terminal output into typed blocks.
Raw item IDs MUST NOT be confused with tool call IDs, and raw output indexes MUST
NOT be interpreted as flattened block positions. Terminal-only success SHALL be
valid. Conflicting identities, invalid completed argument JSON and unmatched
observed fragments MUST fail before tool authorization. The parser MUST reject
duplicate terminal frames already buffered before settlement, then settle without
waiting indefinitely for EOF. Native encrypted reasoning and pending continuation
MUST remain actor-private and MUST NOT enter normalized events or public previews.

#### Scenario: Chunked native reply

- **WHEN** either native route splits UTF-8, multiline SSE, text/refusal or tool
  JSON across chunks and includes opaque reasoning before final tool output
- **THEN** visible deltas arrive before terminal completion, native identities
  reconcile correctly, and only final valid typed calls can execute.

#### Scenario: Full terminal without deltas

- **WHEN** a native stream provides a complete terminal response without prior
  fragments or done events for every output item
- **THEN** collection succeeds without imposing unsupported event choreography.

#### Scenario: Conflicting or interrupted wire data

- **WHEN** buffered terminal records conflict, observed fragments disagree with
  final native output, limits are exceeded, or cancellation interrupts streaming
- **THEN** collection fails or cancels with no partial tool dispatch and no native
  continuation leakage.

### Requirement: Deterministic effect-bound tool decisions

Kuru SHALL evaluate bounded permission rules against the actual validated invocation at `ToolHost::execute` and before runtime outbound `a2a_send`. Selectors SHALL identify native tool names, MCP alias plus original tool name, or configured outbound A2A alias rather than provider-facing hashed names or descriptions. Optional file patterns SHALL match normalized, anchored project-relative targets only; absolute or traversing patterns MUST be rejected. Any matching deny SHALL win over matching ask, which SHALL win over matching allow, independent of rule order. With no matching rule, ordinary read/list tools and configured MCP/A2A calls SHALL retain their post-trust allow behavior; legacy `allow_write` and `allow_shell` true SHALL allow their respective calls, and false SHALL require approval. Ask-capable tools SHALL remain discoverable. Neither discovery nor a direct caller MAY bypass execution-time evaluation, the workspace root, protected paths, or trust preflight.

#### Scenario: Deny wins over grants and rule order
- **WHEN** an invocation matches deny, ask and allow rules in any order and a prior grant exists
- **THEN** Kuru returns a typed denied result and performs no external effect.

#### Scenario: Legacy false requests approval
- **WHEN** a validated write or shell call has no matching explicit rule and its legacy boolean is false
- **THEN** an attached foreground caller may request approval, while an unattended caller receives a typed permission-required refusal before dispatch.

#### Scenario: Direct and routed tools share the gate
- **WHEN** the same native, MCP or outbound A2A invocation is submitted through direct CLI or a model tool call
- **THEN** its effective rule and grant decision is the same; configured MCP startup still requires its separate workspace trust preflight.

#### Scenario: File matching cannot widen authority
- **WHEN** a file operation targets a checked path containing glob metacharacters or a path outside the checked workspace
- **THEN** exact-file grants treat the metacharacters literally, and neither a pattern nor a grant can authorize an outside or protected path.

### Requirement: Checked and revocable tool grants

An approval request SHALL bind the immutable tool identity, validated target, full arguments, retained workspace identity and effective authority context, while its bounded redacted display SHALL be separate from that authorization identity. Kuru SHALL revalidate the retained root immediately before dispatch. Once approval SHALL cover only that exact invocation; session approval SHALL cover the displayed exact-file or whole-tool scope only for the running session; always approval SHALL persist that scope in checked private Kuru state, bound to native root identity, complete reviewed manifest and effective permission/tool-route context. A matching explicit deny MUST remain effective despite any grant. Grant-store failure, cancellation, a closed approval surface, or an unanswered request MUST NOT authorize dispatch. The connector service and runtime outbound A2A path SHALL return a typed permission-required denial when no attached foreground surface can resolve an ask. Session and always grants SHALL be revocable; uncertain dispatch MUST NOT preserve a once grant for replay.

#### Scenario: Changed invocation cannot consume once approval
- **WHEN** a once approval was issued for one exact invocation and arguments, target, workspace identity or route change before execution
- **THEN** the changed operation receives a fresh decision and the old approval produces no effect.

#### Scenario: Persistent grant loses authority context
- **WHEN** a saved always grant is reopened after its reviewed manifest or effective MCP/A2A route changes
- **THEN** it no longer authorizes a call, and no raw command, argument or credential is needed in the grant record.

#### Scenario: No foreground decision is available
- **WHEN** an ask decision occurs in direct CLI, unattended A2A, dreaming or a closed/cancelled foreground interaction
- **THEN** it settles promptly as a typed permission-required refusal with no tool, process, file or network effect.
