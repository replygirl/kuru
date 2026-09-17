## 1. Windows held-probe evidence

- [x] 1.1 Update `packages/kuru-memory/src/provision/native_tests.rs` to assert the checked cleanup error's observed Windows code text while retaining all publication and cleanup assertions. Independent review found no further change.
- [x] 1.2 Run scoped memory typecheck, lint, format and diff checks; record Windows native fixture rerun as pending CI. Package typecheck and lint, `cargo fmt --all -- --check`, and `git diff --check` passed on macOS; the Windows-only fixture has not run on this host.
