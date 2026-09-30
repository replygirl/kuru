# Tasks

Evidence below was observed on macOS 27.0 arm64 (Darwin), Dolt 2.3.5, in the
worktree `tmp/worktrees/test-memory-engine-contract`, with the focused run
`mise run //packages/kuru-memory:test -- engine_contract -- --nocapture`
(12 tests: 9 engine contract tests and 3 unit tests of the test-support
module). Linux and Windows results come only from CI and are not recorded here.

## 1. Test module scaffolding

- [x] 1.1 Register `packages/kuru-memory/src/store/engine_contract_tests.rs` from `store.rs` with `#[path = "store/engine_contract_tests.rs"] mod engine_contract_tests;`, alongside the existing `open_pool_budget_tests`/`recovery_tests`/`template_tests` registrations, and verify `mise run //packages/kuru-memory:test` discovers and runs the new module's tests.
  Evidence: the focused test task ran 12 tests, 12 passed, in five consecutive runs (20.0 s, 18.4 s, 15.6 s, 13.3 s, 13.2 s).
- [x] 1.2 Give the module a doc comment citing this change's slug, the design doc it verifies (`tmp/roadmap/store-creation-design-2026-09-29.md` sections 2-7, 13), the pinned engine (Dolt 2.3.5, upstream commit `ad65af6cc937d10fa3c88e2041fed4325968b581`, `packages/kuru-memory/support/dolt-assets.json`), and that these are contract tests: they change no product behavior and an honest failing result is a valid, reportable outcome.
- [x] 1.3 Each test function's own doc comment cites the specific upstream URL (or, where upstream is silent, the pinned-commit source path/line) and Dolt version its assertion rests on, and states in one sentence what the store-creation design must do instead if the assertion fails.

## 2. S1 — root creation and bootstrap on a copied data directory

- [x] 2.1 Add `dolt_creates_root_from_environment_with_populated_data_and_empty_config`: file-copy a stopped, fully migrated store's `data/` (via the existing checked filesystem helpers, not raw `std::fs`) into a fresh stage directory with no `config/` (no `privileges.db`), start a server on it with `DOLT_ROOT_PASSWORD` set, and verify completion when a connection using the environment root secret succeeds and Kuru's bootstrap can create `kuru_reader` with a new secret; verify the copy's original users (root secret from the source store, and its `kuru_reader`) do not authenticate against the new server.
  Evidence: passed. Users after bootstrap: `__dolt_local_user__`, `event_scheduler`, `kuru_reader`, `root` (all `localhost`); the two engine accounts are added by Dolt on every start. Source secrets: `1045 (28000): Access denied` for `root` and for `kuru_reader`. Main head equals the source head and `dolt_status` is empty after bootstrap.
- [x] 2.2 If root-from-environment on populated `data/` with empty `config/` does not behave as designed, assert the actual observed behavior and record in the test's doc comment what decision (b) must change (e.g., a required `config/` reset step).
  Evidence: not needed; it behaved as designed. The only deviation is the two engine accounts, which the test now asserts separately.

## 3. S2 — bootstrap identity commit on two refs

- [x] 3.1 Add `bootstrap_connection_commits_identity_on_main_and_usage_branch`: from one acquired connection on the copied store, `USE` the usage branch, run a guarded `UPDATE` of the `kuru_instance` row, then `DOLT_COMMIT('-am', ...)`; repeat `USE`/`UPDATE`/`DOLT_COMMIT` on main from the same connection. Verify each ref's head commit hash differs from the template's captured head and that a fresh read on each ref returns the new identity.
  Evidence: passed. Each head equals its `DOLT_COMMIT` result, differs from the template head, has the template head as its only parent, is clean, and holds the new instance; after a restart with the new identity, product pools on `main` and `kuru_usage_v1` connect and a pool on a retained migration branch fails with `memory SQL project/instance identity mismatch`.
- [x] 3.2 Verify the same connection can move between `USE` targets and commit each without reopening a pool (no second `Server::pool` call between the two ref commits).
  Evidence: both commits ran on one detached connection; `active_branch()` reported the expected ref after each `USE`.

## 4. S3 — branch dirty state without a branch pool

- [x] 4.1 Add `dolt_branches_reports_dirty_for_root_and_reader`: from the main pool only, read `dolt_branches.dirty` for a retained migration branch before and after making that branch dirty out of band, as root and as `kuru_reader`; verify the column reflects the change without opening a pool on the branch.
  Evidence: passed; after the change both root and `kuru_reader` read `(true, ["state"])` for the dirtied branch, and every other branch and main stayed clean.
