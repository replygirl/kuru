# Proposal

## Why

Both actor model calls wrapped `collect_completion` in a flat
`tokio::time::timeout(Duration::from_secs(180), …)`
(`packages/kuru-runtime/src/actor.rs`, turn and context compaction), so a
completion that was still delivering deltas failed with "model call exceeded
180 seconds" (or "context compaction model call exceeded 180 seconds") after
three minutes. `collect_completion` forwards every `ProviderEvent` to the
actor's observer inside the timed future, so progress was observable and the
bound could have followed it; and docs/protocols.md promises a 600-second
completion budget that the actor path could never reach. High-effort
reasoning completions are the ones that cross three minutes, so the failure
hit exactly the long calls the 600-second budget exists for.

Behind that, the connector's SSE idle bound borrowed `crate::IO_TIMEOUT`
(60 s), a constant defined for ordinary HTTP I/O. Once the flat 180 s is gone
it becomes the binding silence guard, and it has no derivation: a reasoning
model may hold the stream silent for longer than 60 s between events. Found by
the fixed-wait audit of 2026-10-02, §2a ranks 1 and 3 (unit U1).

## What Changes

- `kuru-connectors` exports its existing private `COMPLETION_TIMEOUT`
  (600 s, unchanged) and adds `STREAM_IDLE_TIMEOUT` (300 s), the stream
  silence budget. The SSE reader in `ResponsesProvider::stream` uses it in
  place of `IO_TIMEOUT`; `IO_TIMEOUT` keeps its catalog, rotation and other
  HTTP uses.
- Derivation of `STREAM_IDLE_TIMEOUT`, documented on the constant and in
  docs/protocols.md: the Responses streaming-events reference
  (developers.openai.com/api/reference/resources/responses/streaming-events,
  read 2026-10-03; no version string, served last-modified
  2026-10-03 01:53 GMT) documents no keepalive, heartbeat or inter-event
  bound for HTTP SSE. The only cadence it states is
  `response.compaction.compacting` "at most once every 30 seconds" during a
  compaction trigger, which is not a heartbeat. The background-mode guide
  states reasoning models "can take several minutes" on complex problems, so
  silence between events is expected. The vendor's own Responses client
  documents `model_providers.<id>.stream_idle_timeout_ms` with a default of
  300000 ms (Codex configuration reference,
  developers.openai.com/codex/config-file/config-reference, read 2026-10-03).
  Kuru adopts that client default as its product silence budget; it is not a
  server guarantee. This cites documentation only; Kuru embeds no Codex
  component.
- One actor helper, `bounded_completion(provider, request, observer, call)`,
  replaces the flat 180 s at both sites and stays inside
  `work.cancellation.wait`. It wraps the actor's observer in a forwarding
  `ProviderSink` adapter that passes every event through unchanged (the
  accounting and progress observers see identical events) and records the
  last-progress instant on receipt and again after forwarding returns, so the
  observer's own accounting work is not counted as provider silence. The
  completion future runs in a biased `select!` against a watcher that sleeps
  until last progress plus the window; when it elapses the call fails with
  `"{call}: no provider progress for 300 s"`. The whole call is bounded by
  `COMPLETION_TIMEOUT` and fails with `"{call} exceeded the 600 s completion
  budget"` (numbers rendered from the constants). A fake or non-HTTP provider
  bypasses reqwest's total, so the actor enforces it itself. A provider error,
  including `ContextTooLarge`, is returned unchanged so the turn's fit-retry
  still recognizes it.
- The actor's no-progress window is `STREAM_IDLE_TIMEOUT` itself, one silence
  number, chosen equal rather than greater. On real wire silence the two
  bounds expire together: the connector re-arms its read after forwarding a
  chunk's events, which is when the adapter last records progress, and both
  messages name a silence. The actor restarts only on forwarded events, so a
  stream carrying nothing but unforwarded lifecycle frames (for example
  `response.in_progress` or `response.output_item.added`) for a whole window
  fails at the actor while the wire is open. The Responses reference documents
  no frame that repeats during silent reasoning, so there is no derivation for
  a larger margin; the actor window also bounds providers that have no stream
  idle bound of their own (a fake or non-HTTP provider).
- No 600 or 180 literal remains in actor.rs; no new flat total is added.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-tools`: the spec was incomplete. Its requirement that the
  completion deadline remain 600 seconds was right, but nothing required the
  runtime to let a progressing completion reach it, and no requirement stated
  a stream silence bound. An added requirement states both bounds, their
  messages and the unchanged forwarding of events.

## Impact

- `packages/kuru-connectors/src/lib.rs`, `providers.rs`: two public
  constants (`COMPLETION_TIMEOUT` newly exported, `STREAM_IDLE_TIMEOUT` new);
  SSE idle bound 60 s → 300 s.
- `packages/kuru-runtime/src/actor.rs`: shared helper at both call sites;
  failure texts change from "… exceeded 180 seconds" to the no-progress and
  total-budget messages (no test or document asserted the old texts).
- `packages/kuru-runtime/src/completion_window_tests.rs` (new) and `lib.rs`.
- `docs/protocols.md`: silence budget and its derivation beside the
  600-second budget.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
