## 1. Plain `cargo test -p kuru` no longer red-flags a healthy build [critical]

- [x] 1.1 @regression (agent) with `apps/kuru-tui/Cargo.toml` reverted to its pre-fix state, run `cargo test -p kuru --test terminal -- real_pty_status_bar_renders_known_cost_from_priced_invocation` (no `--all-features`) -> FAILED: `real_pty_status_bar_renders_known_cost_from_priced_invocation`, screen shows `API-standard estimate: unknown (incomplete; not applied: invocation price)`, confirming the seam is unavailable without the feature.
- [x] 1.2 @regression (agent) with the fix applied, rerun the same exact command -> `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 19 filtered out`.
- [x] 1.3 @integration (agent) run `mise run //apps/kuru-tui:test -- real_pty_status_bar` (the CI-equivalent `--all-features` task) -> both `real_pty_status_bar_renders_known_cost_from_priced_invocation` and `real_pty_status_bar_holds_all_six_session_facts_at_80_and_120_columns` pass; full suite `Finished in 64.16s` with no failures.

## 2. Shipping binary is unaffected

- [x] 2.1 @unit (agent) run `cargo tree -e features -p kuru --no-dev-dependencies | grep -c test-support` -> `0`, confirming the dev-dependency feature does not reach the shipping binary's resolved feature set.

## 3. Repo checks stay green

- [x] 3.1 @unit (agent) run `mise run format:check` -> all formatting checks pass, no diffs.
- [x] 3.2 @unit (agent) run `mise run //apps/kuru-tui:lint` (`cargo clippy -p kuru --all-targets --all-features --locked -- -D warnings`) -> `Finished` with zero warnings/errors.