- [x] 4.2 If `dolt_branches.dirty` is unavailable or does not update as expected under the pinned engine, add a fallback test asserting a revision-qualified `dolt_status` read through the main pool instead, and state in both tests' doc comments which one the design must use.
  Evidence: both work, so the same test asserts both forms (`dolt_branches.dirty` and `` `kuru/<branch>`.dolt_status ``) for both users; the doc comment names `dolt_branches.dirty` as the design's choice.

## 5. S4 — AS OF reads from main without a revision connection

- [x] 5.1 Add `as_of_reads_through_main_pool_open_no_revision_connection`: from a connection opened on main (not on the target branch or commit), run `AS OF <branch-head-hash>` and `AS OF <commit-hash>` queries as `kuru_reader` and as root; verify both succeed and return the pre-migration values, and verify no branch or commit pool was opened (a counting hook or explicit assertion on `Server::pool` call count).
  Evidence: passed. Instead of a counting hook (which would need product code), the test asserts the engine's own session list: `information_schema.processlist` showed only `kuru/main` and `kuru/kuru_usage_v1` sessions while the `AS OF` reads ran. Branch-name, head-hash and parent-hash `AS OF` values equal the values read afterwards through pools on each branch and parent.

## 6. S5 — cross-OS capture format (prepared, not executed here)

- [x] 6.1 Define a test artefact format for a captured stopped-store `data/` tree (manifest plus payload, content-addressed, no absolute paths or host names recorded) as a test-support module, with a round-trip unit test (write then read back) so the format itself is exercised without a real cross-OS capture.
  Evidence: `test_support/engine_contract.rs` (`capture.json` beside `data/`); `capture_round_trips_rejects_tampering_and_plans_the_cross_os_run` passed.
- [x] 6.2 Add `data_tree_captured_on_one_os_opens_and_adopts_on_another`: when a checked-in or `KURU_ENGINE_CONTRACT_CROSS_OS_CAPTURE`-provided capture exists, decode it, open it as an S1/S2 store and verify adoption succeeds; when none is present, assert an explicit, named not-run result (never a silent pass, never a skip that coverage would miss) and cover that not-run branch under coverage.
  Evidence: locally it printed `engine contract S5 NOT RUN: no cross-OS capture: KURU_ENGINE_CONTRACT_CROSS_OS_CAPTURE names no capture directory produced on another OS`. `same_os_capture_opens_and_adopts_through_the_cross_os_consumer` runs the same producer and consumer on this OS and passed (validation with `validate_active`, adoption, restart). `KURU_ENGINE_CONTRACT_REQUIRE_CROSS_OS=1` makes not-run fail. No capture is checked in.
- [x] 6.3 In this change's proposal or a findings note (not a workflow file), describe what CI wiring a real cross-OS run would need: a producing job on one OS, an artifact upload, and a consuming job on each other OS that imports it before running 6.2's test with the env var set. Do not add the workflow file in this PR.
  Evidence: described in `tmp/roadmap/store-creation-design/engine-contract-findings.md` (S5).

## 7. S6 — commit hash shape

- [x] 7.1 Add `commit_hash_width_matches_record_columns`: read a commit hash returned by the pinned engine (via `DOLT_HASHOF` or an equivalent read) and assert its exact length and character set (expected: 32-character lowercase base32, per `packages/kuru-memory/src/store.rs:7001-7009`); verify this matches the width assumed by the decision (c) record design's `CHAR(32)` columns.
  Evidence: passed; 25 hashes from `DOLT_HASHOF`, `DOLT_COMMIT`, `dolt_log`, `dolt_commit_ancestors` and `dolt_branches` were 32 characters of `[0-9a-v]`; they round-trip through a `CHAR(32) CHARACTER SET ascii COLLATE ascii_bin` column, and a 33-character value is refused with error 1105 (`string ... is too large for column 'hash'`).

## 8. S7 — no origin-identifying bytes in a copied store

- [x] 8.1 Add `stopped_data_tree_holds_no_host_path_or_secret_bytes`: enumerate every file under a stopped, copied store's `data/` (including `.dolt/stats` if the pinned engine creates one), and byte-scan each for: the source directory's absolute path, the local host name, the source store's root secret, and the source store's `kuru_reader` secret. Verify none appear in any file.
  Evidence: passed. `data/` holds 10 files, 5 of them in `kuru/.dolt/stats`. No path, host name, container name or secret (whole or 16-character fragment) was found in the source tree or in the served copy. Positive controls: the instance UUID and an uppercase canary in the message namespace were found; a `LONGTEXT` canary was also found. A first run's canary that repeated its own key text was not found, so the scan only proves the absence of literal occurrences.
