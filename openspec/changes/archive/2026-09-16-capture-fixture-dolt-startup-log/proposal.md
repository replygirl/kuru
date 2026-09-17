## Why

A native coverage run reported an early Dolt startup exit while creating an isolated migration fixture, but its private `server.log` disappeared with the fixture before CI could show the cause. Preserve that bounded fixture diagnostic in the test failure without changing startup or migration behavior.

## What Changes

- Add test-only error context to the initial fixture open in `packages/kuru-memory/src/store/migrations.rs`, reading only its own direct staging directory's redacted `server.log` before fixture cleanup.
- Add a focused helper check for exact project-stage selection and bounded log output.

## Impact

Only the memory migration test file changes. Successful runs have the same assertions and runtime; failing startup may print at most a small tail of fixture-owned diagnostics.
