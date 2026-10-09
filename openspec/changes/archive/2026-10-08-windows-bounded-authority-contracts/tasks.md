# Tasks

## 1. Publication and ACL authority

- [x] 1.1 Extend `src/fs/windows.rs` with a missing native publication filename guard; verify rejection before publication with unchanged identities, ACLs and payloads after checked cleanup. Preserve existing passing retained-parent refusal coverage.
- [x] 1.2 Extend `src/windows/security.rs` fail-closed unfamiliar ACE and policy-token extent tests; verify an actual installed ACL is preserved and restored before assertions, and malformed codec inputs cannot supply valid authority.

## 2. Bounded current-image names

- [x] 2.1 Add pure parser tests in `src/windows/process/image.rs` for exact case/Unicode UTF-16 spelling and truncated, unterminated, interior-NUL and wrong-namespace refusal; do not simulate FFI calls or ambient native errors.
- [x] 2.2 Run format and Windows-target lint, review cleanup and exact assertions, and explicitly retain native behavior and every unchanged 95% gate as acceptance required before merge.

Observed locally: package formatting and Windows-target all-target/all-feature
lint pass; independent source reviews confirm exact-state assertions, native ACL
restoration, structural-only token refusal and pure name parsing without FFI
injection. This macOS host cannot execute native Windows tests. The preceding
head passes 136 native platform tests but fails the unchanged 95% gate; the new
conditional ACL installation and every added case require native CI acceptance.
All PR behavior and 95% checks must pass before merge, followed by exact-main
checks before goal completion.
