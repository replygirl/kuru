## 1. Real-PTY frame renders a known cost from a priced invocation [critical]

- [x] 1.1 @regression (agent) `cargo test -p kuru --test terminal --all-features -- real_pty_status_bar_renders_known_cost_from_priced_invocation --exact`, first with `packages/kuru-core/{Cargo.toml,src/model_catalog.rs}` and `apps/kuru-tui/Cargo.toml` reverted to their pre-change content (`git stash` of exactly those three files, test file and fixture kept) -> FAILS: dock shows `cost unknown`, `/cost` shows `API-standard estimate: unknown (incomplete; not applied: invocation price)`. Then with the change restored (`git stash pop`) -> PASSES, dock shows `≈$0.000144 est` and `/cost` shows `API-standard estimate: $0.000144 estimated` in the same frame.
- [x] 1.2 @e2e (agent) same test, full run at both 120x35 and 80x24 after resize -> PASSES at both widths; verbatim dock row at each: `120x35: "◆ ctx ≈2972+8192/128000 assumed  ·  ≈$0.000144 est"`, `80x24: "◆ ctx ≈2972+8192/128000 assumed  ·  ≈$0.000144 est"`.
- [x] 1.3 @unit (agent) `cargo test -p kuru-core --all-features` (existing `model_catalog.rs` test suite, including `malformed_catalogs_are_rejected_before_enrichment`) -> PASSES unchanged: 6 passed, 0 failed; confirms `from_json`'s public validation behavior is unchanged for the embedded catalog and for malformed input.

## 2. No regression in the existing real-PTY suite

- [x] 2.1 @e2e (agent) `cargo test -p kuru --test terminal --all-features --no-fail-fast` (full file, including the pre-existing `real_pty_cost_inspection_survives_120_80_and_40_columns` and `real_pty_status_bar_holds_all_six_session_facts_at_80_and_120_columns` cost-unknown tests) -> PASSES: 18 passed, 0 failed, 1 pre-existing ignored.
- [x] 2.2 @integration (agent) `cargo fmt --all -- --check` and `cargo clippy -p kuru --all-targets --all-features --locked -- -D warnings` -> PASS, no warnings.
