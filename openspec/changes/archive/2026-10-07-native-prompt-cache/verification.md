# Verification

## 1. Compatible native shared-prefix reuse [critical]

- [x] 1.1 @regression (agent) capture final HTTP requests for changing actors and rounds -> identical plain common developer block and routing key; mutable suffix and private inputs outside it; original combined serializer fails
- [x] 1.2 @integration (agent) exercise native continuation, key grouping, legacy/API/custom routes, invalid boundaries and final-body sizing -> one developer prefix, correct opaque ranges/attribution, guarded metadata, unchanged non-native shapes
- [x] 1.3 @eval (agent) run bounded ordinary synthetic native harness with Kuru's own login -> complete fixture passes with positive cached input on a later distinct actor sharing prefix and offered tools; observed facing session/actor/turn summaries have private records; opportunistic/model limits retained

## 2. Runtime and documentation

- [x] 2.1 @integration (agent) exercise runtime request construction across parts and mutable public context -> exact boundary excludes identity, phase, topology and private material
- [x] 2.2 @unit (agent) run relevant static and documentation checks -> affected Rust passes format, typecheck and lint; docs build and content checks pass

## Observed evidence — 2026-10-07

The actual HTTP capture regression `native_common_instruction_blocks_and_cache_key_match_across_actors_and_rounds` failed with the original `prepare_responses_request` implementation: actor-local instructions still changed the top-level field. Restoring this change passed the regression as part of the 90/90 provider tests. Those tests also passed continuation isolation, opaque-range shifting, final-body attribution, invalid UTF-8 boundaries, legacy/API/custom shapes and common/tools/model/effort key grouping, including reordered offered tools. The four-mode runtime boundary regression passed 1/1 with changing actors and public context.

The ignored `native_live_acceptance` test passed 1/1 in 19.16 seconds through Kuru's own native authentication and ordinary Harness, with advertised GPT-5.5 and low effort. Three preplanned A/B/A turns produced six completed streams. Actual provider-reported input/cached-input pairs were 3296/0, 3893/0, 3348/2560, 3950/0, 3786/2560 and 4390/3584; total reported input was 22663, cached input 8704, output 121 and reasoning 12. The assertion matched a positive later completion to a prior distinct actor with the same runtime common prefix and actually offered tools. Phase inventories remained distinct and no tools were called.

The synthetic common catalog has 64 entries; no production padding, prewarm, conditional retry, budget increase or cache-control fields were added. Both estimated input and the connector's final-body `ContextMeasured` estimate totaled 62431 under the unchanged 64000 limit; these local estimates are separate from provider-reported usage. The six-stream, per-request 16000-input, 16KiB-output and 180-second limits remain. Private-context exclusion, exact resumed private records, public replay, terminal restoration, pre-turn rewrite and three post-turn annotations passed. This run supplied no reasoning summaries, so it establishes no new live summary or thinking proof; matching facing-summary records remain asserted when supplied. Cache reuse is observed for this advertised model and remains opportunistic. Earlier GPT-5.6-Luna zero-cache observations and native rejection of the API's explicit breakpoint remain evidence.

Documentation build and content/link checks passed. Final Rust/TOML/fixture and docs formatting and managed-file drift checks passed. Host lint, Windows-target lint and full typecheck passed on accepted N3 main `3b323c32` plus this scoped change. Clippy's single-range Vec warning was corrected only in the continuation test fixture; no runtime behavior or budget changed. Independent final source review found no scope, private-history or transport regression.
