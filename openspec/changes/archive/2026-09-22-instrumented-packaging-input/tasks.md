## 1. Instrumented packaging input

- [x] 1.1 Update `apps/kuru-tui/tests/embedded_runtime.rs` to prepare a private debug-stripped copy only for the implicit coverage fallback and verify the original artifact, coverage output, product bound, explicit-selection path, and existing installation/update assertions remain intact.
- [x] 1.2 Run formatting, strict Cospec validation, source review, and the exact instrumented `packaged_install_and_update_preserve_complete_offline_memory` acceptance with the verified Dolt bundle.

  Evidence: `cargo fmt --all -- --check`, `git diff --check`, and independent source review passed. The exact instrumented filter passed 1/1 in 44.99 seconds with a 125,455,528-byte prepared executable under the 134,217,728-byte product limit, a 57,120,998-byte archive, successful direct install and self-update offline-memory writes, and one 408,128-byte preparation profile. The first two invocations stopped before fixture behavior on an invalid command option and then two compile-only trait disambiguation errors; the corrected invocation produced the recorded pass.
