## Why

The native lost-usage-reply restart fixture failed on macOS with an unclassified owner-lock refusal. The test needs to distinguish each owner lifecycle phase and exclude unrelated test spawns while releasing and reacquiring its lock.

## What Changes

- Update `packages/kuru-memory/src/service.rs` test fixture to label initial open, close, post-close state, and successor open failures.
- Use the existing test-only spawn gate across close and successor admission; retain the usage-reconciliation assertions.

## Impact

Only the memory service test fixture changes. The focused native test and owning memory lint run locally; final-head macOS memory CI remains the merge gate.
