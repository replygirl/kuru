# Tasks

## 1. Restore production compilation

- [x] 1.1 Make only the existing std::sync::Arc import unconditional in service.rs and verify unchanged attachment identity implementation.
- [x] 1.2 Use the existing native CI E0425/E0433 baseline as the regression failure and run the owning default production release build without test-support/all-features to observe the corrected pass.
- [x] 1.3 Run affected granular static checks and record exact results and native limitations.

## 2. Completion

- [x] 2.1 Validate strictly, normally archive this compile-only record without durable spec changes, verify its physical archive, and prepare a normal hooked commit for Product's fresh PR243 delivery.

## Observed evidence

- Existing PR243 before-fix logs are /private/tmp/phase2-pr243-arm-build-112370836453.log and macOS/Windows/Ubuntu native installation logs at the same prefix. ARM build explicitly invoked the owning app release build with default features and failed E0425/E0433 at service.rs437/444; no baseline rerun was needed.
- Actual strict validation/apply 27dc35 both exited 0; all four returned context files were read before code. Initial artifact validation errors were corrected before implementation. The upstream no-delta apply advisory is qualified: this is a compile-only import correction with no specified behavior change; archive skips durable spec sync without changing bookkeeping metadata.
- Frozen source SHA256 f52bb6f128ba5bcb7e69175eb9ff10756b4ad6cb260b0971c427502003665826; sole production diff removes the cfg attribute above the existing Arc import. Attachment identity and all accepted session/proof/authority code remain byte-for-byte unchanged.
- Default production release 34766, host lint 79894, Windows lint 13191, format 49675, docs 13605 and managed 3907 all exited 0. No runtime suites, paid calls or native installation evaluations were run locally. Fresh native CI remains Product's delivery responsibility.
