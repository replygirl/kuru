## ADDED Requirements

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
