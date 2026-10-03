# Verification

## 1. A failed model listing is retried, not cached [critical]

- [x] 1.1 @regression (agent) `mise run //packages/kuru-runtime:test -- model_catalog_tests` before the fix -> exit 101: `failed_model_catalog_is_retried_and_warned_once_per_session` FAILED at the second lookup's `models()` counter assertion (`left: 1, right: 2`): the empty catalog from the first failure was cached and never listed again
- [x] 1.2 @unit (agent) same test after the fix: two failing lookups return fallback metadata without error, the third lookup returns the advertised context window, the listing is then cached (`models()` called exactly 3 times) -> exit 0, `1 passed; 223 filtered out`

- [x] 1.3 @regression (agent) `concurrent_lookups_share_one_slow_failed_listing`: four concurrent lookups against a provider whose `models()` sleeps 200 ms then fails make one `models()` call, all return fallback metadata, a following lookup inside the 30 s window makes none; after the window, four concurrent lookups make one successful call and see the advertised context window -> passes. Mutation (retry gate disabled) -> FAILED `concurrent lookups make one listing per window` (`left: 2, right: 1`): the queued caller re-ran the listing; source restored. The retry-delay elapse is simulated by moving the recorded retry instant to now, not by sleeping
- [x] 1.4 @unit (agent) `failed_model_catalog_is_retried_and_warned_once_per_session` updated for the cadence: a lookup inside the window does not list; after each window a listing is retried (fail, then succeed, `models()` called 3 times in total) -> passes; with the gate disabled it FAILED `a lookup inside the retry delay must not list` (`left: 2, right: 1`)

## 2. The failure is warned once per harness

- [x] 2.1 @unit (agent) the same test asserts exactly one `model catalog unavailable` record for its model, carrying the first failure's text -> passes; the process-wide recorder observed one record containing `catalog endpoint unavailable (attempt 1)`
- [x] 2.2 @unit (agent) mutation: warn on every failure, rerun the test -> FAILED as expected with two records (`attempt 1`, `attempt 2`; `left: 2, right: 1`); source restored

## 3. Repository checks

- [x] 3.1 @integration (agent) `mise run //packages/kuru-runtime:test` (full) -> exit 0, `225 passed; 0 failed` (rerun after the retry gate)
- [x] 3.2 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run cospec -- validate --all --strict` -> rerun after the retry gate: format:check exit 0, lint exit 0, lint:windows exit 0, typecheck exit 0, validate --all --strict exit 0
