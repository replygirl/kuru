# Tasks

## 1. Regression first

- [x] 1.1 Add Unix regression tests (extraction error with a refused stage removal; cancelled extraction with a refused removal; published retention receipted before the lock is released) and verify they fail on unfixed product code (output in `red.log`)
- [x] 1.2 Add `cfg(windows)` regression tests (cancelled activation recovery with a held stage file; first checked result after the window reports stopped recovery; a late first checked result under a pending cancellation completes with stopped recovery) and verify they type-check where the host allows, naming the windows partitions that run them

## 2. Ordered teardown

- [x] 2.1 Add `StageLease` (`close_published`, `discard_after`, `keep`, `Drop`) that records retention before releasing the lock, and route extraction, probe preparation, cold probe and activation through it; verify no remaining `PrivateTemp`/`CacheLock` pair drops implicitly
- [x] 2.2 Report a late first recoverable Windows result as stopped recovery, with no added await point (decision 5 ruling); verify no deadline, spacing, retry limit or recoverable-error set changed
- [x] 2.3 Release the frozen cancellation test's incidental `runtime` directory handle with its blocker, and pause its tokio clock around the activation so the cancellation always reaches the retry-spacing wait (assertions unchanged); flag both in design.md
- [x] 2.4 Update `docs/memory.md` for receipted unpublished stages and the lock order

## 3. Verification

- [x] 3.1 Verify the regression tests pass after the fix and the provision test modules pass at `--test-threads=2`
- [x] 3.2 Run `//packages/kuru-memory:test`, `format:check`, `lint`, `typecheck`, `lint:tooling`, `docs:check` and strict cospec validation and record observed evidence, naming unrun Windows checks with their proving CI partitions
- [x] 3.3 Resolve CI round 1 (run 36424722859): replace the thread-scoped tracing capture with a path-scoped test seam in every test that used it, count only `.json` receipts in the Windows cancelled-activation test, and record mechanisms and evidence (design.md decision 8, verification 5.x)
