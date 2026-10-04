# Tasks

Acceptance evidence is recorded against each task as its check finishes in `verification.md`; unrun checks are named with the reason. Affected surfaces: `packages/kuru-delivery` (`src/coverage*`, `src/open_time*`), `.github/workflows/release.yml` and `docs/development.md`.

## 1. Derivation record

- [x] 1.1 Write `tmp/roadmap/store-creation-design/derivation-delivery-ci-harness.md` (untracked) with, per fix point, what it bounds, the governing budget, the derived value or the deletion, and what expiry now means; verify every cited budget exists by grep.
  - Written: 12 rows (8 coverage, 4 open-time) plus the workflow follow-on, the jobs left without the variables, the PR 4 items and the remainder seen. Cited budgets checked by grep on the base: kuru-memory `server.rs:65-75` (`CLOSE_GRACE` 8 s, `KILL_GRACE` 3 s, `SUPERVISOR_TRANSPORT_ALLOWANCE` 2 s, `SUPERVISOR_REAP_ALLOWANCE`), `server.rs:171-177` (`close_budget()` = 8 + 3 + 13 + 8 = 32 s), `store.rs:66` (`QUERY_TIMEOUT` 30 s), `service/rpc.rs:38` (`OPERATION_TIMEOUT` 35 s), `test_support.rs:582,604` (`FreshOpen::FirstProject` 3 starts, `starts_budget`), `service.rs:1937` (`first_attachment: Some(self.startup_timeout)`), `service.rs:9030` (`owner_retires_at_once_when_its_starter_detaches`); kuru-core `config.rs:597` (`startup_timeout_secs: 30`). `grep -rn SERVICE_IDLE_TIMEOUT packages/kuru-memory/src` finds nothing. The same values are pinned to their source text by `open_time::tests::product_budgets_match_their_sources`.

## 2. Coverage harness

