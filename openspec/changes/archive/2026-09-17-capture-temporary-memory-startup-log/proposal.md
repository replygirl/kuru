## Why

A real Windows export fixture timed out opening its staged Dolt server, but the test-only diagnostic helper discarded the retained server log because it recognized only a different startup error spelling. The actual Dolt cause is still unknown.

## What Changes

- Extend the exact startup-error predicate in `packages/kuru-memory/src/test_support.rs` to include a supervisor readiness deadline, so existing bounded staged or active log capture annotates that fixture failure.
- Add a synthetic control proving this error is annotated while unrelated opening errors remain unchanged.

## Impact

Only test-support error reporting changes. Ordinary memory opens, production deadlines, retries, and fixture behavior stay unchanged; native Windows confirmation awaits CI.
