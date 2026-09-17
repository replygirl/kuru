## 1. One frame carries the session's operating facts [critical]

- [x] 1.1 @regression (agent) `cargo test -p kuru --test terminal -- real_pty_status_bar_holds_all_six_session_facts_at_80_and_120_columns` -> 1 passed, 0 failed; each completed real-PTY frame at 120x35 and 80x24 carries `◇ demo F2  ~ default F3  ◌ IFS F4`, `◆ ctx ≈2808+8192/128000 assumed  ·  cost unknown` and `◈ Permissions · 0 session · 0 always  F5`. Against the unmodified tree the same assertions fail on the missing cost token, since no dock row rendered a cost figure at all.
- [x] 1.2 @e2e (agent) same test's `/cost` and `/permissions` assertions -> the frame's `cost unknown` token co-occurs with `/cost`'s own `API-standard estimate: unknown` and `...charge): unknown` lines, no `$` appears in the dock while the price is unknown, and the dock still reads `0 session · 0 always` in the same frame as the open inspector's `No session or always grants.`
- [x] 1.3 @e2e (agent) `cargo test -p kuru --test terminal -- real_pty_cost_inspection_survives_120_80_and_40_columns` -> 1 passed, 0 failed; the 40-column case still finds its unknown price and assumed bound with every control rendered.
- [x] 1.4 @manual (agent) capture the rendered dock at 120, 80 and 40 columns from the real PTY -> 120 and 80: `◇ demo F2  ~ default F3  ◌ IFS F4` / `◆ ctx ≈2808+8192/128000 assumed  ·  cost unknown` / `◈ Permissions · 0 session · 0 always  F5`. 40: `◇ demo F2` / `~ default F3  ◌ IFS F4` / `◆ ctx 11k/128k~ cost unknown` / `◈ Permissions · 0 session · 0 alway…` — abbreviated, nothing dropped.

## 2. An incomplete estimate names what it left out

- [x] 2.1 @regression (agent) `mise run //packages/kuru-memory:test -- usage_ledger::tests` -> 9 passed, 0 failed; `cache-write rate` is the only named term when the tier applied, `long-context tier` is named and nothing is priced when the input count is missing, a complete subtotal names nothing, and a pre-ledger history gap names nothing. The `unapplied` assertions cannot compile against the unmodified tree.
- [x] 2.2 @unit (agent) `mise run //apps/kuru-tui:test` (includes `ui::tests::cost_projection_keeps_absent_components_and_old_history_incomplete`) -> exit 0; `/cost` prints `$0.000012 known subtotal (incomplete; not applied: long-context tier, cache-write rate)` and `API-equivalent estimate (not a subscription charge): unknown (incomplete; not applied: invocation price)`, and the same test asserts no `$0 ` figure appears.
- [x] 2.3 @integration (agent) `mise run //packages/kuru-runtime:test -- accounting` -> 12 passed, 0 failed, including the real-Dolt exact-retry, abandoned-dream and pre-ledger cases.

## 3. An unknown configuration key states the documented policy

- [x] 3.1 @regression (agent) `cargo test -p kuru-core --test config_schema` -> 5 passed, 0 failed; unknown root, memory, MCP and permission-rule keys all carry the documented sentence, the key itself is never echoed, and `assumed_context_window_tokens=0` keeps its own message. Against the unmodified tree the rejection text is `configuration type error in …`, so the new test fails.
- [x] 3.2 @manual (agent) run the real binary against a project whose `.kuru/config.toml` has an unknown key -> `Error: configuration key error in /private/tmp/kuru-p1-fix38-probe/.kuru/config.toml: unknown keys are rejected: a configuration that needs a new key requires a newer Kuru version and never silently changes authority`

## 4. The repository's own gates

- [x] 4.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run typecheck` -> all clean after `mise run format:fix`
- [x] 4.2 @integration (agent) `mise run //apps/kuru-tui:test` and `mise run //packages/kuru-core:test` -> both exit 0; kuru-core reports 23+8+8+6+5+4+4 passed, 0 failed
- [x] 4.3 @integration (agent) `mise run //apps/kuru-docs:check` and `mise run cospec:validate` -> docs artifacts, local links and anchors passed; cospec validation passed with 0 errors, 0 warnings
- [~] 4.4 @runtime (agent) `mise run coverage` -> defer: the change's owner excluded the combined coverage aggregate from this task; the 90% gate is enforced by CI on the branch
