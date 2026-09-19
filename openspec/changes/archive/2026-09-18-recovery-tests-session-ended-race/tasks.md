## 1. Bound the fixture's `session_ended` wait

- [x] 1.1 Add a private `await_flag` helper in `packages/kuru-memory/src/store/recovery_tests.rs`, mirroring the file's existing bounded `durable_observation` poll, and use it at all four `session_ended` assertion sites instead of a bare synchronous `.load()`; leave the diagnostic-only `.load()` inside the `with_context` format string at the top of `production_upgrade_reconciles_lost_commit_reply_after_routed_session_ends` unchanged (best-effort text on an already-failing path). Leave the `discarded` assertions unchanged — they have a true happens-before edge via `compare_exchange` before the socket shutdown that unblocks `open()`.
- [x] 1.2 Run the targeted regression tests three times to check stability, and record the observed results.
- [x] 1.3 Run format, the owning package's lint task, and `mise run cospec:validate`; record results. Validate and archive this change.
