# Verification

## 1. A cancelled partition on a live runner publishes its diagnostics [critical]

- [~] 1.1 @runtime (agent) GitHub-hosted runner of a native-tests partition (any OS in the matrix), a partition cancelled by the host or its time limit while the test step runs, expecting the diagnostics step to run after the cancel and upload `*-coverage-diagnostics-*-attempt-N` when the diagnostics directory exists (a dead runner publishes nothing, since the grace period is short; expected, not a regression) -> defer: only a natural cancellation on a healthy host exercises it; record the next occurrence in the roadmap notes

## 2. Static workflow checks

- [x] 2.1 @integration (agent) `grep -n 'failure()' .github/workflows/*.yml` -> exactly three conditions remain, each paired with `cancelled()`: `native-tests.yml` "Upload failed partition diagnostics", `ci.yml` the same step, `bundle-build.yml` "Upload both builds for diagnosis". Observed 2026-10-02, macOS arm64: all three read `failure() || cancelled()`; the cache-save, release, success-only evidence, `!cancelled()` merge-report and `always()` usage-scan uploads are untouched.
- [x] 2.2 @integration (agent) `mise run //packages/kuru-delivery:test` -> the `native_workflow_partitions_every_os_and_keeps_the_aggregate_fail_closed` guard pins the new condition, asserts the old `failure()`-only form is gone and keeps the merge pattern from matching diagnostics names. Observed 2026-10-02, macOS arm64: exits 0, the guard passes.
- [x] 2.3 @integration (agent) `mise run lint:tooling` -> actionlint, shellcheck and `check:repo` accept the workflows. Observed 2026-10-02, macOS arm64: exits 0 ("Repository metadata invariants passed").
