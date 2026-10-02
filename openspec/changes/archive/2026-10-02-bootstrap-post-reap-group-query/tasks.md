# Tasks

## 1. Platform observation

- [x] 1.1 Keep `OwnedProcessGroup::presence_after_reap` a single signal-zero query and verify its unit tests still count one syscall
- [x] 1.2 Add `PermissionListing` (at most one bounded off-executor listing per cleanup, budget `min(SNAPSHOT_TIMEOUT, deadline - now)`) and `GroupPresence::Recycled`, and verify unit tests for empty, foreign, own, unavailable, zero-budget, already-taken, invalid-group and join-failure listings
- [x] 1.3 Add read-only `observe_group_after_reap` / `GroupObservation` and `snapshot` uid/ruid columns, `group_members_within`, `processes` and `still_listed`, and verify parser and member-selection unit tests

## 2. Cleanup callers

- [x] 2.1 Thread one `PermissionListing` through hooks, RPC, Unix shell, coverage and bounded delivery command cleanup loops and the platform native fixture, and verify their package tests pass
- [x] 2.2 Add a bounded-command test whose signal zero always reports `EPERM` and whose listing sleeps through its budget, and verify it lists once and times out within `CLEANUP_TIMEOUT` plus scheduling slack

## 3. Test identity

- [x] 3.1 Replace the bootstrap capture cleanup query with `post_reap_group`, and verify the regression test against a real foreign-uid process group passes (and fails on the pre-fix query, which returns `EPERM`)
- [x] 3.2 Replace numeric post-reap checks in `bootstrap_install.rs` (producer and group), `advisory.rs`, `coverage.rs`, `hooks.rs` and `kuru-runtime` `review_tests.rs` with listed evidence, recorded `(pid, command)` rows or, for the runtime's directly owned shell, a listed `(pid, ppid = test process)` row that also catches an unreaped zombie, and verify no `kill -0` / signal-zero-only post-reap assertion remains by grep

## 4. Verification

- [x] 4.1 Run `//packages/kuru-platform:test`, `//packages/kuru-delivery:test`, the affected connectors and runtime tests, `format:check`, `lint`, `lint:windows`, `typecheck` and `lint:tooling`, and record observed results in verification.md
