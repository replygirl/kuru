## Why

In review, `memory-fixture-budget-model` was accepted except for how `fixture_deadline` combines budgets. It added every product bound a fixture could reach, so deadlines grew to 1548s. The lead adopted the reviewer's single-stall model instead. A fixture's backstop only has to outlast the Dolt server starts it really performs, plus the one product step that stalls. That lets the stalled step report its own error first.

Separately, the runtime warm-up timed out at exactly `LOCK_TIMEOUT`. It could race the product's own cache-lock error, and it did not cover the installer's extraction and version probe.

## What Changes

- `packages/kuru-memory/src/test_support.rs`: `fixture_deadline(fresh, reopened)` changes to the single-stall model:
  - Each Dolt server start gets the full `server_start_budget()`. A fresh open is four starts and a reopen is one.
  - The fixture gets one single-stall term, the largest of `close_budget()`, `QUERY_TIMEOUT` and the default startup timeout.
  - The fixture gets one `QUERY_TIMEOUT` for its own operations.
  - Call sites keep adding `SERVICE_IDLE_TIMEOUT` for owners that retire only through idle expiry.
  - With default budgets: (1,0) = 190s, (1,1) = 222s, (2,1) = 350s, (2,4) = 446s.
  - The doc comment describes this model.
- `fresh_open_budget()` keeps the whole-open model (lock, four starts with query sessions, three closes, quiescence). The crashed-owner readiness wait keeps using it; that wait watches one uncapped whole open.
- `warm_runtime_cache()` times out at `LOCK_TIMEOUT` + `VERSION_TIMEOUT` + a named margin. The product's own lock error then wins, and the extraction and version probe are covered. `VERSION_TIMEOUT` becomes `pub(crate)`; its value does not change.

## Impact

Test helpers only. Call sites and their fresh/reopen counts are unchanged. Backstops shrink to the single-stall bound. Nextest's terminate-after, when adopted, must sit above the largest derived bound: 446s, plus 30s where an owner retires through idle expiry.
