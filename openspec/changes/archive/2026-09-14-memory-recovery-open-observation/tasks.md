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
- [x] 2.2 Pass the corrected fixture under the native Windows instrumented
  memory/runtime shard and emit its ordinary checked receipt.
  Exact-head run `34807526553`, job `103862209833`, passed all 97 memory
  tests and all 76 runtime tests, including the corrected process-loss fixture,
  the new pre-DDL open-error regression, the unresolved-move diagnostic control,
  and the stock-shell model-tool replay. Artifact `10333921680` contains receipt
  SHA-256 `dc97191ce2e7b7f64ea2eb310d54fa3c1fb6602a341764ae746ae17b24db5121`,
  which binds synthetic source `4d3c2d7b3775dc921646882a734a6c6062acc7fe`
  to tree `6d4252b963d4aab00bc9f31b4481fd0fa61b8189` with 9 selected
  executables successful, 55 omitted, and 576 profiles.
