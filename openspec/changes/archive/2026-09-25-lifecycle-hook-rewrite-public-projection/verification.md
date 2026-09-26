## Evidence environment

Local evidence comes from macOS arm64 on 2026-09-25. It was produced in the #89 worktree with that worktree's own `target/`, with `CARGO_TARGET_DIR` set explicitly to it. The package test environment was `RUST_TEST_THREADS=2` and `KURU_TEST_SUPERVISOR_PREPARED=1`.

Focused runs:
- `cargo test -p kuru-runtime --lib -- hook_platform_tests hook_tests accounting_tests permission_tests`: 65/65 passed. The first run had one failure, `hook_tests::parallel_post_hooks_settle_independently_but_rejoin_in_original_call_order` (see Notes); the rerun of the full group passed 65/65.
- `cargo test -p kuru-runtime --lib -- context compaction omit public session fork`: 50/50 passed.
- `cargo test -p kuru-connectors --lib hooks::`: 14/14 passed.

Static checks:
- `cargo fmt --all --check` is clean.
- `cargo clippy -p kuru-connectors -p kuru-runtime --all-targets --all-features -- -D warnings` is clean.
- `mise run docs:check` passed.

## 1. Rewritten input replaces the original in every provider projection [critical]

- [x] 1.1 @regression (agent) `hook_platform_tests::pre_turn_rewrite_reaches_the_provider_without_its_durable_hook_provenance` rewrites a turn targeted at part 0 (only that input is rewritten). It then runs a later turn targeted at part 1, compacts part 0, resumes the source session and runs a turn, and runs a turn in a fork of the source head -> passed. No request's messages or instructions contain `platform original`. The later, resumed and forked requests all carry `platform rewrite` in their instructions. The turn-scoped record holds `input: platform rewrite` and no original. `history()` of the source session and the fork's inherited public record keep the original. Before/after proof: with the public-projection substitution disabled, the test failed in two runs. The first failed at `hook_platform_tests.rs:234` ("later turn did not project the rewritten input"). The second, with that positive check removed, failed at the new no-original instructions assertion (`hook_platform_tests.rs:239` in that temporary copy, `:244` in the committed file). Both temporary edits were reverted, with a grep count of 0.
- [x] 1.2 @regression (agent) `hook_tests::rewritten_safe_retry_uses_its_own_public_turn_after_marker_or_later_answer` now also asserts that the retry's instructions carry no original input -> passed. The retry's context holds this turn's interrupted primary record. With the substitution disabled, it failed at `hook_tests.rs:2017`.
- [x] 1.3 @integration (agent) `hook_tests::inspection_skips_hooks_while_runtime_rewrite_preserves_the_durable_input_and_replay` and the remaining `hook_tests` without a pre-turn rewrite -> passed; behavior without a rewrite is unchanged

## 2. Omission loop and cleanup bound

- [x] 2.1 @integration (agent) `hook_tests::post_tool_annotation_stays_with_its_actor_and_can_be_omitted_by_context_fit`, `accounting_tests` compaction and context-fit cases, and the `context`/`compaction`/`omit` filters -> passed. Provenance is now filtered once at private-context read time and never counted as omitted.
- [x] 2.2 @regression (agent) `hooks::tests::cancellation_after_root_exit_stops_the_held_output_drain` -> passed. The test now gives the hook a 30,000 ms deadline, lets the worker enter the post-exit drain, and asserts that `quiesce` finishes within `CLEANUP / 2` of cancellation. With the drain's caller-loss check disabled, it failed with "held output drain outlived caller loss for 4.789648083s". The temporary edit was reverted, with a grep count of 0.

## 3. Native platforms and live model

- [~] 3.1 @integration (agent) native Windows and Linux runtime suites -> defer: not pushed, per instruction. The PowerShell branch of the platform test's fake hook runs only on Windows CI.
- [~] 3.2 @eval (agent) live provider turn with a rewriting pre-turn hook -> defer: the provider-visible content is determined by the runtime projection that rows 1.1 and 1.2 pin. A live evaluation needs the user's participation.

## Notes

- **Mechanism.** The lead suggested reusing the kuru-hook-linked private row. That row is keyed by actor namespace and session, while the public chain is keyed by the origin session and turn and is projected to every actor. An actor outside a targeted turn has no such row, and forks keep private rows on the parent session. A turn-scoped record keyed by the primary public node therefore stores the rewritten input only (no original), before dispatch, in the existing state store. It needs no schema migration.
- **Retry edge cases.** A retry of the same turn whose hooks rewrite again replaces the record with the newest rewrite. A retry that no longer rewrites leaves the earlier record, so later projections show that earlier rewritten text. Neither case projects the original.
- **Flake (corrected).** `parallel_post_hooks_settle_independently_but_rejoin_in_original_call_order` failed once at `hook_tests.rs:1146` under the concurrent group run. This record first attributed that failure to something outside this change. The attribution was wrong. The test came from 2dca3016 in this PR, and its settle-order assertion depended on the completion order of the parallel wave, which the fixture did not control (3 failures in 20 isolated runs). The fixture is made deterministic in the follow-up `lifecycle-hook-rewrite-retry-tombstone` change.
