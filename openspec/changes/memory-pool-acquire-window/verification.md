# Verification

Local runs: macOS arm64, uninstrumented debug build, pinned Dolt. "Red" is the
first commit on `fix/memory-pool-acquire-window` (budget scope, slot, pending
count, slow-acquire records and typed fields, with origin/main's 2 s pool
ceiling and post-open creation window unchanged); "green" is the named later
commit.

## 1. A contended acquire waits for its statement budget [critical]

- [ ] 1.1 @regression (agent) `open_pool_budget_tests::contended_acquire_outlives_the_slow_threshold_within_its_statement_budget` (every main-pool permit held; A = `within(QUERY_TIMEOUT, SELECT 1)` and B = `within(QUERY_TIMEOUT, acquire)` each outlive their slow-acquire record, events only; one release goes to A by SQLx's FIFO; B dropped while queued clears the pending count) -> red at the first commit (the 2 s ceiling ends A), green after the ceiling change. Red observed 2026-10-02 at the first commit: `A, a contended statement, ended before slow-acquire record 1 of the held pool (0 recorded): Err(error communicating with database: memory pool acquire on kuru/main timed out after 2.002 s (pool ceiling 2.000 s) waiting for a connection held by Kuru work (every permit held); pool size 4 of 4, 0 idle, 4 checked out; ...)`
- [ ] 1.2 @regression (agent) `open_pool_budget_tests::stage_pool_uses_remaining_startup_budget_and_post_open_pools_are_statement_bounded` (opening halves unchanged; a post-open creation gated at authentication stays pending across the two-record event chain, then succeeds with a `QUERY_TIMEOUT` ceiling) -> red at the first commit (the 2 s post-open window ends creation), green after the creation budget. Red observed 2026-10-02 at the first commit: `post-open pool creation ended before slow-acquire record 1 of the held pool (0 recorded): Err(authenticate memory branch pool: connect to authenticated project memory; connection phase: authentication gate entered: pool timed out while waiting for an open connection)`
- [ ] 1.3 @integration (agent) `retained_open_pools_acquire_under_their_statement_budget` -> main and usage ceilings equal `QUERY_TIMEOUT`; with every permit held, `within(500 ms)` fails `HeldConnections`, `bound == StatementBudget`, `budget` and `window` equal to what was passed, `waited >= window`

## 2. The diagnostic names its bound and its cause

- [ ] 2.1 @integration (agent) `stalled_authentication_names_the_new_connection_phase` and `cancelled_release_is_not_counted_as_a_held_connection` under `within_or(QUERY_TIMEOUT, .., gate.entered())` -> `NewConnection`, gate phase, `bound == StatementBudget`, `budget == QUERY_TIMEOUT`, wait class from held sessions only
- [ ] 2.2 @integration (agent) `budget_elapsed_in_execution_is_not_an_acquire_timeout` -> `BudgetElapsed { acquire: None }`, `pool_acquire_timeout` is `None`, the next statement runs normally
- [ ] 2.3 @unit (agent) `unscoped_acquire_reports_the_pool_ceiling` -> `bound == PoolCeiling`, wait `NoIdentityCallback`, chain holds SQLx `PoolTimedOut`
- [ ] 2.4 @unit (agent) slow-acquire record fields -> present and secret-free
- [x] 2.5 @unit (agent) `pool::tests` Display and carrier tests -> at the first commit `diagnostic_names_its_wait_and_omits_a_stale_phase` asserts `timed out after 2.004 s (pool ceiling 2.000 s) waiting with no new`; new `budget_elapsed_carries_only_an_acquisition_it_ended` asserts `(statement budget 0.500 s, 0.400 s left when the acquire began)`, finds the diagnostic through `BudgetElapsed`'s source under anyhow context with no SQLx `PoolTimedOut` in the chain, and `memory statement budget of 30.000 s elapsed` with no diagnostic for execution expiry; new `budget_scopes_nest_to_the_earliest_deadline` (scope ended by an event, no time) reports the earliest enclosing deadline's budget. All `pool::tests` pass. Observed 2026-10-02

## 3. Fence, identity and creation [critical]

- [ ] 3.1 @regression (agent) `acquire_failure_before_a_write_is_never_uncertain` (`put` queued behind held permits, woken by `store.pool.close()`) -> fails with `PoolClosed` in its chain and no uncertain record
- [ ] 3.2 @regression (agent) `post_open_identity_rejection_ends_the_acquire_at_once` (retained pool and creation) -> authored mismatch, `pool_acquire_timeout` is `None`, sticky on the retained pool, creation retains nothing
- [ ] 3.3 @regression (agent) `opening_first_acquire_outlasting_the_floor_is_one_working_session` -> opening pool opens on its first attempt, no abandoned authentication, one working session
- [ ] 3.4 @integration (agent) `first_connection_release_held_past_the_creation_budget_ends_creation_at_its_budget` -> `BudgetElapsed`, no pool retained, a second creation opens one working session

## 4. Suites, family and static checks

- [ ] 4.1 @benchmark (agent) `mise run //packages/kuru-memory:test` and `//packages/kuru-runtime:test` wall times at origin/main 1752e076 and at the final commit -> recorded, any grown test named
- [ ] 4.2 @regression (agent) the three #170 family tests (`mode_baseline_tests::all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation`, `accounting_tests::admitted_without_observation_remains_incomplete_and_unknown`, `review_tests::unavailable_mcp_status_stays_out_of_provider_input_and_memory`) 5 times each -> all pass
- [ ] 4.3 @regression (agent) `format:check`, `lint`, root `lint:windows`, `typecheck`, `docs:check`, `cospec:managed:check`, `cospec validate --strict` -> exit 0
- [ ] 4.4 @runtime (agent) PR CI first-attempt run (native partitions and coverage gate, with the opening retry branch and opening first-release cut coverage deltas reported) -> green with no rerun
