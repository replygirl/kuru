# Verification

Local host: macOS arm64, 2026-09-30, worktree based on `origin/main`
0b39e733. Every mise call ran with `MISE_LOCKED=1` and
`MISE_CEILING_PATHS=<repo>/tmp/worktrees`; `mise.lock` was unchanged after
each. Logs are in the session scratchpad (`full-test.log`, `neg-*.log`,
`typecheck.log`, `move.diff`).

Rebased 2026-09-30: the branch was moved onto `origin/main` `b8c7f489`
(confirmed by `git merge-base origin/main HEAD`), which carries #140; #141
was not merged at rebase time. `format:check` and `typecheck` were re-run
locally post-rebase. The PR's CI run (`gh run 36767143779`, all jobs green,
see item 2.3 and coverage evidence below) is the full-suite evidence for the
rebased tree across every OS; nothing in 1.1's assertions changed shape
after the rebase.

## 1. Fresh-open engine/lock/lease sequence is unchanged [critical]

- [x] 1.1 @equivalence (agent) `mise run //packages/kuru-memory:test` -> exit 0: lib 371 passed, 3 ignored (the existing opt-in lifecycle measurements), 0 failed; the integration targets passed 10, 5, 12 and 1. No existing test file was edited. The invariant modules all passed: `store::recovery_tests` 23, `store::open_error_reap_tests` 5 (including `staged_open_error_returns_only_after_its_server_is_reaped`, which asserts the moved "open migrated staged main pool" context), `store::marker_fixture` 2 (both ready-marker boundaries, with preservation and reuse), `store::open_pool_budget_tests` 2, `store::migration_lifecycle_tests` 3 and `store::template_tests` 2.
- [~] 1.2 @regression (agent) run the existing engine_ledger-based fresh-open count assertion -> defer: no kuru-memory test asserts an exact engine start, close or launch count (`grep` for start-count assertions found only the `fresh_open_budget` doc comment's "four server starts" model in `test_support.rs`), and this change takes no new measurement; the release harness that reports per-case starts observes it. The counts are unchanged by construction: the normalized diff of the moved code against `HEAD:store.rs` 1788-1951 (`move.diff`) shows one `Server::open_with_guard` per job (3 staging starts, as before), one close per init and validate-and-mark job plus the migration worker's own close (3 closes, as before), and the unchanged active start in `open_inner`. The only other differences are the lock returned as each job's value instead of assigned to `lock`, the per-start context strings selected by a private `Start` enum with identical text, and `return` statements becoming tail expressions.
- [x] 1.3 @integration (agent) every real fresh open in 1.1 through the extracted worker against the bundled Dolt supervisor, including the 13 `temporary_cold()` call sites (`facade.rs` 1, `store.rs` 5, `store/migrations.rs` 6, `store/template_tests.rs` 1) and the test template's own cold build -> all passed; `marker_fixture::released_marker_observation_activates_the_same_committed_store_once` confirms the activated store keeps the stage identity and `initial_revision` and carries one initialization commit.

## 2. Cancellation during stage build still reaps before the lock returns [critical]

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test -- stage_worker` -> `store::stage_worker::tests::cancelled_open_during_stage_build_keeps_startup_lock_until_reap ... ok` (11.2 s), and it passed again in the full run. It cancels the opener at two job boundaries. At `migrate` it pauses at `AfterDdl` and aborts the opener; while the worker is paused, the startup lock is still held and the ledger still shows the stage engine. After resuming, the lock is acquired only when `engine_ledger` shows no live supervisor under the root. At `validate-and-mark` it pauses at the ready-marker `Before` boundary and aborts, then checks the same thing. Each time, a following ordinary open preserves the unready stage once and opens. The init job has no pause point; it releases its lock through the same close and owner drop.
- [x] 2.2 @regression (agent) Negative checks with the hand-off broken, each a scratch edit reverted afterwards: (a) `drop(server.take_reap_guard())` on the migrate start -> FAILED with `the cancelled stage opener released the startup lock while its migration worker was paused; live owners: [...staging-... has not been reaped]` (`neg-Migrate.log`); (b) the same drop on the validate start of the observed opener only -> FAILED with `a second opener acquired the startup lock before the cancelled stage engine was reaped: [...]` (`neg-Validate.log`).

- [x] 2.3 @runtime (agent) the cancellation test on the native Windows and Linux test jobs -> PR #143 head `63216d2aa5cef3734c7b31ed32c950ce741eecfb`, CI run [36767143779](https://github.com/replygirl/kuru/actions/runs/36767143779), all jobs `pass` (`gh pr checks 143 --json name,bucket,link`); `store::stage_worker::tests::cancelled_open_during_stage_build_keeps_startup_lock_until_reap ... ok` observed directly in job logs on Linux (`Native memory partition (ubuntu-24.04-arm, 3)`, job 110064441430) and on Windows arm64 (`native-tests (windows-11-arm) / Behavior partition (windows-11-arm, 5)`, job 110064556943), fetched with `gh api --allow-escape-sequences repos/replygirl/kuru/actions/jobs/<id>/logs`; coverage gate: ubuntu-latest merge (job 110070447100) reports `coverage ubuntu-latest: 94.63% of lines (104337 of 110256) ... gate 90%`, and the `Require native coverage and installation checks` jobs for macos-latest, ubuntu-latest, windows-latest and windows-11-arm all report `pass`.

## 3. Creation selector reaches no product caller

- [x] 3.1 @unit (agent) `mise run //packages/kuru-memory:typecheck` (`cargo check --all-targets --all-features`, so the lib with `test-support` and no `cfg(test)`, and the test build) -> exit 0, no warnings. `mise x -- cargo check -p kuru-memory --locked` (no features, where the field does not exist) -> exit 0 with only the 2 existing `fixture_commit_malformed_state` dead-code warnings. The workspace `mise run typecheck` -> exit 0.
- [x] 3.2 @regression (agent) `grep -rn "OpenOptions {" packages/ apps/ --include=*.rs` -> only the definition and `impl` in `store.rs`, plus 3 function signatures returning `OpenOptions` (`kuru-delivery/tests/support/mise_acceptance.rs:566`, `kuru-runtime/src/preferences_tests.rs:20`, `kuru-memory/src/test_support/windows.rs:53`). There is no struct literal outside `OpenOptions::new`.
- [x] 3.3 @equivalence (agent) `temporary()` (passes `Creation::Default`), `temporary_cold()` (`Creation::Cold`) and the test template's `build()` (`Creation::Default`), the only `open_temporary` callers, at every call site -> every `temporary()`/`temporary_cold()` call site compiled unchanged under the workspace typecheck, and the kuru-memory ones passed in 1.1.

## 4. Lint and type checks stay clean

- [x] 4.1 @unit (agent) `mise run //packages/kuru-memory:lint` -> exit 0; `mise run //packages/kuru-memory:lint:windows` (clippy for `x86_64-pc-windows-msvc`) -> exit 0; `mise run format:check`, `mise run lint:tooling` and `mise run cospec -- validate memory-stage-worker-extraction --strict` -> exit 0.
