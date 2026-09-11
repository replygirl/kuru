## 1. Retain the failure and process owner

- [x] 1.1 Add real controlled timeout regressions and record their failure against the original consuming capture behavior without changing product deadlines or treating a compile error as evidence. Both controls compiled and reached the missing-observation assertions on macOS; 0 passed/2 failed in 0.36 seconds, exit 101. See verification evidence and `/tmp/kuru-bootstrap-observation-red.log`.
- [x] 1.2 Implement fixture-local bounded captures, named context, non-reaping root observations and awaited owned cleanup; verify the controlled regressions pass and all existing bootstrap assertions remain intact. All 17 default-feature bootstrap cases passed, including actual timeout, excess-output and surviving-child controls; the initial natural-exit correction is recorded in verification.
- [x] 1.3 Add the existing pinned rustix Unix test dependency through the workspace and verify ordinary/default-feature test compilation plus relevant strict lint/format checks. Package-context mise execution compiled without tooling/all-features, Cargo.lock stayed unchanged, and strict delivery Clippy plus the granular code-format graph passed.

## 2. Complete integration

- [ ] 2.1 Run normal concurrent hooks and required native integration, record exact results and the limits of historical-cause inference, then strictly validate and actually archive the completed change.
