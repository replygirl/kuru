# Tasks

## 1. Diagnostics uploads on cancel

- [x] 1.1 Change the three failure-gated diagnostics upload steps (`native-tests.yml`, `ci.yml`, `bundle-build.yml`) to `failure() || cancelled()` and verify `grep -n 'failure()' .github/workflows/*.yml` shows only those three, each with `cancelled()`
- [x] 1.2 Update the `release_workflow.rs` guard for the new condition and verify `mise run //packages/kuru-delivery:test` passes

## 2. Verification

- [x] 2.1 Run `mise run lint:tooling` and verify it exits 0
- [x] 2.2 Run `mise run cospec -- validate upload-diagnostics-on-cancel --strict` and `apply --json` and verify the gate is clear
