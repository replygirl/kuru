## 1. Capture exact fixture startup cause

- [x] 1.1 Extend `packages/kuru-memory/src/test_support.rs` to recognize the supervisor readiness deadline in test-only bounded startup log capture, preserving ordinary and unrelated error behavior.
- [x] 1.2 Add a synthetic `test_support.rs` control for that exact error and unchanged unrelated errors; run focused memory tests and scoped static checks, recording native Windows confirmation as pending.

Observed on macOS: focused `//packages/kuru-memory:test -- startup_log_capture_is_opt_in_exact_and_bounded` passed 1/1; scoped memory typecheck and lint, Rust format check, and strict Cospec validation passed. Native Windows confirmation and the actual Dolt startup cause remain pending on a corrected CI head.
