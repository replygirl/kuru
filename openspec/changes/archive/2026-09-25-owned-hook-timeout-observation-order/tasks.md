## 1. Preserve owned slow-hook cleanup in the Windows fixture

- [x] 1.1 Update `packages/kuru-connectors/src/hooks.rs` to join the slow hook future with bounded start/lock observation, and keep every assertion after owned completion.
- [x] 1.2 Validate the test-only Cospec change and Rust formatting; retain normal final-head hooks and exact-head Windows native CI as deferred before-merge gates.
