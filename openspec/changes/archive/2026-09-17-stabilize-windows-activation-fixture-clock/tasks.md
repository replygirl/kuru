## 1. Windows activation fixtures

- [x] 1.1 Add `tokio` test-util as a `kuru-memory` dev dependency and scope `pause`/`resume` to the positive checked-activation call in `packages/kuru-memory/src/provision/native_tests.rs`.
- [x] 1.2 Make the persistent blocker fixture require at least one checked denial while preserving its real elapsed-budget, stage, error, identity and cache-lock assertions.
- [x] 1.3 Run focused package typecheck/lint/format/diff checks and record Windows native execution as pending CI. Host package typecheck/lint, formatting and diff checks passed; independent review was clear. Windows-only fixture execution remains pending native CI. Optional macOS-to-Windows cross-check stopped in `libsqlite3-sys` because this host lacks the MSVC C SDK headers, before reaching the fixture.
