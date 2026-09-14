## 1. Observe the existing recovery boundary

- [x] 1.1 Update `packages/kuru-memory/src/store/recovery_tests.rs` so the
  process-loss child retains its memory-open task and selects the opening result
  against the DDL pause boundary under the existing 10-second deadline,
  distinguishing open error, task failure, unexpected early success, and the
  reached boundary while preserving parent-owned termination and cleanup.
- [x] 1.2 Add a deterministic real-open regression in
  `packages/kuru-memory/src/store/recovery_tests.rs` that uses the existing
  invalid startup timeout and AfterDdl hook to prove the validation cause is
  surfaced with pre-DDL context instead of a deadline error.

## 2. Evidence

- [x] 2.1 Pass the focused recovery tests, memory package typecheck, formatter,
  strict Cospec validation, and actual apply gate without changing fixture or
  product deadlines.
  The new deterministic open-error regression passed 1/1. The first existing
  real process-loss fixture run inside the sandbox surfaced its loopback-bind
  denial through the new pre-DDL diagnostic instead of a generic deadline;
  the authorized focused run outside that sandbox then passed 1/1 in 3.04
  seconds with its existing preservation, recovery, and cleanup assertions.
  Memory all-target/all-feature typecheck, package formatting, and diff checks
  passed. Strict validation and the actual apply gate passed before source work.
- [ ] 2.2 Pass the corrected fixture under the native Windows instrumented
  memory/runtime shard and emit its ordinary checked receipt.
