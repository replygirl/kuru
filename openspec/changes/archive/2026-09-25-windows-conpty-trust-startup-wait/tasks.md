## 1. Windows trust fixture

- [x] 1.1 Update `apps/kuru-tui/tests/windows_terminal.rs` so the approved persistent-choice path uses its configured cold-start bound and requires the exact missing Responses-key diagnostic, while preserving the short decline wait, no-alternate-screen check, and persisted-trust check.
- [x] 1.2 Verify the focused diff against the existing Unix trust fixture, run Rust formatting and strict Cospec validation, and record that exact-head native Windows CI remains required before merge.
