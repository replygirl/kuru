# Tasks

## 1. Regression first

- [x] 1.1 Add Unix regression tests (extraction error with a refused stage removal; cancelled extraction with a refused removal; published retention receipted before the lock is released) and verify they fail on unfixed product code (output in `red.log`)
- [x] 1.2 Add `cfg(windows)` regression tests (cancelled activation recovery with a held stage file; first checked result after the window reports stopped recovery; cancellation at a late first checked result) and verify they type-check where the host allows, naming the windows partitions that run them

## 2. Ordered teardown

- [x] 2.1 Add `StageLease` (`close_published`, `discard_after`, `keep`, `Drop`) that records retention before releasing the lock, and route extraction, probe preparation, cold probe and activation through it; verify no remaining `PrivateTemp`/`CacheLock` pair drops implicitly
- [x] 2.2 Make each Windows checked no-move a cancellation point and report a late first recoverable result as stopped recovery; verify no deadline, spacing, retry limit or recoverable-error set changed
- [x] 2.3 Release the frozen cancellation test's incidental `runtime` directory handle with its blocker (assertions unchanged) and flag it in design.md
- [x] 2.4 Update `docs/memory.md` for receipted unpublished stages and the lock order

## 3. Verification

- [x] 3.1 Verify the regression tests pass after the fix and the provision test modules pass at `--test-threads=2`
- [x] 3.2 Run `//packages/kuru-memory:test`, `format:check`, `lint`, `typecheck`, `lint:tooling`, `docs:check` and strict cospec validation and record observed evidence, naming unrun Windows checks with their proving CI partitions
