# Tasks

## 1. Hooks and red regression tests

- [ ] 1.1 Retain each branch pool's connection observation with an authenticated-connection counter and a test-only authentication gate, and verify the hook compiles against today's raw pool
- [ ] 1.2 Add regression tests for session churn (receipted main, usage and candidate writes; sequential statements; fresh open) and verify each fails on origin/main for the stated defect
- [ ] 1.3 Measure per-pool authenticated connections across one `all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation` run before the fix and verify the numbers are recorded

## 2. Pool funnel, inline release and diagnostic

- [ ] 2.1 Add `MemoryPool` (no `Deref`), its `Executor` with inline release, `PooledSession`, and read transactions on one released session, and verify every store pool site compiles through it
- [ ] 2.2 Release a new pool's first connection inline and verify its identity on one session, and verify the sequential-statement and fresh-open tests pass
- [ ] 2.3 Add `PoolAcquireTimedOut` with its three wait classes, the anyhow context and the `Executor` carrier, and verify the contended, stalled-authentication and classifier tests pass
- [ ] 2.4 Log the typed diagnostic in the service owner's storage-failure mapping without changing the wire fault, and verify it by test

## 3. Receipted write sessions

- [ ] 3.1 Return write sessions to the pool only on a receipted success in `mutate`, `mutate_session_catalog` and the usage ledger's `change`, and verify the write regression tests pass
- [ ] 3.2 Add guard tests that an uncertain and a rejected write end their SQL sessions before reconciliation, and verify they pass

## 4. Documentation and evidence

- [ ] 4.1 Document how to read a pool acquire timeout in `docs/development.md` and verify `mise run docs:check`
- [ ] 4.2 Measure per-pool authenticated connections across the same dream test after the fix and verify the numbers are recorded beside the before numbers
- [ ] 4.3 Run the kuru-memory and kuru-runtime suites, format, lint (host and Windows target) and typecheck, and verify observed results in verification.md
