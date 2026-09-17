## 1. Windows warm-cache fixture

- [x] 1.1 Extend `packages/kuru-memory/src/provision/native_tests.rs` fixture invalidation to retry only existing raw sharing violation 32 or typed uncertain raw access denied 5, with the current identity check and two-second bound intact.
- [x] 1.2 Verify scoped formatting, typecheck, lint, and the real warm-cache fixture where supported; record that Windows native proof remains pending until CI.

Observed: `mise run format:rust:fix`, memory package typecheck and lint passed. The focused real-Dolt warm-cache test passed 1/1 on macOS. The new Windows-only predicate has not run natively yet; PR CI must establish that result.
