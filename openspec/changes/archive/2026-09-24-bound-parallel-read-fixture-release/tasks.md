## 1. Runtime fixture

- [x] 1.1 Change `packages/kuru-runtime/src/tests.rs` to invalidate the prepared file read by adding a hardlink and prove the existing refusal, accepted serial write receipt, cancellation, and no-replay assertions still hold.
- [x] 1.2 Ensure the parallel-read test gate releases on every test exit path and all waits are bounded with useful failure diagnostics.
- [x] 1.3 Run focused native runtime verification, owning lint and format checks; record the distinct Windows CI result without inferring it from local execution. The exact native macOS case passed 1/1, runtime lint and Rust format passed. Native Windows verification remains pending on the final pushed head; the cancelled prior job did not print the inferred file-sharing error.
