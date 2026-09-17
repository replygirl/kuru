## 1. Windows activation recovery fixture

- [x] 1.1 Correct the persistent held-descendant assertion in `packages/kuru-memory/src/provision/native_tests.rs` to require an observed checked no-move without assuming a second attempt fits the deadline
- [x] 1.2 Preserve the separate transient-blocker successful-retry proof and all persistent-fixture deadline, rejected-error, stage, destination, and lock assertions
- [x] 1.3 Run scoped local static checks, record the old native Windows failure and corrected native run as pending, then validate and archive this test-only change
