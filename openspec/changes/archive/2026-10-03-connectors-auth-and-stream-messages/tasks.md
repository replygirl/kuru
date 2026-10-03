# Tasks

## 1. Shared transport classification and route-aware stream messages

- [x] 1.1 Add `TransportKind::classify(&reqwest::Error)` (`pub(crate)`, re-exported from `providers`) and make `diagnostics::transport` reuse it, and verify the provider transport messages are unchanged
- [x] 1.2 Add `Operation::is_responses` and use it in `status_message`, `rejected` and `stream_event`, and verify the existing status tests pass
- [x] 1.3 Thread `Operation` into `stream_failed`, `stream_protocol` and `stream_event`, with Responses wording for quota, usage and overload codes and byte-identical ChatGPT wording, and verify every ChatGPT-route assertion in `sse.rs` and `subscription_tests.rs` still passes unchanged
- [x] 1.4 Give `Decoder` an `operation` field via `Decoder::new(operation)` and pass it at every helper call site in `sse.rs` with no other logic change, and verify `response()` passes its existing `operation`

## 2. Classified authentication transport messages

- [x] 2.1 Format the `post_json` send, `exchange` send and `json_response` body-read failures from `TransportKind` only, leaving the serde parse message unchanged, and verify no reqwest `Display` or body reaches the message

## 3. Regression tests

- [x] 3.1 Add auth tests for a refused connection (closed local port as issuer) and a stalled server (accepts, never responds) for `post_json` and `exchange`, and a stalled body for `json_response`, with an injected short `http_timeout`, and verify distinct messages that echo no URL, port or body, failing before the fix
- [x] 3.2 Add diagnostics unit tests on every stream code under both a Responses and a ChatGPT completion operation, and an sse decoder test under a Responses operation, and verify they fail before the fix

## 4. Checks

- [x] 4.1 Run `mise run //packages/kuru-connectors:test`, `format:check`, `lint`, root `lint:windows`, `typecheck`, `docs:check` and `cospec -- validate --all --strict`, and verify each exits 0
