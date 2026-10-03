# Proposal

## Why

Native OpenAI login and token refresh discard the transport error at three
sites in `packages/kuru-connectors/src/auth/http.rs`: the authentication
response body read in `json_response` (a `response.chunk()` loop, not
`bytes()`), the send in `post_json` and the send in `exchange`. Each uses
`map_err(|_| ...)`, so a login blocked by a firewall or proxy (connection
refused) reads exactly like a server that never answered: "OpenAI
authentication request failed". Login is the first thing a new user does, and
the crate already classifies `is_timeout()` / `is_connect()` for provider
transport in `providers/diagnostics.rs`. This is fixed-wait audit unit U3,
§3B rank 1.

The completion stream diagnostics `stream_failed`, `stream_protocol` and
`stream_event` in `providers/diagnostics.rs` hard-code "ChatGPT" and
"subscription". The API-key `responses` route uses the same `sse::response`
decoder, so an `OPENAI_API_KEY` user whose stream reports `insufficient_quota`
is told to fix "a subscription quota limit" they do not have, and every stream
failure on that route names the wrong product. This is §3I rank 4.

## What Changes

- `TransportKind` gains a `pub(crate)` constructor
  `TransportKind::classify(&reqwest::Error)` (timeout, then connect, else
  other). The provider `transport()` helper now calls it instead of its own
  inline branch; its messages are unchanged.
- Authentication formats its own messages from that kind only, never the
  reqwest error's `Display` (it can carry a URL) and never a body:
  - `post_json` send: "OpenAI authentication request timed out" /
    "... could not connect" / "... failed";
  - `exchange` send: "OpenAI token exchange request timed out" /
    "... could not connect" / "... failed";
  - `json_response` body read: "authentication response read timed out" /
    "... could not connect" / "... failed".
  The "failed" forms are byte-identical to today's messages. No
  authentication variant is added to the provider `Operation` enum: login
  stays separate from provider diagnostics. The serde parse `map_err` in
  `json_response` ("invalid authentication JSON response") is not transport
  and keeps its message. The refresh path's own `is_connect()` replay
  decision in `PreparedRefresh::send` is untouched.
- `stream_failed`, `stream_protocol` and `stream_event` take the `Operation`
  and phrase messages as `{operation} ...`. `stream_event` branches the
  quota/usage/overload codes on `Operation::is_responses()`, the same predicate
  `status_message` and `rejected` now share: on Responses, `insufficient_quota`
  reads "is blocked by an API quota or billing limit", `usage_not_included`
  reads "usage is not included in the API account's plan or billing" and
  `server_is_overloaded` reads "Responses completion service is overloaded".
  Every ChatGPT-route message stays byte-identical, including "ChatGPT
  subscription does not include this usage" and "ChatGPT service is
  overloaded". The `codex` and `responses` routes remain separate.

Departures from the audit's file list, stated explicitly:

- `providers/sse.rs` changes mechanically to pass `operation`. Of its twelve
  helper call sites only the two in `response()` have `operation` in scope;
  the other ten are `Decoder` methods, so `Decoder` gains an `operation` field
  and a `Decoder::new(operation)` constructor in place of `#[derive(Default)]`.
  `response()` constructs it with its existing `operation`; the unit tests
  construct `Decoder::new(Operation::ChatgptCompletion)`. No decoding logic
  changes, and the discarded `response.incomplete` / `error` reason at
  `sse.rs:556-573` (audit item 13) is untouched.
- `providers.rs` gains one line, `pub(crate) use diagnostics::TransportKind;`,
  because `diagnostics` is a private module and authentication must reach the
  shared classifier.

Not now: `sse.rs:47` "ChatGPT response is not an event stream" has the same
wording defect with `operation` in scope, but changing it is outside this
mechanical argument threading; it belongs with item 13's sse.rs work.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-connectors/src/providers/diagnostics.rs`: `TransportKind`
  visibility, derives and `classify`; `Operation::is_responses`; route-aware
  stream helpers; unit tests.
- `packages/kuru-connectors/src/providers/sse.rs`: `Decoder::new(operation)`
  and argument threading only; tests construct the decoder explicitly.
- `packages/kuru-connectors/src/providers.rs`: one `pub(crate) use`.
- `packages/kuru-connectors/src/auth/http.rs`: classified transport messages
  at the three sites.
- `packages/kuru-connectors/src/auth/tests.rs`: refused, stalled-send and
  stalled-body tests.
- No public API, dependency or documentation change: no user documentation
  quotes the old strings (only archived change records do, which are history).

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
