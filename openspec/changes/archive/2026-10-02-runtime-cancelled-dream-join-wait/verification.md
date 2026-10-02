# Verification

## 1. The marker wait no longer races the pipeline on a flat number [critical]

- [x] 1.1 @regression (agent) run the paused-clock test where a synthetic operation progresses every two thirds of the derived gap bound and reaches its condition after more than 10 s, then temporarily replace the new wait with the old `timeout(10 s, poll)` shape -> new wait passes; the old shape fails with `Elapsed`; record both outcomes (the swap is not committed). Observed 2026-10-02 local macOS: `progressing_operation_beyond_a_flat_ten_seconds_reaches_its_event` passes; with the old shape swapped in it failed with "old flat 10 s wait expired on a progressing operation".
- [x] 1.2 @unit (agent) run the paused-clock stall test where the operation marks one step and then makes no progress -> the wait reports a stall only after one full gap bound of silence and names the last completed step. Observed: `silent_operation_stalls_after_one_gap_and_names_its_last_step` passes.
- [x] 1.3 @unit (agent) run the paused-clock early-finish test where the operation ends before its condition -> the wait returns the finished outcome instead of waiting out a bound. Observed: `operation_that_finishes_first_returns_its_outcome_without_waiting_a_bound` passes.

## 2. The dream-join wait no longer ties the product's own quiesce bound

- [x] 2.1 @unit (agent) assert that the derived gap bound for the cancelled-dream fixture is strictly greater than `HookHost::quiesce_bound()` -> confirms the join bound is derived and encloses the product wait. Observed: `dream_gap_bound_takes_the_larger_stated_budget_above_quiesce` passes and the fixture's assertion holds (30 s > 10 s).

## 3. No behavioral regression elsewhere in the package

- [x] 3.1 @integration (agent) run `mise run //packages/kuru-runtime:test -- --lib hook_tests::` and then `mise run //packages/kuru-runtime:test` -> all tests pass; record the pass counts. Observed: `hook_tests::` 22 passed (32.45 s); full suite 224 passed, 0 failed (262.88 s).
- [x] 3.2 @manual (agent) read the derivation comments in the committed diff -> each bound names its terms and sources (memory startup budget, hook timeout_ms, quiesce bound) rather than a bare literal. Observed: `dream_gap_bound` names `turn_admission_deadline()`, hook `timeout_ms` and `quiesce_bound()`; the fixture comment states the gap and its quiesce enclosure.

## 4. Observed on the PR's own CI run

- [x] 4.1 @e2e (agent) observe the fixed test pass on the macOS coverage partition under real CI load, the only environment the original failure was observed in -> local runs cannot reproduce that runner, so this waited on the PR's CI. Observed 2026-10-02: PR #181 head `6d97d1324215f84e769ef7c8949194a50084b235`, run `37048078012` — all four macOS coverage partitions pass (`native-tests (macos-latest) / Coverage partition (macos-latest, 1..4)`, jobs `110974757153`, `110974757268`, `110974757366`, `110974757199`), plus `native-tests (macos-latest) / Coverage merge (macos-latest)` (job `110981458186`) and `native-tests (macos-latest) / Require native coverage and installation checks` (job `110982009483`); all 66 PR checks report `pass` (4 `skipping`, the unaffected `dolt-windows-arm64` legs). This is the one real-CI observation this change record needed; it was not rerun — it is the PR's first and only CI run on this head.
