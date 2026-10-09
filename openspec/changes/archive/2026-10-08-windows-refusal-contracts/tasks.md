# Tasks

## 1. Filesystem ownership and access refusal

- [x] 1.1 Add real wrong-object/type tests in `src/fs/windows.rs` and insufficient-security-access tests in `src/windows/security.rs`; observe refusal before mutation and unchanged retained identity, ACLs and bytes.
- [x] 1.2 Run format and native Windows target lint for these tests; name native behavior and coverage checks that remain dependent on CI.

## 2. Native process and input contracts

- [x] 2.1 Extend `tests/windows_process.rs` with pending-write error propagation through a second write and shutdown, plus live and reaped trusted-child snapshots; retain explicit child cleanup before assertions.
- [x] 2.2 Extend `tests/fixtures/process.rs` and `tests/windows_commands.rs` with read-only current-image access and command/drive-environment preflight validation before execution.
- [x] 2.3 Simplify `src/windows/console.rs` test-support mismatch translation into observations asserted after restoration in the existing fixture; preserve every mode/focus observation, checked union decoding and native API error.
- [x] 2.4 Run format and Windows target lint, and review cleanup/assertion preservation; report actual native behavior and coverage from PR CI before merging.

Observed locally: package format check and Windows-target all-target/all-feature
lint pass. Independent source reviews confirm retained refusal evidence, exact
console assertions and tag checks before union access, and owned cleanup.
This macOS host cannot execute the added Windows cases. Prior exact-head CI
passes 130 native platform tests but fails the unchanged 95% gate; it does not
verify these new cases. Their native execution and every PR 95% gate remain
mandatory before merge; exact-main checks remain mandatory before goal completion.
