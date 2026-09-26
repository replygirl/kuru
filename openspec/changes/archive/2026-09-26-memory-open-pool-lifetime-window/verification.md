## 1. Retained pools keep the ordinary lifetime window [critical]

- [x] 1.1 @regression (agent) run `store::open_pool_budget_tests::retained_open_pools_keep_the_ordinary_acquire_window` before and after the fix -> observed on macOS aarch64 host 2026-09-26: on the unfixed head's `server.rs` (8c301300) the test FAILED in 3.64s with `retained main pool kept an opening-phase acquire window` (left: 31.859502834s, right: 2s); with the fix it PASSED. Both retained main and usage-ledger pools report exactly `ORDINARY_POOL_WINDOW`, and a contended acquire on the retained main pool returns typed `PoolTimedOut` within twice the ordinary window.

## 2. Opening first acquisition still uses the startup budget

- [x] 2.1 @regression (agent) run `store::open_pool_budget_tests::migrated_stage_pool_uses_remaining_startup_budget_and_post_open_pools_stay_ordinary` with the migrated staged main-pool handshake stalled past the ordinary window -> observed: with the first-acquire retry removed (`first_acquire_window` forced to the ordinary window) it FAILED in 4.75s with `open migrated staged main pool` / `authenticate memory branch pool` / `connection phase: initial authentication callback entered` / `pool timed out while waiting for an open connection`; with the fix it PASSED.
- [x] 2.2 @integration (agent) same test, stall beyond the whole startup budget -> observed PASS with typed `PoolTimedOut`; the post-open fresh branch pool stalled past the ordinary window also returned typed `PoolTimedOut` (PASS).
- [x] 2.3 @integration (agent) `server::startup_budget_tests::opening_pool_identity_rejection_is_terminal` and `initial_authentication_uses_remaining_startup_budget_and_reaps_on_expiry` -> observed PASS (4 tests in both modules passed, 43.59s).

## 3. Regression suites and gates

- [x] 3.1 @integration (agent) full `mise run //packages/kuru-memory:test` -> observed exit 0: lib 278 passed (482.84s); integration targets 6, 5, 12 and 1 passed; 0 failed.
- [x] 3.2 @integration (agent) the two originally failing kuru-runtime tests -> observed 2 passed (14.92s). This host does not reproduce the Windows load, so this shows no regression, not the Windows fix.
- [~] 3.3 @runtime (agent) native Windows CI at the fixed head -> defer: requires a push, which this change does not perform. The Windows supervisor accept change from the archived fix is still neither compiled nor exercised locally.
- [x] 3.4 @integration (agent) fmt check, kuru-memory clippy `-D warnings`, strict cospec validation -> observed: `cargo fmt --all --check` exit 0; `mise run //packages/kuru-memory:lint` clean; cospec strict validation passed.
