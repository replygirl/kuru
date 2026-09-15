## 1. Typed endpoint availability

- [x] 1.1 Factor a private typed SQLx connection attempt while preserving the existing `connect_pool` contract, add a fail-closed initial-phase predicate to `ConnectionObservation`, and classify only pre-callback `sqlx::Error::Io` with `ErrorKind::ConnectionReset` as unavailable in `live_endpoint`. The frozen `server.rs` is `d7e2b450257eae93468ecd9e040bd1ef778060469254a01b09dbfe3feee4b71b`; the exact portable regression observes the typed initial-phase reset and passed after the correction.
- [x] 1.2 Continue that exact unavailable result through the existing lifecycle lease and owned server startup path while preserving all other authentication, identity, data-directory, protocol, SQL and I/O errors. The implementation changes only the private connection result at `live_endpoint`; existing `connect_pool`, verification and supervisor ownership paths remain intact, including the outer authentication context on setup errors. Native lifecycle acceptance remains task 3.2.

## 2. Deterministic lifecycle regression

- [x] 2.1 Add a direct portable `server.rs` regression whose owned bounded listener accepts exactly the raw availability probe and SQL connection, uses safe zero linger to reset the latter before `after_connect`, and is joined on every path; verify the released helper errors and the corrected helper returns no live endpoint. The released code failed the exact test with raw-probe success followed by `after_connect not entered` and `Connection reset by peer (os error 54)`; the corrected exact test passed 1/1.
- [ ] 2.2 Extend the whole-Job-loss Windows lifecycle regression with the same reset-listener shape after proving the old Job is reaped; verify production open recovers the committed row and cleans up normally, while existing occupied-lifecycle, wrong-credential, checked-directory and SQL-identity controls remain terminal. The frozen `windows_lifecycle.rs` is `5f8261b134e63d50654a2182e0dc48b3bb711c8a7efd14eb335d29d792558846`; its timeout path aborts and awaits the owned listener. Execution is pending native Windows CI.

## 3. Verification

- [x] 3.1 Run focused connection and lifecycle regressions, memory typecheck and lint, Rust formatting, strict Cospec validation and diff checks; record actual host and native evidence separately. The exact portable reset regression passed 1/1; memory typecheck and lint, Rust format fix/check, `git diff --check`, and strict Cospec validation passed. The Windows-only lifecycle regression remains task 3.2.
- [ ] 3.2 Run the new deterministic regression and complete memory/runtime, installation and native-mise acceptance on native Windows, and require the ordinary fail-closed CI aggregate before archive.
