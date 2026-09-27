## 1. Acceptance evidence (defined before implementation)

- [x] 1.1 Record the before baseline: wall time for `mise run //packages/kuru-memory:test` and `mise run //packages/kuru-runtime:test`, per-package line coverage from one `mise run coverage`, and the `RUST_TEST_THREADS` history. Verify the numbers are written in this file.
- [x] 1.2 After the default flips, run the full `kuru-memory`, `kuru-runtime` and `kuru-tui` suites locally. Verify that every suite passes with no test removed or ignored.
- [x] 1.3 Per-package line coverage after the change stays within noise of the baseline, and the 90% workspace gate holds. Verify by reporting both sets of numbers here.
- [ ] 1.4 Windows is verified by native CI on the PR head, not assumed: the copied tree passes `Directory`/`LifecycleLease` private-object and DACL validation. Verify the run ID is recorded.
- [ ] 1.5 The PR's own CI run measures the per-binary saving on each native OS. Verify the timings are recorded against the baseline.

### Observed baseline (before, macOS arm64, base `8225613d`)

- `RUST_TEST_THREADS=2` came from `8e621705` (#11, the Dolt bundling change). It then
  spread to the coverage shards in `86a4665c` (#19) and to delivery `test:install` in
  `67448442` (#88). None of those commit messages gives a reason for choosing 2.
- `mise run //packages/kuru-memory:test`: exit 0, 660 s wall including build. The lib
  ran 278 tests in 568.82 s; the integration targets ran 5 in 13.28 s, 12 in 19.82 s,
  1 in 3.12 s and 6 in 0.01 s.
- `mise run //packages/kuru-runtime:test`: exit 0, 659 s wall. The lib ran 220 tests in
  603.14 s.
- `mise run coverage`: exit 0, 1637 s wall, 87,523/93,264 lines (93.84%). By package:
  kuru-memory 94.63%, kuru-runtime 96.14%, kuru-tui 93.83%, kuru-connectors 94.90%,
  kuru-core 96.45%, kuru-platform 88.28%, kuru-archive 98.29%, kuru-delivery 84.64%.
  The per-file summary is kept in local scratch (`template-fixture-coverage-before.txt`).

## 2. Template creation under lock (T1)

- [x] 2.1 In `packages/kuru-memory/src/test_support.rs` (or `test_support/template.rs`), derive the fingerprint from the SHA-256 of the supervisor `test_supervisor()` returns plus the current schema version. Verify that a unit test shows a changed supervisor or schema version yields a different key.
- [x] 2.2 Create the template in the private profile directory beside `kuru-test-supervisors`, under a `lock_file` lease, from one cold `temporary_cold()`-style open followed by `close()`, staged in `PrivateTemp` and published atomically. Verify with a test that concurrent processes produce one template and a second process reuses it.
- [x] 2.3 Revalidate an existing template's private objects on reuse, and discard and rebuild an invalid or incomplete one under the lock. Verify that a test corrupts a template and observes the rebuild.

## 3. Private copy excluding runtime files (T2)

- [x] 3.1 Open and cleanly close one store, list the retained tree and fix an explicit allowlist. Lock, lease, PID, socket, endpoint and Dolt server lock/info files are excluded. Verify with a test asserting the exact copied set.
- [x] 3.2 Copy per file through `Directory`/`seal_private`, rejecting symlinks and hard links, so owner-only modes and Windows DACLs are recreated. Verify with a test that a link in the template is refused and that the copy passes the store's private-object validation.

## 4. Default template API with cold opt-out (T3)

- [x] 4.1 Make `MemoryStore::temporary()` (test-support gated, `packages/kuru-memory/src/store.rs`) copy the template and run the unchanged `open_inner`, and add the explicit cold API. Verify that no non-gated code changes, using `git diff` scoped to cfg-gated items.
- [x] 4.2 Enumerate every `temporary()` caller in `packages/kuru-memory` (lib, `tests/memory.rs`) and tag each as 1a/1b/2/3/identity. Move class-2, class-3 and identity/secret/migration-asserting tests to the cold API. Verify that the tag list is recorded here.
- [x] 4.3 Add an explicit cold-path test in `kuru-memory` that bypasses the template and asserts a fresh staged open. Verify that it passes.

## 5. Runtime and TUI call sites (T4)

- [x] 5.1 Scan `packages/kuru-runtime` and `apps/kuru-tui` for tests that compare `instance()`, secrets or migration state across two temporary stores. Move any hits to the cold API. Verify that the scan result is recorded here.
- [x] 5.2 Run the `kuru-runtime` and `kuru-tui` suites with the template default. Verify that they pass.

## 6. Measurements (T5)

- [x] 6.1 Re-run the memory and runtime suites locally with the same settings as 1.1 and record the after wall times. Verify the comparison is recorded here.
- [x] 6.2 Run `mise run coverage` once, with no concurrent coverage writer, and record per-package coverage for 1.3.
- [ ] 6.3 Validate strictly and archive the change before the final branch commit.

### Observed after (macOS arm64, local, `RUST_TEST_THREADS=2`, includes build)

- Implementation: `packages/kuru-memory/src/test_support/template.rs`, the
  test-support-gated `temporary`/`temporary_cold`/`open_temporary` in `store.rs`
  and `facade.rs`. `git diff` of `store.rs` and `facade.rs` touches only
  `cfg(any(test, feature = "test-support"))` items and `cfg(test)` modules.
- The fingerprint also hashes the pinned engine version, the fixture scope, the
  capture format constant, and the sources of `store.rs`, `store/migrations.rs`,
  `store/usage_ledger.rs`, `server.rs` and the template module, because
  migrations run in the test process, not the supervisor.
- Fixture tests (all pass): `fingerprint_changes_with_every_input`,
  `concurrent_processes_create_one_template` (a spawned child test process and
  the parent race; exactly one reports `Created`, the other `Reused`),
  `shared_template_holds_only_clean_private_store_state`,
  `invalid_templates_are_rejected_and_rebuilt` (changed bytes, a missing
  manifest with an abandoned stage, and, on Unix, a planted link),
  `template_copies_are_isolated_stores_with_their_own_servers` and
  `cold_constructor_runs_every_migration_under_a_new_identity`.
- Observed template contents: `identity.json`, `ready.json`, `config/`
  (`branch_control.db`, `privileges.db`), `data/` (Dolt `.dolt` trees without
  `noms/LOCK`) and `home/` (without `eventsData/dolt.lock`); 35 entries.
  Excluded: `lifecycle.lock`, `server.log`, `server.yaml`, empty `staging/`,
  `LOCK` and `*.lock`, and `locks/<hash>` outside the store.
- 4.2 tags for `kuru-memory` `temporary()` callers:
  - Cold (migration or receipt-authority assertions): `store.rs`
    `historical_v1_read_rejects_dirty_dropped_receipt_authority`,
    `migration_validation_rejects_extra_schema_or_receipt_authority` (4 opens);
    `store/migrations.rs` `v6_registry_rejects_v7_store_without_mutating_it`,
    `migration_attempt_rejection_covers_ambiguous_ready_and_dirty_shapes`,
    `migration_attempt_rejection_preserves_invalid_inventory`,
    `test_v8_receipt_order_and_operation_uniqueness_are_enforced`,
    `historical_and_out_of_order_attempts_fail_without_mutation` (2 opens).
  - Template, 1b (observe sessions on the server each copy still owns):
    `store/operational_gc_tests.rs` (6) and the session-teardown and receipt
    reconciliation tests in `store/recovery_tests.rs` (5). Kept on the template
    by decision; none is process-lifecycle or identity work, and all passed.
  - Template, 1a: the other 23 `store.rs` sites, `store/export.rs` (5),
    `store/usage_ledger.rs` (4), `tests/memory.rs` (5) and the facade wrapper.
  - Class 2 and the remaining class 3 tests never used `temporary()`; they
    open caller-owned directories and are unchanged.
  - Identity or secret comparisons across `temporary()` stores: none.
- 5.1 scan: `kuru-runtime` and `kuru-tui` contain no `instance`, credential or
  `kuru_migrations` comparison across `temporary()` stores. The seven tests with
  two `temporary()` stores use them sequentially and independently; the
  cross-store cases (`tests.rs` `other_memory`, TUI `terminal.rs` identity
  check) open caller-owned directories. No runtime or TUI call site changed.
- `mise run //packages/kuru-memory:test`: exit 0, 618 s wall (before 660 s).
  The lib ran 285 tests (278 plus 7 new) in 520.96 s (before 278 in 568.82 s);
  `tests/memory.rs` ran 5 in 3.02 s (before 13.28 s); the others ran 12 in
  18.45 s, 1 in 1.55 s and 6 in 0.01 s.
- `mise run //packages/kuru-runtime:test`: exit 0, 236 s wall (before 659 s).
  The lib ran 220 tests in 200.33 s (before 603.14 s).
- `mise run //apps/kuru-tui:test`: exit 0, 366 s wall; lib 114 tests in 20.43 s.
  No before baseline was recorded for this suite.
- `mise run coverage`: exit 0, 1154 s wall (before 1637 s), 88,143/93,919 lines
  (93.85%). By package: kuru-memory 94.63% (29,791/31,483), kuru-runtime 96.14%,
  kuru-tui 93.83%, kuru-connectors 94.90%, kuru-core 96.45%, kuru-platform
  88.28%, kuru-archive 98.29%, kuru-delivery 84.64%; every package other than
  kuru-memory is line-for-line identical to the baseline.
- Per-file check inside kuru-memory: `store.rs` 10,032 to 10,063 covered lines
  (95.71% to 95.72%), `facade.rs` 4,714/5,140 to 4,714/5,145 (5 new gated lines),
  `test_support.rs` 89.43% to 91.54%, new `test_support/template.rs` 94.02%.
  `service.rs` went from 2,790 to 2,787 covered lines (91.63% to 91.53%): lines
  4041, 4042 and 4045 are the retry body of the idle-service owner-lock wait in a
  service test that opens caller-owned directories, not `temporary()`. The owner
  released its lock before the first poll in this run; no cold path lost coverage.
  In kuru-platform the summary shows `fs.rs` one line lower and `unix.rs` one
  higher (package total unchanged); a per-line lcov diff of `fs.rs` finds no
  line covered before that is uncovered after.
- Static chain: `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling
  ::: cospec:validate ::: cospec:managed:check ::: docs:check` exit 0 on the
  committed tree.
- Pending CI: Windows verification (1.4) and per-binary CI timings for the
  `kuru_runtime` lib, `kuru_memory` lib and `kuru-tui` on each native OS (1.5).
- Not yet observed: 1.4 and 1.5 need native CI on the pushed PR head, and 6.3
  archives only after them. The Windows template path (native process spawn in
  the two-process test, DACL-checked copies, external lifecycle root) was not
  compiled or run locally.