- [x] 2.1 Derive or delete `RUNNER_CLEANUP_TIMEOUT`, `COMMAND_TIMEOUT`, `LIST_TIMEOUT` and `CAPTURE_TIMEOUT` under the shard deadline chain with the derivation at each constant, and verify expiry diagnostics are kept.
  - `COMMAND_TIMEOUT` (d1#1): deleted. `checked_output`/`identity`/`verify_source` take a deadline: the shard deadline before the tests, `evidence_deadline` (job limit less one reserve slice) for the receipt (`ReceiptOptions.deadline`). A passed deadline is refused naming it; expiry keeps `bounded_output`'s timeout, cleanup and tree diagnostics.
  - `RUNNER_CLEANUP_TIMEOUT` (d1#3): the settle after an ordinary exit is no longer bounded by it (deleted there: the settle takes its wait's deadline, the shard deadline, and expiry is `TimedOut`, so the partition reports a stall with the settle step appended to the stall sample instead of bailing "did not settle"). For the waits after a tree is stopped it is derived: `RESERVE_SLICES = 4`, `EVIDENCE_RESERVE / 4 = 150 s`, written at the constant.
  - `RUNNER_CLEANUP_TIMEOUT`'s third role, named in review of #216: the output drain after an ordinary exit also took it (30 s, non-fatal). Deleted there: the drain is the rest of the executable's wait and takes the shard deadline (`timeout_at(deadline, ..)`). No product process inherits a test's stdout (kuru-memory owner stdout null on Unix `service.rs:1367` and Windows by `NativeSpawnSpec` default; supervisor on a private pipe), so a pipe still open at the deadline is a leaked holder and the executable is reported `Stalled` without a second terminate. The stall error now says what the wait was doing (`deadline reached while waiting for <exe>: <wait>`), from a `wait` field on `StallEvidence` that `StallReport` does not serialize. The two passed-deadline refusals share one phrasing, `coverage deadline <t> passed before running <cmd>`.
  - `LIST_TIMEOUT` (d1#5): deleted; the list takes `remaining` (the shard deadline).
  - `CAPTURE_TIMEOUT` (d1#113): deleted; `Host::capture` takes the shard deadline (every capture precedes the tests), the cold-runner note moved to the trait doc.
- [x] 2.2 Derive `GROUP_BOUND` and replace the three thin races in `coverage.rs` inline tests with observed events, and verify the tests still assert the same outcomes.
  - `GROUP_BOUND = RUNNER_CLEANUP_TIMEOUT` (d1#25). d1#26: the six `sleep 300`/`sleep 30` members are `tail -f /dev/null`, blocking until the group signal. d1#29: the stall test's deadline is the observed `test stuck has been running` line, through the test-only `DeadlineAt` wrapper over the real `GroupProcess`; the member starts before the marker lines (a draft that printed the marker first failed locally: the group kill raced the member's fork and the cleanup wait expired after 150 s with the group present). d1#36: the test polls the non-reaping `root_state()` until `Exited` before `wait(ZERO)`. All assertions unchanged except one, changed in review: `unix_group_exit_preserves_status_and_kills_group_before_reap` no longer asserts `elapsed < GROUP_BOUND` (vacuous at 150 s); with the drain on the deadline, `Exited` itself proves the relay reached end of file. Before that change, the six Unix group tests passed three consecutive runs locally (macOS).

## 3. Open-time harness

- [x] 3.1 Derive `run_bound`, `QUERY_BOUND` (`census.rs`), `RETIRE_BOUND` (no `SERVICE_IDLE_TIMEOUT`) and the launch waits in `open_time/launch.rs` from existing budgets, and verify each cites a constant that exists.
  - Product values restated once at `open_time.rs` with citations (the tool does not depend on kuru-memory) and pinned to their source text.
  - `run_bound` (d1#42, d1#47): `RUN_BOUND` = first-project fresh-open budget 310 s + one `OPERATION_TIMEOUT` 35 s = 345 s (was 180 s, below the product's own 310 s).
  - `RETIRE_BOUND` (d1#39): `max(QUERY_TIMEOUT, STARTUP) + close_budget()` = 62 s (was 120 s, citing a nonexistent idle timeout); the summary prints the constants instead of "bound 120 s"; module doc's "idle window" corrected.
  - `QUERY_BOUND` (d1#50): `RUN_BOUND`, the harness's one-command bound (no product or vendor budget for `lsof`/`NETSTAT.EXE`; expiry is report-only).
  - Launch waits (d1#48, Windows): the post-EOF quiescence wait and the post-terminate reap take the run's `bound` (the outer timeout decides the first).

## 4. Workflow

- [x] 4.1 Record `KURU_COVERAGE_JOB_STARTED` and `KURU_COVERAGE_JOB_MINUTES` in the release workflow `tests` job (the #204 follow-on), and verify the workflow checks pass.
  - First step `Record the job start for the inner test deadline` (same text as native-tests.yml:89 and ci.yml:296); the test step sets `KURU_COVERAGE_JOB_MINUTES: "60"` ("Must equal this job's timeout-minutes."). `mise run lint:tooling` (actionlint, shellcheck, `check:repo`): exit 0. `release_workflow` tests: 36 passed (the secret-store session equality still holds; the env precedes `run:`). Jobs still running delivery test binaries without the variables, not changed: native-tests.yml `install` and release.yml `verify-staged` (`test:previous-release-update`).

## 5. Tests

- [x] 5.1 Add deterministic tests pinning each derivation, including a regression test that fails before the fix (a literal expiring before the shard deadline decides the outcome) and passes after.
  - Regression: `coverage::tests::list_and_selections_take_the_time_left_before_the_shard_deadline` (the scripted launcher records each list bound and selection `remaining`). Mutation: with the list capped back at `remaining.min(Duration::from_secs(120))` it failed ("120s not in 2100..=2100"); restored, it passes.
  - Pins: `cleanup_and_receipt_bounds_are_slices_of_the_evidence_reserve`, `orchestrate::tests::every_process_bound_takes_the_partitions_deadline_chain` (every capture and source check gets the shard deadline, the receipt the evidence deadline, in both modes), `a_capture_past_its_deadline_is_refused_without_running`, the passed-deadline refusal in `modified_tracked_source_cannot_claim_head_identity`, and `open_time::tests::product_budgets_match_their_sources`.

## 6. Documentation and evidence

- [x] 6.1 Update `docs/development.md` where it describes these bounds (including the stale `SERVICE_IDLE_TIMEOUT` line) and verify `mise run docs:check` passes.
  - Partition runner paragraph (every pre-test wait on the shard deadline, receipt on job limit less a slice, the four reserve slices); open-time cases 1 and 3, the ramp sentence, and the infrastructure-failure paragraph (62 s retirement bound and its derivation, 345 s run bound). `mise run docs:check`: exit 0.
- [x] 6.2 Run format check, lint (host and Windows target), typecheck and the delivery tests; record observed results and name unrun checks; verify the hk pre-push hook passes.
  - See `verification.md` 1.4 and 1.5. The hk pre-push hook was not run: this stage commits without pushing; the orchestrating session pushes.
- [x] 6.3 Name every partition-behaviour change in the pull request body.
  - #216's body lists them, including the ordinary-exit drain added in review, the stall error wording and the unified refusal text; the local checks are filled from `verification.md` 1.4 and 1.5. The draft in `tmp/roadmap/store-creation-design/pr-body-delivery-ci-harness.md` is kept in sync.
