## Evidence environment

Local evidence comes from macOS arm64 on 2026-09-25. It was produced in the #89 worktree with that worktree's own `target/`, with `CARGO_TARGET_DIR` set explicitly. The environment was `RUST_TEST_THREADS=2` and `KURU_TEST_SUPERVISOR_PREPARED=1`.

Focused runs:
- `cargo test -p kuru-runtime --lib -- hook_platform_tests hook_tests accounting_tests permission_tests`: 66/66 passed.
- `cargo test -p kuru-runtime --lib -- context compaction omit public session fork`: 50/50 passed.

Static checks:
- `cargo fmt --all --check` is clean.
- `cargo clippy -p kuru-connectors -p kuru-runtime --all-targets --all-features -- -D warnings` is clean.
- `mise run docs:check` passed.

Deviation: the code was drafted before these artifacts were written. `validate --strict` and the apply gate (exit 0) then ran before the recorded evidence and the commit.

## 1. Retry projects what its latest attempt sent [critical]

- [x] 1.1 @regression (agent) `hook_tests::retry_that_no_longer_rewrites_projects_the_original_input_again` -> passed. The first attempt is rewritten and then stopped by cancellation, triggered on its `pre_turn rewritten` hook event, before dispatch: no provider request is made, and the record holds `input: stale rewrite`. The retry's hook allows the input, so the retry sends `user: stale original` and the record becomes `cleared: true`. The later turn's instructions contain `stale original`, and no request carries `stale rewrite`. With the tombstone write disabled, it failed at `hook_tests.rs:2137` (the record was still not cleared). With that assertion also removed, it failed at the later-turn projection assertion (`:2160` in the committed file). The temporary edits were reverted, with grep counts of 0 and 1.
- [x] 1.2 @integration (agent) existing rewrite leak tests `hook_platform_tests::pre_turn_rewrite_reaches_the_provider_without_its_durable_hook_provenance`, `hook_tests::rewritten_safe_retry_uses_its_own_public_turn_after_marker_or_later_answer` and `hook_tests::inspection_skips_hooks_while_runtime_rewrite_preserves_the_durable_input_and_replay` -> passed within the 66/66 run

## 2. Deterministic parallel settle order

- [x] 2.1 @regression (agent) `hook_tests::parallel_post_hooks_settle_independently_but_rejoin_in_original_call_order` run 30 times in isolation (`--exact`, separate processes) -> 30/30 passed. The first call's post hook is now released by a subscriber only after the runtime's `ToolSettled` event for `parallel-second`. The re-review measured the pre-fix fixture at 3 failures in 20 isolated runs; that before-state was not re-measured here.

## 3. Native platforms

- [~] 3.1 @integration (agent) native Windows and Linux suites -> defer: not pushed, per instruction
- [~] 3.2 @eval (agent) live provider turn with a retried rewriting pre-turn hook -> defer: needs user participation. The provider-visible content is pinned by row 1.1.

## Notes

- **Tombstone scope.** Only a re-admitted turn (journal `Resumed`) that is not rewritten writes the tombstone. A first attempt cannot have an earlier record, and a turn that may have been dispatched is never re-admitted. Ordinary turns therefore do no extra reads or writes.
- **Deferred follow-up (lead).** A batched multi-key read to replace the public window's up-to-16 point `get`s per actor request is not part of this PR.
- **Corrected attribution.** The archived `lifecycle-hook-rewrite-public-projection` verification Notes now attribute the parallel post-hook flake to this PR (2dca3016).
