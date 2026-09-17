## 1. Name unapplied priced terms

- [x] 1.1 Add `UnappliedPriceTerm` and a defaulted `MoneyEstimate.unapplied` field in `packages/kuru-core/src/accounting.rs`, and verify the stored record shape (`InvocationUsage`, `InvocationStart.price_at_invocation`) is untouched so no migration is required. `MoneyEstimate` is only produced by the read fold and consumed by the TUI; no persisted record embeds it, so no migration was needed.
- [x] 1.2 Record the term at each incompleteness in `EstimateFold` (`packages/kuru-memory/src/store/usage_ledger.rs`) without changing the arithmetic, and verify `reducer_marks_terminal_gaps_and_prices_known_normal_terms` and `reducer_keeps_complete_and_partial_frozen_price_subtotals_honest` assert the named terms, an empty set for a complete subtotal, an empty set for a pre-ledger history gap, and an undecidable long-context tier priced not at all. `mise run //packages/kuru-memory:test -- usage_ledger::tests` → 9 passed, 0 failed.
- [x] 1.3 Render the names in `format_session_usage` (`apps/kuru-tui/src/ui.rs`) and verify `cost_projection_keeps_absent_components_and_old_history_incomplete` pins the incomplete and unknown labels including their named terms. Covered by `mise run //apps/kuru-tui:test` → exit 0.

## 2. Stand the cost and context meters in the dock

- [x] 2.1 Add the meter row to `dock_controls` in `apps/kuru-tui/src/ui/render.rs`, sourcing cost from `View::usage` and context from `View::request_context`, reusing one window-provenance helper with `draw_status`, and verify the dock row count in `draw` matches the rows the composer renders at every width branch. `control_rows` raised from `1 +` to `2 +`; the three width branches still mirror `dock_controls`.
- [x] 2.2 Degrade the row at very narrow widths by abbreviating token counts and keeping the assumption marker, and verify the existing 40-column real-PTY case still passes with every control present. Captured at 40 columns: `◆ ctx 11k/128k~ cost unknown` above `◈ Permissions · 0 session · 0 alway…`, with the model, effort and mode chips still on their own rows.
- [x] 2.3 Add `real_pty_status_bar_holds_all_six_session_facts_at_80_and_120_columns` to `apps/kuru-tui/tests/terminal.rs`, and verify one completed PTY frame carries model, effort, mode, cost, context use and permission state, agrees with `/cost`, agrees with `/permissions` while its inspector is open, and shows no `$` when the price is unknown. Passes at 120x35 and 80x24 alongside the pre-existing 120/80/40 case.

## 3. Reject an unknown configuration key with the documented policy

- [x] 3.1 Carry the documented forward-compatibility sentence on unknown-key rejections in `packages/kuru-core/src/config.rs` without echoing the key, and verify an out-of-range value keeps its own message. `config_type_error` substitutes the policy only for a serde `unknown field` rejection.
- [x] 3.2 Add `unknown_keys_are_rejected_with_the_documented_forward_compatibility_message` to `packages/kuru-core/tests/config_schema.rs`, and verify it pins both the rejection text and the same sentence in `docs/configuration.md` and `apps/kuru-docs/reference/configuration.md`. `cargo test -p kuru-core --test config_schema` → 5 passed, 0 failed.

## 4. Documentation and verification

- [x] 4.1 Update `docs/interface.md`, `docs/usage.md` and `apps/kuru-docs/guide/first-conversation.md` for the standing meter row and the named unapplied terms, and verify `mise run //apps/kuru-docs:check` passes. Passed, including the local link and anchor check.
- [x] 4.2 Run format, lint, typecheck, the TUI and core suites, the targeted memory ledger tests, the runtime accounting tests and `mise run cospec:validate`, and record the observed results. All green; see verification.md. `mise run coverage` and `mise run check` were deliberately not run for this task.
