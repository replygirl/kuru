# Proposal

## Why

The coverage and open-time harnesses in `packages/kuru-delivery` (`src/coverage*`, `src/open_time*`) decide partition and gate outcomes on guessed literals: `RUNNER_CLEANUP_TIMEOUT` (30 s; on expiry the post-terminate settle fails the partition "did not settle"), `COMMAND_TIMEOUT` (30 s), `LIST_TIMEOUT` (120 s), `CAPTURE_TIMEOUT` (10 min), the open-time `run_bound` (180 s), `QUERY_BOUND` (30 s), `RETIRE_BOUND` (120 s, documented as derived from a `SERVICE_IDLE_TIMEOUT` that exists nowhere in the repository), the Windows launch waits in `open_time/launch.rs`, and the inline-test bounds and thin races in `coverage.rs`. A slow runner, a cold registry fetch or a busy host can expire one of these while the shard deadline still has minutes left, failing a partition for a reason that is not a defect in the code under test. A wait must end on the event it waits for, bounded by a derived budget; a CI flake in our own code is a bug.

The shard deadline chain (`shard_deadline` = job minutes minus `EVIDENCE_RESERVE`, handed to the runner as `remaining_until`) is already correct and is the model. Separately, #204 records a follow-on: the release workflow's `tests` job does not record `KURU_COVERAGE_JOB_STARTED`/`_MINUTES`, so a launch stall there is bounded by a 35-minute local window instead of the job deadline.

## What Changes

- Each fix point (inventory: `tmp/roadmap/test-wait-inventory-2026-10.md` section 3 "kuru-delivery CI harness", section 4 row 3) is derived from the shard deadline chain or deleted under it, with the derivation written at the constant: `RUNNER_CLEANUP_TIMEOUT`, `GROUP_BOUND`, `COMMAND_TIMEOUT`, `LIST_TIMEOUT`, `CAPTURE_TIMEOUT`, `run_bound`, `QUERY_BOUND`, `RETIRE_BOUND` (corrected to cite budgets that exist), the launch waits (`open_time/launch.rs`), `census.rs:20`, and the three thin races in `coverage.rs`'s inline tests. The per-point derivation table is recorded in `tmp/roadmap/store-creation-design/derivation-delivery-ci-harness.md` (untracked) and the final value or deletion at each site states what expiry now means.
- A wait that remains keeps a useful diagnostic on expiry. No retry, no raised literal, no new literal without a derivation, and no new helper unless it replaces three or more sites.
- The release workflow's `tests` job records its start and minutes (`KURU_COVERAGE_JOB_STARTED`, `KURU_COVERAGE_JOB_MINUTES`) as the ci.yml and native-tests jobs do, so launches there take the job deadline.
- `docs/development.md` is updated where it describes these bounds, including the stale `SERVICE_IDLE_TIMEOUT` (30 s idle) statement.
- Every partition-behaviour change is named in the pull request body.
- Non-goals: what the coverage gate enforces (the 90% line gate and the profile-set invariant from #200) does not change; test-only delivery files (`tests/`, PR 4 of the sweep) are untouched; no retry.

## Capabilities

### Modified Capabilities

None. The living specs were right; the implementation guessed its bounds.

## Impact

`packages/kuru-delivery/src/coverage.rs`, `src/coverage/orchestrate.rs`, `src/open_time.rs`, `src/open_time/census.rs`, `src/open_time/launch.rs` (and their inline tests), `.github/workflows/release.yml` (`tests` job start recording), and `docs/development.md`. Partition behaviour changes where a literal expiry no longer fails a partition and the shard deadline decides instead; the exact list is in the pull request body. `docs:check` and the delivery package checks must pass.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
