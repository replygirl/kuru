## 1. The live `codex` route completes a turn [critical]

- [x] 1.1 @runtime (agent) with the pre-fix decoder plus the new diagnostics trace, run one live `codex`-route turn in an isolated data dir: `kuru run "Reply with the single word: ready" --json --debug` -> `Error: provider errors: completed response omits streamed output` / `all peers failed to produce a contribution`; ring `trace-3.jsonl` shows 7 peers x 2 streamed items (`reasoning` text_len 0 at index 0, `message` text_len 5 at index 1), **0** terminal items, and 7 `kuru.actor` `"status":"error"` spans of 1.7-4.2 s.
- [x] 1.2 @runtime (agent) with the fix applied, rerun the same command against the same live project -> stdout JSON ends `{"kind":"response","actor":"e1567a83-468e-5927-a7f7-dcd081401411","detail":"[response completed]"}`; ring `trace-0.jsonl` shows 29 streamed items, 0 terminal items (the empty array again) and 16 `"status":"ok"` records with no `"status":"error"`.
- [x] 1.3 @runtime (agent) read the committed turn back from the live memory store with `kuru memory export` (no provider call) -> turn `408bb8af…4052` holds `"text": "ready"`, `"response_outcome": "text"`, `"input_tokens": 7784`, `"output_tokens": 189`, `"limited": false`; nine `assistant` records carry `{"type":"text","text":"ready"}`. Usage is captured and non-unknown.

## 2. The mirrored live shape is a regression fixture

- [x] 2.1 @regression (agent) splice `accepts_subscription_terminal_envelope_with_empty_output` and `terminal_output_owns_order_and_presence_but_never_drops_visible_text` onto the pre-fix `sse.rs` (`git checkout origin/main -- packages/kuru-connectors/src/providers/sse.rs`) and run `cargo test -p kuru-connectors --lib providers::sse` -> `test result: FAILED. 10 passed; 2 failed`, with `accepts_subscription_terminal_envelope_with_empty_output` panicking on `completed response omits streamed output` — the exact live error — and the adjacent case on `completed response disagrees with streamed output`.
- [x] 2.2 @unit (agent) restore the fix and rerun `cargo test -p kuru-connectors --lib providers::sse` -> `test result: ok. 12 passed; 0 failed`.

## 3. The honesty guarantees still hold

- [x] 3.1 @unit (agent) in `terminal_output_owns_order_and_presence_but_never_drops_visible_text`, a streamed `message` carrying `output_text` that the terminal output omits -> error containing `completed response omits streamed output`; a counterpart whose visible text is rewritten to `not ready` -> error containing `completed response disagrees with streamed output`.
- [x] 3.2 @unit (agent) same test: a reordered terminal array, and one that replaces the streamed `reasoning` item with a populated summary, both succeed and are retained exactly as the terminal array sent them; a terminal array that drops the reasoning item entirely succeeds -> `assert_eq!(result["output"], reordered)` and `json!([message])` hold.
- [x] 3.3 @unit (agent) an empty terminal array with nothing streamed stays an empty completion (`requires_authoritative_output_and_unique_terminal_item_ids`), and the existing delta-vs-terminal conflict, tool-fragment-type, duplicate-ID, budget and redaction tests are unchanged -> `mise run //packages/kuru-connectors:test` -> `test result: ok. 203 passed; 0 failed`.

## 4. Repo checks stay green

- [x] 4.1 @integration (agent) `mise run //packages/kuru-connectors:test` -> `203 passed; 0 failed`.
- [x] 4.2 @integration (agent) `mise run //packages/kuru-runtime:test -- stream` -> `1 passed; 0 failed; 140 filtered out`.
- [x] 4.3 @integration (agent) `mise run //apps/kuru-tui:test -- stream` -> `1 passed; 0 failed; 76 filtered out`; `mise run //apps/kuru-tui:test -- diagnostics` (the layer this change touches) -> `7 passed; 0 failed`, including `layer_rejects_foreign_and_unadmitted_secret_fields_across_rotation` and `normal_layer_keeps_parent_correlation_without_span_rows`.
- [x] 4.7 @unit (agent) added `diagnostics::tests::debug_level_stream_reconcile_event_is_gated_by_the_debug_flag`, pinning the 1.3 gate directly: a `kuru.provider` `responses-stream-reconcile` `tracing::debug!` event is dropped with `debug: false` and retained with all fields (`stage`, `item_index`, `item_id`, `item_type`, `text_len`) with `debug: true`. `mise run //apps/kuru-tui:test -- diagnostics` -> `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 70 filtered out`.
- [x] 4.4 @unit (agent) `mise run format:check` -> `Finished in 1.83s`, no diffs; `mise run //packages/kuru-connectors:lint` -> `Finished` with no warnings; `mise run typecheck` -> `Finished in 86.09s`.
- [x] 4.5 @unit (agent) `mise run docs:check` -> `Public docs artifacts, local links and anchors passed (/kuru/).`
- [~] 4.6 @integration (agent) full-workspace `mise run test` / `mise run coverage` -> defer: out of scope for this unit and explicitly excluded by its instructions; the three package suites above plus format, lint, typecheck and docs were run instead.
