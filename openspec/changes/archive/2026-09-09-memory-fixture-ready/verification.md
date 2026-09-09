## 1. Readiness under the real harness [critical]

- [x] 1.1 @regression (agent) run the actual fixture with RUST_TEST_THREADS=1 under a 12-second process-group deadline -> original fixture timed out with exit 124; corrected fixture completed in 0.02s with all four durable messages
- [x] 1.2 @integration (agent) run the fixture normally with each child explicitly using one harness thread -> completed in 0.05s while preserving concurrent child-process initialization
- [x] 1.3 @integration (agent) run core Clippy, formatting and cospec strict validation -> checks passed with no production code or test-count changes

The bounded command was `timeout --signal=TERM --kill-after=2s 12s env RUST_TEST_THREADS=1 mise run //packages/kuru-core:test -- independent_processes_initialize_and_write_one_durable_database`. Its old output is recorded in `/tmp/kuru-memory-fixture-single-thread.log`. A direct child diagnostic confirmed the exact `test NAME ... ready` prefix before the fix. Afterward, both the forced invocation and the normal owning-task invocation passed; strict core Clippy and workspace formatting checks passed.

Readiness now uses a unique token and a five-second channel deadline. The stdout reader retains diagnostics, and the error path kills and reaps the fixture's children before reporting them. The hosted Ubuntu run canceled during this investigation had already passed its memory tests and coverage; it was still compiling other checks. The local forced single-thread reproduction establishes this separate fixture defect without claiming the hosted job had hung.
