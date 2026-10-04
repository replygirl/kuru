# Tasks

Acceptance evidence is recorded against each task as its check finishes in `verification.md`; unrun checks are named with the reason. Affected surfaces: `packages/kuru-delivery` (`src/coverage*`, `src/open_time*`), `.github/workflows/release.yml` and `docs/development.md`.

## 1. Derivation record

- [ ] 1.1 Write `tmp/roadmap/store-creation-design/derivation-delivery-ci-harness.md` (untracked) with, per fix point, what it bounds, the governing budget, the derived value or the deletion, and what expiry now means; verify every cited budget exists by grep.

## 2. Coverage harness

- [ ] 2.1 Derive or delete `RUNNER_CLEANUP_TIMEOUT`, `COMMAND_TIMEOUT`, `LIST_TIMEOUT` and `CAPTURE_TIMEOUT` under the shard deadline chain with the derivation at each constant, and verify expiry diagnostics are kept.
- [ ] 2.2 Derive `GROUP_BOUND` and replace the three thin races in `coverage.rs` inline tests with observed events, and verify the tests still assert the same outcomes.

## 3. Open-time harness

- [ ] 3.1 Derive `run_bound`, `QUERY_BOUND` (`census.rs`), `RETIRE_BOUND` (no `SERVICE_IDLE_TIMEOUT`) and the launch waits in `open_time/launch.rs` from existing budgets, and verify each cites a constant that exists.

## 4. Workflow

- [ ] 4.1 Record `KURU_COVERAGE_JOB_STARTED` and `KURU_COVERAGE_JOB_MINUTES` in the release workflow `tests` job (the #204 follow-on), and verify the workflow checks pass.

## 5. Tests

- [ ] 5.1 Add deterministic tests pinning each derivation, including a regression test that fails before the fix (a literal expiring before the shard deadline decides the outcome) and passes after.

## 6. Documentation and evidence

- [ ] 6.1 Update `docs/development.md` where it describes these bounds (including the stale `SERVICE_IDLE_TIMEOUT` line) and verify `mise run docs:check` passes.
- [ ] 6.2 Run format check, lint (host and Windows target), typecheck and the delivery tests; record observed results and name unrun checks; verify the hk pre-push hook passes.
- [ ] 6.3 Name every partition-behaviour change in the pull request body.
