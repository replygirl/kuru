# Tasks

## 1. Budget scope and red regression tests

- [x] 1.1 Add `pool::within`, `pool::within_until`, the test-only `within_or`, the task-local scope with its in-flight acquire slot, `BudgetElapsed`, `AcquireBound` with the `bound`/`budget` fields, the pending-acquire count and the slow-acquire records, and verify the pool unit tests pass with the new wording
- [x] 1.2 Add the take-once `gate_next_pool_authentication` server seam and the regression tests `contended_acquire_outlives_the_slow_threshold_within_its_statement_budget` and `stage_pool_uses_remaining_startup_budget_and_post_open_pools_are_statement_bounded`, and verify each fails on the 2 s ceiling with its recorded assertion message

## 2. Ceiling, creation budget, identity and sites

- [x] 2.1 Set every `Server::pool` pool's ceiling to `QUERY_TIMEOUT`, keeping the opening first window, floor and retry branch, and verify the two regression tests pass
- [x] 2.2 Run post-open pool creation under one creation budget and verify a held first release ends creation at its budget with no pool retained
- [x] 2.3 Make authored identity rejections terminal for every pool attempt and sticky on retained pools, and verify the retained-pool and creation rejection tests
- [ ] 2.4 Convert memory statement and operation budgets to `within`/`within_until`, wrap the unscoped explicit acquires, and verify the reviewed site list
- [ ] 2.5 Give each receipt-bearing writer one write deadline before its acquire and verify the fence test (`acquire_failure_before_a_write_is_never_uncertain`)
- [ ] 2.6 Split `ORDINARY_POOL_WINDOW` into `OPENING_POOL_FLOOR`, `SLOW_ACQUIRE_THRESHOLD` and rpc.rs `PROBE_BUDGET` with values unchanged, and verify the compile-time handler budget assertion still holds

## 3. Documentation and evidence

- [ ] 3.1 Update `docs/development.md` "Memory pool acquire timeouts" and the rpc.rs `REPLY_MARGIN` comment, and verify `mise run docs:check`
- [ ] 3.2 Record suite wall times, the family runs, static checks and coverage deltas in the verification ledger
