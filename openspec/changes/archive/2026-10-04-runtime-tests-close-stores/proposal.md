# Proposal

## Why

Main `cbebf7b7` failed run 37186425537 (ubuntu-latest coverage partition 6,
job 111389939936): all 283 tests passed, then the partition's profile-set
invariant failed with "new kuru-12898-10634519169386027498_2.profraw; an
instrumented process outlived the partition's tests". The `kuru_runtime` lib
(34 tests) was the partition's last executable, and at `cbebf7b7` none of its
store-opening tests closed their temporary memory stores. This is the family
that change `instrumented-child-outlives-test` diagnosed: a dropped store's Dolt
supervisor sits outside the test's process group, stops Dolt on its own, and
exits after the test process, writing its profile during the export.

That family's success path is already fixed on main. `94cf53c5` (#208)
archived `runtime-tests-close-their-stores` with the run's red commit as its
parent. It ends all 80 dropping tests with `crate::tests::close_stores`. Measured
on `94cf53c5`, the whole `kuru_runtime` lib (236 tests) records 300 supervisor
spawns, 300 `owner_finished` and 0 `owner_dropped_live`. Two gaps remain:

- The teardown is the test's last statement, so it runs only on success. An
  assertion failure, a panic or a `?` early return skips it. The store then drops
  live during unwinding, and its supervisor can again write a late profile.
- Nothing structural keeps a future test from opening a store without that
  teardown. The ledger detects such a drop only when the test calls the
  assertion itself, so a forgotten teardown fails in CI's partition export,
  not locally and by name.

The CI artifact holds no spawn rows, because they landed in #208. So the 12898
writer cannot be named from the run. By the family diagnosis it is a
kuru-memory supervisor (inferred, not measured); see verification 1.1.

## What Changes

- New `kuru_memory::test_support::closing(body)`, a teardown scope around a
  test body. While the scope is active on the test's thread, every local
  `MemoryStore` the test opens (`temporary`, `temporary_cold`, `open`,
  `open_observed`, the read-only local fallback of `open_managed_observed`)
  registers its shared server handle with the scope. Only the server is
  retained, so the supervisor stays owned until the scope closes it, while the
  test's views, pools and fixture permit are still released when the test
  drops them. A retained store clone would hold the process's four fixture
  permits, and a loop that opens a fifth store would then wait forever. After
  the body returns, panics or returns early, the scope closes every retained
  server. Close is idempotent, so a store the test already closed is not
  affected. On success it then asserts
  that `unawaited_supervisors(test_supervisors())` is empty and fails with the
  ledger's description if not. After a panic it resumes the original panic once
  the stores are closed.
- Every `#[tokio::test]` in `kuru-runtime` runs its body in `closing`. That
  includes the test modules, the `engine.rs` `publication_tests` and the
  `dream.rs` `cancellation_tests`. The trailing `close_stores` calls added by
  #208 are now redundant and are removed. Mid-test closes, such as a close
  before a reopen or a close in a loop iteration, stay.
- Enforcement: a `kuru-runtime` unit test scans the crate's sources. It fails,
  naming file, line and test, for any `#[tokio::test]` whose body does not start
  with `closing(`. A future test that opens and drops a store therefore cannot
  skip the scope, and the scope reports it by name.
- Test code only. Registration is compiled under
  `cfg(any(test, feature = "test-support"))`, and outside a `closing` scope a
  store behaves exactly as before. Product drop and close behaviour is
  unchanged.

Out of scope, with reasons:

- `kuru-connectors` opens no memory store in its tests.
- `apps/kuru-tui`: `src/ui/runtime_tests.rs`, `src/memory_notice.rs` and
  `tests/{cli,trust,terminal,preferences,embedded_runtime,ui_runtime}.rs` open
  stores and close them on their success paths. Their partitions'
  late-profile writers are mostly `kuru` children spawned through a PTY or a
  subprocess, which elect out-of-process owners. Adopting `closing` there is a
  follow-up, and the shared helper is ready for it.
- Managed (remote) owners are separate processes. Accounting tests await their
  exit with `await_managed_quiescence`, so `closing` registers local stores
  only.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/test_support.rs`, a new
  `test_support/closing.rs`, `facade.rs`, which registers opened local stores,
  and `store.rs` (`server_for_teardown`). All of it is compiled only under
  test-support.
- `packages/kuru-runtime/src/**`: every async test body is wrapped. `tests.rs`
  replaces `close_stores` with the scope, its regression tests and the
  source-scan guard.
- `docs/development.md`: the coverage-partition section names `closing` as the
  runtime test teardown.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
