# Tasks

## 1. Correct task interpreter selection

- [x] 1.1 Audit repository Windows task bodies and fix every confirmed PowerShell body that uses the default cmd interpreter; retain Unix failure handling and remove inherited PSModulePath at owned stock PowerShell launches.
- [x] 1.2 Add regression coverage for the real task shell/environment contract and native command failure propagation; run the focused delivery launcher checks and affected formatting/lint checks.
- [x] 1.3 Record observed local results and explicitly retain native PR and exact-main checks as external delivery gates after the local source archive.

The test-only regression failed against original task configuration (exit 101),
then passed against the correction. Final focused delivery launcher checks,
Rust/TOML formatting, delivery host lint and Windows-target lint pass. The
native PowerShell probe tests exit 7 and a missing program under the actual
task shell/environment, with LLVM_PROFILE_FILE retained. It compiles and lints
locally; actual Windows execution follows in PR #266 CI. Every PR and exact-main
check must pass before goal completion; no native runtime pass is claimed here.
