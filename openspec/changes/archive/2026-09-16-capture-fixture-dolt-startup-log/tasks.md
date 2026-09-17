## 1. Fixture startup diagnostics

- [x] 1.1 In `packages/kuru-memory/src/store/migrations.rs`, preserve the initial fixture-open error and append a bounded, redacted `server.log` tail from only its matching direct staging directory; verified in `server.rs` that the supervisor writes this log before its failure response, and startup cleanup awaits its exit before the error returns.
- [x] 1.2 In `packages/kuru-memory/src/store/migrations.rs`, test stage selection, missing-log behavior, and output bounds; both focused memory tests passed (1/1 each), package typecheck passed, and Rust formatting and `git diff --check` passed.
