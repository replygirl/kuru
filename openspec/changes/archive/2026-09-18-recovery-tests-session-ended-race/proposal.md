## Why

`store::recovery_tests::absent_fast_forward_keeps_the_same_ready_attempt_for_next_open`
failed intermittently on Windows CI (`assertion failed:
proxy.session_ended.load(Ordering::Acquire)`, run 35382536799, job
105721841828). The `session_ended` flag is written from the `AckDropProxy`'s
own spawned task, a task distinct from the task driving `MemoryStore::open()`
that the failing assertion runs alongside. The proxy sets the flag only after
it observes the routed SQL session's real server-side teardown
(`await_session_end` polling `information_schema.processlist`), which can lag
the socket shutdown that unblocks `open()` by an unbounded, though
deadline-bounded, amount of wall-clock time. Four assertions across three
tests in this file read `session_ended` with a bare synchronous `.load()`, so
each races against the proxy's own task with no happens-before edge between
them — a fixture bug, not a defect in `MemoryStore::open()` reconciliation.

## What Changes

Add a bounded polling helper (`await_flag`), mirroring the bounded
`durable_observation` poll already used in this same file for an analogous
out-of-band-condition wait, and use it at every `session_ended` assertion
site instead of a bare `.load()`. The sibling `discarded` assertions are
unchanged: `discarded` is set via `compare_exchange` strictly before the
socket shutdown that unblocks `open()`, so it has a true happens-before edge
and is safe to read synchronously. No production code changes; this is a
test-fixture-only fix.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None — the fixture race is corrected in place; no capability spec was wrong.

## Impact

`packages/kuru-memory/src/store/recovery_tests.rs` only: one new private test
helper (`await_flag`) and four call sites
(`production_upgrade_reconciles_lost_commit_reply_after_routed_session_ends`,
`production_upgrade_reconciles_lost_branch_reply_after_exact_ref_creation`,
`production_upgrade_reconciles_lost_fast_forward_reply_after_target_publication`,
`absent_fast_forward_keeps_the_same_ready_attempt_for_next_open`) changed from
a synchronous assert to a bounded await. No product code, no public API, no
migration.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
