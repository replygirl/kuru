# Design

## Context

The actor's flat 180 s bound timed the whole model call, ignoring the
`ProviderEvent`s that `collect_completion` already forwards to the actor's
observer inside the timed future. The connector's own bounds were already the
right shape: reqwest's request timeout carries the 600 s total
(`COMPLETION_TIMEOUT`) and the SSE reader re-arms an idle timeout on each wire
chunk. The idle value, though, was `IO_TIMEOUT` (60 s), defined for ordinary
HTTP I/O with no stream derivation.

## Goals / Non-Goals

**Goals:**

- A completion that keeps making progress runs until the documented 600 s
  total, at both the turn and the context compaction site.
- One derived silence budget, shared by the connector's SSE bound and the
  actor's no-progress window.
- The actor enforces the total itself, because a provider outside the
  connector's HTTP client (a fake, or a future non-HTTP provider) has no
  reqwest deadline.

**Non-Goals:**

- Changing `COMPLETION_TIMEOUT`, retry, rotation or catalog bounds.
- New `ProviderEvent` variants or a liveness signal from unforwarded frames.
- Background-mode Responses or stream resumption.

## Decisions

- **One helper, adapter sink.** `bounded_completion` wraps the observer in a
  `ProgressWindow` sink that forwards each event unchanged and records progress
  before and after forwarding. Rejected: a re-arm hook in `AccountingObserver`,
  which would couple accounting to timing and miss the compaction path's
  observer shape.
- **Biased `select!` inside `timeout`.** The completion future is polled first,
  so an event that lands at the same instant as the window expiry wins.
  `timeout(COMPLETION_TIMEOUT, …)` maps elapse to the distinct total-budget
  message. Everything stays inside `work.cancellation.wait`, so cancellation
  still preempts both bounds.
- **Window equal to `STREAM_IDLE_TIMEOUT`.** Rejected: a larger actor window,
  because no documented frame repeats during silent reasoning, so any margin
  would be a guessed number.
- **300 s silence budget.** Rejected: keeping 60 s (`IO_TIMEOUT`, no stream
  derivation, fails silent reasoning) and dropping the idle bound (a hung
  stream would hold the turn for the full 600 s). The vendor documents no
  keepalive; its own Responses client defaults its SSE idle timeout to
  300000 ms. See the proposal for citations.

## Risks / Trade-offs

- [A stream that sends only unforwarded lifecycle frames for 300 s fails at
  the actor while the wire is open] → No documented Responses frame repeats
  during silent reasoning; if one appears, forward a progress observation
  rather than widening the window.
- [A truly hung connection now takes 300 s instead of 60 s to fail] →
  Cancellation still ends the call at once; 300 s matches the vendor client's
  default and is well inside the 600 s total.
- [Vendor documentation changes] → The derivation is written on the constant
  and in docs/protocols.md with the read date, so a later audit can re-check it.

## Integration contract

- **Ownership.** `kuru-connectors` owns both constants and the only vendor
  knowledge (`STREAM_IDLE_TIMEOUT` derivation). `kuru-runtime` consumes them
  through the crate's public exports and states no number of its own.
- **Wire shape.** Unchanged: the Responses SSE request, the accepted frames,
  terminal handling and the "Responses stream exceeded idle timeout" message
  are as before; only the idle duration changes from 60 s to 300 s on both the
  API-key (`responses`) and subscription (`codex`) routes.
- **Fixture shape.** Runtime tests use fake `Provider`s that sleep on tokio's
  paused clock and emit `TextDelta`/`Completed`; they never open sockets, so
  auto-advance cannot fire memory or HTTP timers. Existing connector live HTTP
  fixtures keep passing explicit idle durations to `sse::response`.
- **Schema / id reconciliation.** None; no identifiers, events or persisted
  records change.
