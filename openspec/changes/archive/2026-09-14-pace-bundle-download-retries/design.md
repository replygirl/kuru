## Context

Final-head Intel jobs twice ended in HTTP 504 while preparing a pinned Dolt
archive. The later diagnostic proves the helper reached the third allowed GET
in about 13 seconds and its final 504 had no `Retry-After`, so the short existing
pauses did not use the unchanged 120-second acquisition budget to span a brief
upstream outage.

## Goals / Non-Goals

**Goals:**

- Space the existing retries by five and fifteen seconds.
- Prove production timing once while keeping the complete real-HTTP fixture
  suite fast through a private explicit-policy seam.

**Non-Goals:**

- More attempts, a longer total or lock deadline, another retryable failure
  class, workflow caching, or a generic retry framework.

## Decisions

Define the production retry delays once as `[5s, 15s]`. Keep
`prepare_asset(...)` production-shaped and delegate to a private
`prepare_asset_with_policy(...)` that accepts the existing total budget and an
explicit retry-delay slice. `download(...)` consumes that slice instead of
constructing delays. Tests call that private core with injected budgets and
delays; the production entry remains fixed to the production constants, no
production function contains conditional test behavior, and no API becomes
public.

Most loopback fixtures use `[25ms, 50ms]`. Deadline and cancellation fixtures
use deliberate local schedules that preserve their timing invariant. One
500-then-success immutable-GET test calls real `prepare_asset(...)`, observes at
least the five-second first pause within a bounded outer wait, counts exactly
two identical requests, and verifies the published bytes. No constant-only
arithmetic test substitutes for observed time.

## Integration contract

The helper continues to repeat only the exact manifest-owned immutable GET.
The same three-attempt maximum, 120-second total download deadline, 180-second
stable-lock bound, 15-second connect and 30-second read-idle bounds, status and
transport classification, `Retry-After`, size, digest, private staging, and
publication rules apply.

## Risks / Trade-offs

Persistent failure now takes twenty additional seconds before the third result,
but remains within the preexisting total budget. Focused tests inject only their
delay schedule through the private core, while the single real-delay case guards
against accidentally shipping the fast test schedule.
