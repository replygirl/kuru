## 1. API-key completion deadline [critical]

- [x] 1.1 @regression (agent) run the focused API-key provider fixture with an injected short completion deadline -> observed `api_key_completion_uses_its_operation_deadline` accepts either the inner request-timeout or outer operation-deadline error at their shared local boundary; `api_key_completion_overrides_shorter_generic_client_deadline` accepts a 150 ms completion despite an 80 ms generic-client timeout and a 2-second operation limit, while `api_key_catalog_keeps_its_separate_http_deadline` accepts a 150 ms catalog response despite that short completion setting.

## 2. Bounded SSE decoding [critical]

- [x] 2.1 @regression (agent) send a local fragmented/chunked SSE response with more than 2 MiB of ignored comments/deltas followed by a small completion -> temporarily restoring only the original 2 MiB wire ceiling caused `discarded_fragmented_traffic_does_not_spend_retained_response_budget` to fail with `Responses stream exceeds wire size limit`; after restoring 64 MiB, the focused provider suite passed.
- [x] 2.2 @regression (agent) decode a valid one-line completed item larger than 64 KiB but below the retained response cap -> observed `accepts_large_single_line_completed_item_and_reconciles_final_envelope` pass with a 128 KiB item and matching final envelope.
- [x] 2.3 @unit (agent) decode retained final output over 2 MiB and wire or single-event payloads over their private limits -> observed `rejects_oversized_retained_wire_and_event_budgets` pass with bounded, redacted limit errors.
- [x] 2.4 @integration (agent) run existing failed/partial subscription stream fixtures -> observed `repeated_401_and_partial_stream_errors_do_not_retry_or_leak_tokens` and malformed-stream fixtures pass in the focused provider suite.

## 3. Scoped verification

- [x] 3.1 @integration (agent) run the focused `kuru-connectors` provider/SSE tests through its mise task -> `mise run //packages/kuru-connectors:test -- providers::` passed 23 tests, 0 failures after the restored 64 MiB wire ceiling; the subsequent `api_key_` deadline filter passed 4 tests, 0 failures.
- [x] 3.2 @integration (agent) run the single coordinated combined coverage and full formatting/static/docs checks after Phase 0 integration -> observed `mise run format:fix`, format check, Rust lint/typecheck, tooling lint, strict cospec with managed drift check, and docs check all exit 0; the one `mise run coverage` pass exits 0 in 265.58 seconds and writes `target/coverage.lcov` under the enforced 90-percent line gate.
