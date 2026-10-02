# Tasks

## 1. Hooks and red regression tests

- [x] 1.1 Retain each branch pool's connection observation with an authenticated-connection counter and a test-only authentication gate, and verify the hook compiles against today's raw pool
- [x] 1.2 Add regression tests for session churn (receipted main, usage and candidate writes; sequential statements; fresh open) and verify each fails on origin/main for the stated defect
- [x] 1.3 Measure per-pool authenticated connections across one `all_four_modes_keep_builtin_dream_requests_and_own_memory_isolation` run before the fix and verify the numbers are recorded

## 2. Pool funnel, inline release and diagnostic

- [x] 2.1 Add `MemoryPool` (no `Deref`), its `Executor` with inline release, `PooledSession`, and read transactions on one released session, and verify every store pool site compiles through it
- [x] 2.2 Release a new pool's first connection inline and verify its identity on one session, and verify the sequential-statement and fresh-open tests pass
- [x] 2.3 Add `PoolAcquireTimedOut` with its three wait classes, the anyhow context and the `Executor` carrier, and verify the contended, stalled-authentication and classifier tests pass
- [x] 2.4 Log every timed-out acquisition with the typed fields from the pool funnel, so the service owner's log names the wait without changing the wire fault, and verify the warning is emitted where the diagnostic is built
- [x] 2.5 Stop counting a pooled session as checked out once its release takes the connection, before awaiting SQLx's return, and verify a release cancelled at a test-only release gate leaves the count and the next timeout's wait class correct
- [x] 2.6 Bound a new pool's inline first-connection release by its attempt's deadline, closing the connection when it expires, and verify a held first release still opens the pool with verification on its own connection
- [x] 2.7 Name the no-callback wait class for what the counter can observe (no connection reaching Kuru's identity callback, including an unfinished TCP or MySQL handshake), and verify the classifier, Display, docs and specs agree
- [x] 2.8 Record on the pool's observation when its first release is cut at the attempt's deadline, and verify the session-count tests accept exactly that one extra authentication and still fail on churn

## 3. Receipted write sessions

- [x] 3.1 Return write sessions to the pool only on a receipted success in `mutate`, `mutate_session_catalog` and the usage ledger's `change`, and verify the write regression tests pass
- [x] 3.2 Add guard tests that an uncertain and a rejected write end their SQL sessions before reconciliation, and verify they pass
- [x] 3.3 Bound each receipted write's session return by the write's own budget, one deadline taken before its apply, and verify a held return closes the connection and the write still returns `Ok`

## 4. Documentation and evidence

- [x] 4.1 Document how to read a pool acquire timeout in `docs/development.md` and verify `mise run docs:check`
- [x] 4.2 Measure per-pool authenticated connections across the same dream test after the fix and verify the numbers are recorded beside the before numbers
- [x] 4.3 Run the kuru-memory and kuru-runtime suites, format, lint (host and Windows target) and typecheck, and verify observed results in verification.md
- [x] 4.4 Record the lead's 2026-10-02 ruling deferring D2 to a follow-up change, the reduces-not-eliminates scope with the residual paths and their measured counts, and the open tracking item, and verify `cospec validate --all --strict`