- [x] 8.2 Add the byte-scan helper under `test_support/` only if no existing helper does this scan; exercise it directly with a unit test asserting it detects an injected marker, so the helper itself is covered.
  Evidence: `scan_detects_an_injected_marker_and_skips_lock_files` passed.

## 9. S8 — record-aware classification equivalence

- [x] 9.1 Enumerate, in the test module's doc comment, each inline check `classify_historical_attempts_in` performs today (`packages/kuru-memory/src/store/migrations.rs:1502-1573`): head revision, `kuru_schema AS OF 'HEAD'` version, `dolt_status` dirty count, receipt match against `kuru_migrations`, sole-parent lookup via `dolt_commit_ancestors`, parent schema validation (a second pool at the parent commit), and ancestry of head and parent via `dolt_log`.
- [x] 9.2 Add one test per inline check (or one parameterized test covering all) that runs the equivalent query through the main pool only (no branch or commit pool) on a migrated store with at least one retained migration branch, and asserts it returns the same answer the existing branch-pool-based check would give.
  Evidence: `record_classification_checks_have_main_pool_equivalents` passed for all six retained branches (v2-v7).
- [x] 9.3 Add a case where the retained branch's `kuru_instance` row differs from main's (simulating a template-era or foreign branch) and verify whether each main-pool equivalent still returns a correct, safe answer (accept or fail closed, never a wrong accept) without opening a pool on that branch.
  Evidence: on an adopted copy, `migrations::validate_active` fails with `memory SQL project/instance identity mismatch`, while every main-pool answer equals the source's; a dirtied branch reads dirty and a force-moved ref reads a different head and parent.
- [x] 9.4 For each check in 9.1, state explicitly in the doc comment whether a main-pool equivalent exists and passes 9.2/9.3, or whether it does not, naming which decision (c) mechanism (dolt_branches, kuru_migration_publications, AS OF) each equivalent uses.
  Evidence: module doc table and the S8 test doc comment. Full parent schema validation has no drop-in equivalent: main's `information_schema` returned 0 columns for `kuru/<parent>` (13-45 at a pool on the parent), while `SHOW TABLES`/`SHOW CREATE TABLE ... AS OF` returned identical definitions.

## 10. Coverage and cross-cutting constraints

- [x] 10.1 Verify every added test-support helper (byte scanner, capture format reader/writer) is exercised by at least one test in this change, so the 90% line coverage gate is not weakened by dead helper code.
  Evidence: each public helper is called by the unit tests and the engine tests; the coverage gate itself was not run locally (CI).
- [x] 10.2 Verify any fixture in this change that copies or walks a directory tree more than 8 levels deep uses `TempDir::with_depth_budget` at its own call site; otherwise the existing default depth guard applies.
  Evidence: every engine fixture root is `TempDir::new(...).with_depth_budget(16)`.
- [x] 10.3 Verify no test changes the test runner's global environment (child fixtures only), and that any instrumented child fixture retains the runner's `LLVM_PROFILE_FILE` destination when clearing its own environment.
  Evidence: the tests only read the two S5 variables; every engine starts through `Server::open`, which retains `LLVM_PROFILE_FILE`.
- [ ] 10.4 Verify every test in this change passes on Linux, macOS and Windows through the existing per-OS native test jobs, without a rerun; where a spike item's assertion differs by OS (e.g., host name bytes, path separators), the test accounts for it directly rather than being skipped on any OS.
  Pending CI: only macOS arm64 was run locally. Path needles cover both separators, UTF-16LE and the Windows verbatim prefix; `lint:windows` passed.

## 11. Acceptance evidence to collect (not yet run)

- [x] 11.1 Record, once run, the output of `mise run //packages/kuru-memory:test` (or the equivalent focused invocation for `engine_contract_tests`) from inside this worktree as the local pass/fail evidence for S1-S4, S6-S8; this task is not run as part of authoring this change and must not be marked done from files alone.
  Evidence: see 1.1; S1-S4 and S6-S8 passed on macOS arm64.
- [x] 11.2 Record, once run, whether S5's capture-present branch or its not-run branch executed locally, and on which OS.
  Evidence: the not-run branch, on macOS arm64; the same-OS consumer passed.
- [x] 11.3 Name explicitly, before archiving this change, which of S1-S8 passed, which failed with a stated required design change, and which (S5's cross-OS branch) reported an asserted not-run result; never infer a pass from the presence of test source files.
  Evidence: S1, S2, S3, S4, S6, S7 and S8 passed. S7's negative covers literal bytes only, and S8 found no main-pool equivalent for full parent schema validation. S5 cross-OS reported not-run.
