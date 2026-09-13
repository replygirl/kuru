## 1. Private operation budget and provider retry

- [x] 1.1 Add private absolute operation budget, attempt counters, fixed-seed retry schedule, and request-template reuse in the existing catalog and completion paths; retain the existing 60-second catalog and 600-second completion deadlines without changing public provider/configuration shapes.
- [x] 1.2 Extend private rejected-response diagnostics to classify only route-correct 429, 500, and 503 retries; parse one bounded Retry-After delta-seconds or canonical IMF-fixdate minimum with overflow, cap, deadline, and fixed-redaction behavior.
- [x] 1.3 Explicitly configure provider and refresh clients with `reqwest::retry::never()` and add the exact direct `httpdate` 1.0.3 dependency edge without lockfile version drift or a generic MCP/A2A client change.
- [x] 1.4 Add local API-key JSON, subscription pre-acceptance SSE, and catalog regression fixtures that fail before the retry loop and pass with exact shared send/rotation/delay counts, stable route/session input, and terminal route-correct quota/access/model behavior.
- [x] 1.5 Add deterministic Retry-After and cancellation fixtures for canonical/invalid/overflow hints, aggregate and operation-deadline exhaustion, 500-millisecond/one-second equal jitter, entropy fallback, and cancellation during backoff; verify no later provider send or pending-state/public-output mutation.

## 2. No replay after uncertainty

- [x] 2.1 Preserve one-send behavior after ambiguous transport, accepted JSON/SSE response, malformed body, partial output, failed event, idle stream, cancellation, or decoder failure; add a raw local peer regression that reads a whole request then closes and fails before/after without replay.
- [x] 2.2 Verify fixed redacted exhaustion and terminal diagnostics contain no remote body/header/URL/token/raw error-chain values while preserving existing bounded diagnostic-reader and SSE size/deadline behavior.

## 3. Owned refresh allowance and reconciliation

- [x] 3.1 Refactor provider entry to snapshot subscription identity and API-key environment once without eager network rotation, then grant detached owners one shared RefreshAllowance with a caller-granted absolute `std::time::Instant`, account/session/generation, nonreclaimable provider replay reservation, shared logical-rotation permission, and at most two refresh sends.
- [x] 3.2 Build and clone the secret-bearing refresh request before exact pending publication; reread that pending record under the retained lease before POST. Evaluate the audited auth-local `reqwest::Error::is_connect` connection-acquisition class first, even when it also reports timeout or direct/proxy acquisition context, and map only that class to `TokenPostNotDispatched`; preserve redirect denial, classify every other timeout, canceled/send, proxy, CONNECT, or response outcome as possibly dispatched, and never infer an undispatched token POST from `is_timeout`, proxy/CONNECT observation, or the lower HTTP/1 layer's internal returned-request path that reqwest does not expose.
- [x] 3.3 Remove outer whole-refresh cancellation and retain the existing exclusive lease through phase deadlines plus exact rollback, new-generation, pending, or unknown reconciliation on every post-pending return; check receiver closure/deadline before every OAuth POST and never re-anchor authorization at worker start.
- [x] 3.4 Add fake-secret native HTTP/1 and proxy fixtures proving zero OAuth POST for the audited `is_connect` safe class, separate CONNECT counting, one allowed safe repeat, exact generation publication, terminal possibly-dispatched outcomes, caller-loss/expired-delayed-owner cleanup, and concurrent durable-newer-generation reuse under one rotation permission.
- [x] 3.5 Add required native Windows fixture coverage for the pinned zero-token-POST and proxy CONNECT cases; leave it explicitly unverified locally until Windows native CI supplies observed evidence.

## 4. Documentation and focused verification

- [x] 4.1 Update `docs/protocols.md` and the curated configuration reference with finite retry/Retry-After limits and no-replay/rotating-token limits, without claiming remote idempotency or exposing diagnostics.
- [x] 4.2 Run the focused owning provider, SSE, auth, format, lint, typecheck, and documentation checks through mise; record the baseline regression failure and post-fix observed evidence, with native Windows evidence honestly unrun locally.
