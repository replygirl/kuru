# Tasks

## 1. Measure the class

- [x] 1.1 Run the whole `kuru_runtime` lib under the lifecycle trace and verify every test that records `owner_dropped_live`

## 2. Close every store at teardown

- [x] 2.1 Add `test_support::test_supervisors()` and verify the engine ledger test covers a supervisor started before a later mark
- [x] 2.2 Add `crate::tests::close_stores` with regression test `close_stores_requires_every_store_the_test_opened`, and verify it reports an open store as unreaped before it closes the store
- [x] 2.3 End every dropping test with `close_stores` and verify the whole lib records 0 `owner_dropped_live`

## 3. Evidence and checks

- [x] 3.1 Run partition 8's runtime set and `dreaming_rejects…` alone, instrumented like the runner, and verify there are 0 late profiles
- [x] 3.2 Run the narrowed package tests and static checks, and record them in verification.md
