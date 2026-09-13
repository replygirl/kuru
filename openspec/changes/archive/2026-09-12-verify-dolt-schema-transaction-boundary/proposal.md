## Why

The ordered-migration runner depends on the exact Dolt v2.3.3 boundary for DDL,
manual `DOLT_COMMIT`, connection loss and checked branch publication. The native
probe found that pre-commit DDL leaves a dirty working set despite an unchanged
HEAD, so Kuru must retain that negative evidence and prove an isolated branch
keeps active main clean before production migrations are designed or built.

## What Changes

- Extend `packages/kuru-memory/src/store/recovery_tests.rs` with real pinned-Dolt
  fixtures for acknowledged manual commit, actual commit-reply loss, and the
  observed dirty-DDL outcome after pre-commit connection loss.
- Add a test-only isolated migration branch attempt that proves failed DDL never
  dirties active main, retains the failed branch, builds and validates a fresh
  complete attempt, and reconciles an actual lost fast-forward reply to one
  clean published head without changing source or candidate history.
- Add only test-local helpers in the same module if necessary; add no production
  migration path, schema, fault switch, dependency, or language tool.

## Impact

The memory package's native real-engine test and coverage time increase by four
focused cases. Production behavior and store format remain unchanged.
