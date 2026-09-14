## 1. Live-owner refusal fixture

- [x] 1.1 Update `packages/kuru-memory/src/store/purge.rs` so the one-second budget applies only to `MemoryStore::purge`, while preserving the existing live-owner refusal assertions.
- [x] 1.2 Run the focused real-memory refusal test with cargo-llvm-cov 0.9.1 and an isolated instrumented target; observed 2026-09-13: 1 passed, 118 filtered, individual test 2.61s, one raw parent-test-process profile, and no LLVM profile errors. The retained profile does not establish instrumented Dolt child/supervisor coverage.
