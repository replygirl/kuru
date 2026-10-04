# Proposal

## Why

Change `instrumented-child-outlives-test` closed the store of only one
`kuru_runtime` lib test, `provider_free_undo_preserves_sessions_and_archives_added_identities`.
The CI coverage partition failed because that test drops its temporary memory
store without `close()`. A lifecycle trace of the whole `kuru_runtime` lib
(229 tests) shows that 80 tests drop a live local store the same way. Each
dropped store hands its Dolt supervisor to a detached reaper thread. That
supervisor stops Dolt and exits on its own after the test returns. libtest
finishes tests in no fixed order, so any of these 80 tests can be the last in
its executable. Whichever partition the plan gives that executable then fails
with "coverage partition raw profiles changed while its coverage was exported".
Locally, the unfixed `dreaming_rejects_last_role_removal…` left a late
kuru-memory supervisor profile in 2 of 3 instrumented runs.

## What Changes

- New `kuru_memory::test_support::test_supervisors()` marks every supervisor the
  calling test starts. A fixture that opens its store before it can take a
  `supervisor_mark()` (`MemoryStore::temporary()` passed straight into
  `Harness::new`) still gets a complete check from `unawaited_supervisors`.
- New `kuru-runtime` teardown `crate::tests::close_stores(stores)` closes each
  store the test still holds. It then asserts that the test has no unawaited
  supervisor, meaning none still unreaped and none dropped live. All 80
  dropping tests end with it. Tests that open several stores pass all of them,
  or close each one in its loop iteration. The reopen test in
  `preferences_tests` closes its first owner before it reopens.
- The fix is in test code only. Product drop and close behaviour is unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/test_support.rs`: `test_supervisors()`. The engine
  ledger unit test covers its test-wide scope.
- `packages/kuru-runtime/src/tests.rs`: `close_stores` and a regression test
  showing that it catches an open store. The other `*_tests.rs` files and the
  `engine.rs` `publication_tests` module call it.
- `docs/development.md`: coverage partition section.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
