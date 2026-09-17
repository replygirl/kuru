## 1. Durable typed and legacy history [critical]

- [x] 1.1 @integration (agent) actual Dolt v1/v2 upgrade, v3 reopen and old candidate/revision read -> package-owned real-Dolt tests passed: `released_v2_json_looking_text_and_historical_views_survive_typed_v3`, `stopped_released_v1_upgrades_once_and_readonly_preserves_it`, `ready_released_v1_stage_activates_its_original_marker_then_upgrades`, and `retained_v2_attempts_and_candidate_survive_test_v4_progression`.
- [x] 1.2 @integration (agent) typed checkpoint reconciliation and import fixtures -> real-Dolt `failed_database_batch_rolls_back_and_committed_receipt_reconciles` passed; the instrumented CLI suite passed `cli_imports_a_real_legacy_wal_without_changing_its_layout` and project purge isolation/confirmation cases.
- [x] 1.3 @unit (agent) mixed-block codecs, malformed formats and absent/zero metadata -> `mise run //packages/kuru-core:test` passed all 36 tests; real-Dolt `committed_unknown_and_malformed_formats_refuse_history_and_export` additionally passed, including bounded errors that omit malformed private payloads.

## 2. Native tool protocol compatibility [critical]

- [x] 2.1 @integration (agent) native HTTP fixtures for both routes, typed receipts and actor-local opaque continuation -> package-owned connector tests passed for native Responses and subscription continuation, including consecutive distinct calls and retained opaque reasoning; explicit current-input boundary tests passed with omitted optional assistant history and interleaved peer input.
- [x] 2.2 @integration (agent) unsupported content and malformed/mismatched receipt fail before request -> `unsupported_typed_content_is_rejected_before_dispatch` and `validates_native_output_and_partial_tool_handshakes` passed, including malformed legacy text and mixed typed receipt shapes; all-message validation also checks unsupported earlier content alongside valid current receipts.
- [x] 2.3 @eval (agent) deterministic scripted provider/tool turns preserve visible answer, receipt identity/order and actor isolation -> both-route scripted HTTP continuations and runtime real-Dolt large/redacted receipt, dream receipt, and completed-turn retry fixtures passed; actor unit regressions cover encoded history bounds and preserved structured result values.

## 3. User-facing compatibility [critical]

- [x] 3.1 @e2e (agent) real CLI and PTY text conversation plus completed retry -> `mise run //apps/kuru-tui:test -- real_pty_accepts_chat_navigation_commands_and_restores_terminal` passed; focused runtime completed-turn exact retry and redacted receipt reopen tests passed with real Dolt.
- [x] 3.2 @e2e (agent) mixed legacy/typed JSON and Markdown memory export -> real-Dolt mixed-format export and CLI `memory_export_is_provider_free_and_publishes_one_committed_snapshot` passed; `mise run //apps/kuru-tui:test -- markdown_keeps_typed_content_structured_and_bumps_the_outer_format` passed with structured block payloads and the version-2 envelope.

## 4. Repository and delivery gates

- [x] 4.1 @integration (agent) format, lint, typecheck, docs, tooling, cospec and combined coverage -> all granular gates passed; corrected `mise run coverage` exited 0 with the existing 90% workspace line threshold, including all memory and runtime tests. Strict Cospec validation passed with zero errors and warnings; managed integration drift check passed.
- [x] 4.2 @equivalence (agent) independent review against typed-message specs and existing behavior -> independent Sol review covered the complete P1 diff and then reviewed the explicit current-input boundary correction; all reported receipt-matching and malformed-current-receipt findings are resolved, with no remaining concrete issue.

## Delivery after archive

The final pushed PR must pass the existing native CI graph before merge. Record
its exact SHA and run results in the private delivery evidence after execution;
the archived local ledger does not claim remote jobs that have not run yet.

## Observed local evidence

Focused memory commands use `mise run //packages/kuru-memory:test -- <filter>`.
Besides the cases above, the following passed: `export_keeps_legacy_and_typed_message_formats_distinct`,
`failed_database_batch_rolls_back_and_committed_receipt_reconciles`,
`production_upgrade_` (three lost-reply recovery cases),
`migration_attempt_rejection_preserves_exact_capacity`,
`absent_fast_forward_keeps_the_same_ready_attempt_for_next_open`,
`process_loss_after_accepted_ddl_retains_attempt_until_cold_recovery`, and
`cancelled_upgrade_call_retains_writer_through_accepted_ddl_boundaries`.
The two-step production migration exposed a test pause hook firing twice; the
fixture now arms it once and passes with its original ten-second bound.

The docs build and content/link checks passed. `mise run lint:rust`,
`mise run typecheck`, and `mise run format:code` passed on the final source.
`mise run lint:tooling` and `mise run cospec:managed:check` also passed.
The corrected combined coverage run exited 0 at 93.46% workspace line coverage
(37,796 of 40,440 lines), preserving the 90% threshold and writing
`target/coverage.lcov`; the full run took 522 seconds.

The first combined run passed every target except ten memory migration fixtures
with obsolete v2 assumptions. Their corrections preserve exact receipt,
ancestry, working-set and old-schema assertions, and passed independent review.
All ten then passed focused real-Dolt reruns using filters
`historical_and_out_of_order_attempts_fail_without_mutation`,
`migration_lifecycle_tests`, `manual_dolt_commit`,
`dropping_precommit_ddl_session_retains_dirty_working_ddl_outside_head`,
`isolated_schema_retry_keeps_main_clean_and_reconciles_lost_fast_forward_reply`,
`registry_definitions_and_reserved_names_are_bounded`,
`migration_validation_rejects_extra_schema_or_receipt_authority`, and
`ready_released_v1_stage_rejects_dirty_or_newer_attempts`.
The corrected memory package also passed lint. The initial combined run passed
selected-note forgetting and all project-purge fixtures.

Independent review found that historical receipts could be checked against a
newer pending native call. Both-route HTTP regression tests now cover consecutive
distinct calls. Cross-layer review additionally identified history omission and
interleaved peer input as reasons to carry an explicit transient current-input
boundary; the final correction and malformed-current-receipt refusal passed
focused tests and independent review.
