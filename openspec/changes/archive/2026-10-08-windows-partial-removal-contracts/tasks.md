# Tasks

## 1. Native partial removal

- [x] 1.1 Add the pinned-descendant test in `packages/kuru-platform/src/fs/windows.rs`; capture the actual uncertain outcome, exact root/child identities and ACLs, removed payload and untouched adjacent bytes, then release the blocker and complete checked cleanup before assertions.
- [x] 1.2 Run formatting and Windows-target platform lint, independently review native ownership/cleanup, and retain native CI behavior and the unchanged 95% gate as pending acceptance until executed.

Observed locally: Rust formatting and package Windows-target all-target/all-feature lint pass. Independent review confirms the deterministic one-descendant shape, real sharing refusal, retained authority observations and complete checked cleanup before assertions. Native execution is pending CI; the preceding head passes 141 Windows platform behavior tests but fails the unchanged 95% gate. Neither this native pass nor a final Windows coverage percentage is claimed.
