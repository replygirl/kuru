## Context

The existing direct Responses and native ChatGPT routes are intentionally
separate. Response-budget and diagnostics work already supplies their 60-second
catalog and 600-second completion deadlines, bounded SSE handling, and finite
redacted rejected-response classification. There is no shared accounting for a
provider resend, the existing subscription 401 rotation, refresh sends, or
backoff.

Retrying an explicit rejected response differs from replaying an ambiguous
transport result or accepted stream. A rotating refresh POST has an additional
durable-state boundary: once `refresh_pending` is published, caller loss cannot
permit the owner to abandon or invent credential state. The pinned
reqwest 0.13.5 / hyper-util 0.1.20 / hyper 1.11.1 HTTP/1 trace establishes one
narrow auth-local result: the audited `hyper-util::ErrorKind::Connect`
constructors arise in connection acquisition before `pooled.try_send_request`,
so an auth `reqwest::Error::is_connect()` may identify a token POST that was not
given to HTTP dispatch. Native direct and proxy fixtures must prove zero OAuth
POSTs. HTTP/1 `take_message` recovery is a separate internal request-future
behavior; auth receives no returned request object, so its final canceled/send
error remains ambiguous. Neither proxy CONNECT nor generic provider transport
is safe to replay.

## Goals / Non-Goals

**Goals:**

- Bound provider and refresh activity by a single operation budget while keeping
  useful retry behavior for explicit 429, 500, and 503 responses.
- Keep credential rotation durable, single-use, and reconciled beyond caller
  lifetime without replaying a possibly consumed token.
- Provide finite, redacted exhaustion outcomes and deterministic local evidence.

**Non-Goals:**

- Add a public retry setting, provider selection/fallback policy, tool replay,
  request idempotency promise, MCP/A2A retry, or login/device-flow retry.
- Treat an unaudited/generic `is_connect`, a proxy CONNECT, a timeout, or a
  received response as proof that an OAuth POST was absent.
- Change completion, `Provider`, configuration, credential-store, or routing
  public shapes.

## Decisions

1. **One private counter and absolute deadline.** A mutex-protected attempt state
   is shared by provider calls and the detached refresh owner, but never held
   across awaits. Four application sends cover provider and refresh calls; three
   provider sends, one logical rotation, two refresh sends, two delays, a
   30-second per-delay cap, and a 60-second aggregate delay cap make the mixed
   `503 → 401 → refresh → success` path fit exactly. Every request execution
   consumes a send slot; lower HTTP/1 recovery stays inside that request future.

2. **Rejected-status replay is provider-only.** The finite set is 429, 500, and
   503 before body/SSE acceptance. Responses quota/billing codes remain terminal
   on 429; subscription bodies do not receive API billing semantics. Other 5xx,
   408, 502, and 504 remain terminal until their contracts are separately
   established. No partial, malformed, accepted, or ambiguous result replays.

3. **Retry-After is a bounded minimum.** Read one value of at most 128 bytes.
   Accept delta seconds or canonical IMF-fixdate only; format parsed dates back
   to canonical IMF before accepting. A syntactically numeric `u64` overflow or
   unrepresentable duration is terminal instead of silently falling back. A
   valid hint is never clamped downward; if it cannot fit the remaining absolute
   deadline or caps, no replay occurs. Missing, invalid, duplicate, oversized,
   and obsolete-format values use seeded equal-jitter fallback: 500 milliseconds
   then one second, uniformly in `[base / 2, base]`. Seed once from existing OS
   entropy; entropy failure uses the upper bound. A valid header delay is the
   maximum of that jittered delay and the header minimum, and positive fractional
   date waits round up. Tests inject private seed and wall-clock values only.

4. **Refresh authorization is absolute at grant time.** The caller gives the
   owner a `std::time::Instant` when granting `RefreshAllowance`, capped by the
   caller's then-remaining operation time and the auth 60-second deadline. The
   owner checks that deadline and receiver closure before its first OAuth POST
   and each subsequent one; delayed scheduling cannot extend authorization.
   Once possible dispatch begins, only bounded response/publication reconciliation
   can outlive the caller.

5. **Safe refresh repeat is a narrow pinned transport contract.** Auth prepares
   a cloneable request before pending publication and disables redirects and
   reqwest policy retries. Only auth-local `reqwest::Error::is_connect`, backed
   by every audited `ErrorKind::Connect` constructor before
   `pooled.try_send_request`, maps to `TokenPostNotDispatched`; native fake-server
   tests must observe zero OAuth POSTs for it and count proxy CONNECT separately.
   HTTP/1's internal `take_message` recovery is not exposed to auth, and a final
   canceled/send error remains ambiguous. The worker may retry once while
   preserving a nonreclaimable provider replay slot. Other outcomes retain no
   replay and reconcile exact durable new-generation, pending, or unknown state.

   Auth builds and validates a cloneable request before setting pending, publishes
   then exact-rereads pending under its lease, and only then POSTs. The one logical
   rotation permission covers expiry, received 401, and durable newer-generation
   reuse; the latter uses no auth send. Every post-pending return reconciles an
   exact rollback, new generation, or pending record, otherwise reports unknown.
   The outer whole-refresh timeout is removed: phase deadlines and reconciliation
   govern the owner so caller loss cannot cancel durable post-dispatch work.

6. **Retry policy stays private and route-specific.** Provider and refresh
   clients set `reqwest::retry::never()` so future reqwest policy retries cannot
   expand Kuru's budget. The resolved current connector graph has HTTP/1 only;
   the implementation records the lower pooled-request distinction without
   treating socket/proxy attempts as application sends. Generic MCP/A2A clients
   remain untouched. `httpdate` 1.0.3, already locked transitively, becomes an
   exact direct dependency for canonical IMF-fixdate formatting and parsing.

## Integration contract

The external provider contract is an explicit rejected HTTP status and a
bounded `Retry-After` header, never remote error text. Responses API-key and
subscription requests keep their separate endpoints, headers, credentials,
account/session generation checks, and finite diagnostics. The native refresh
fixture uses fake credentials and local HTTP/1/proxy peers; it establishes
whether `/oauth/token` was received, not remote token idempotency. Native
Windows executes the same required zero-POST transport proof before merge;
unrun local Windows work is recorded as unchecked rather than substituted by a
macOS result.

## Risks / Trade-offs

- **A rejected 500/503 may already have caused remote work.** → The policy is a
  bounded client choice, not an idempotency guarantee; no tool runs before a
  complete validated provider result.
- **A narrow safe class could become invalid after dependency/feature change.**
  → Pin the source premise with HTTP/1 native zero-POST fixtures and reject any
  generic transport inference.
- **Detached owner work can outlive an actor.** → Absolute grant-time deadline,
  receiver-closure checks before dispatch, exclusive lease ownership, and exact
  record reconciliation bound it without dropping durable pending state.
- **Server retry hints can be hostile or malformed.** → Single bounded parsing,
  overflow rejection, finite jitter, aggregate caps, and fixed redacted errors.
