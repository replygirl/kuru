# Verification

## 1. Sequential work reuses one authenticated session [critical]

- [ ] 1.1 @regression (agent) `sequential_statements_authenticate_one_session` on a live isolated store -> red on origin/main (more than one authenticated connection on a fresh branch pool), green after the fix
- [ ] 1.2 @regression (agent) `fresh_open_authenticates_one_session_per_pool` -> red on origin/main, green after the fix: main and usage pools each authenticated one connection after a warmed open
- [ ] 1.3 @regression (agent) `receipted_writes_reuse_their_pooled_session` -> red on origin/main (the second write enters the authentication gate), green after the fix; usage-ledger writes on the usage pool included
- [ ] 1.4 @regression (agent) `candidate_view_writes_reuse_candidate_pool_sessions` -> red on origin/main, green after the fix

## 2. The uncertain-write fence is unchanged [critical]

- [ ] 2.1 @integration (agent) `uncertain_write_still_ends_its_session_before_reconciliation` -> the injected post-commit failure settles by its receipt, its connection id is absent from the processlist, the pool is one connection smaller
- [ ] 2.2 @integration (agent) `rejected_write_closes_its_session` -> a reasoning-summary conflict returns the typed conflict and its session is absent from the processlist

## 3. A timed-out acquire names its wait

- [ ] 3.1 @integration (agent) `contended_acquire_names_held_connections` -> held-connections class, size, checked-out and window named; chain still holds SQLx's pool timeout
- [ ] 3.2 @integration (agent) `stalled_authentication_names_the_new_connection_phase` -> new-connection class, the gate's phase, one authentication during the wait
- [ ] 3.3 @unit (agent) wait classifier and carrier tests -> the idle-check-or-release class has no phase; the carrier is `Io` of kind `Other` with the typed payload
- [ ] 3.4 @unit (agent) service owner storage-failure mapping test -> still `StorageFailed`, typed warning emitted

## 4. Exposure measurement

- [ ] 4.1 @benchmark (agent) per-pool authenticated connections across one `all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation` run, before and after -> numbers recorded

## 5. Suites and static checks

- [ ] 5.1 @regression (agent) `mise run //packages/kuru-memory:test`, `mise run //packages/kuru-runtime:test` -> exit 0
- [ ] 5.2 @regression (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check` -> exit 0
- [ ] 5.3 @runtime (agent) native Windows and Linux coverage partitions -> CI on the PR
