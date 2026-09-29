# Proposal

## Why

The memory service answers an outcome (reconcile) query with in-flight whenever the original request's receipt key is still registered as running, before it looks at storage. A client that lost a reply therefore gets in-flight even when the committed receipt is already durable and visible, and every lost-reply test has to wait for the owner's `replied` signal or poll the outcome in 20 ms loops to get a definite answer. The same bookkeeping also records a handler that was dropped or aborted as `Completed`, so a later guarded miss in the same service generation could report absent although the aborted effect may still start or be half-applied.

The living `durable-service-receipts` spec permits both behaviours: its "Work remains in flight" scenario allows in-flight or still uncertain whenever a query races a worker, and nothing requires the service to give a definite answer that already exists.

## What Changes

- An outcome query for a main-view unit write or a usage-ledger record first makes a bounded lock-free read of the exact receipt. A matching receipt is reported as committed at once, even while the original worker is still registered; a mismatching receipt is still the `ReceiptConflict` fault.
- Otherwise, while the worker is registered, the owner waits for that worker to settle, inside a handler deadline derived from and lower than the existing `OPERATION_TIMEOUT`, without holding the write guard or a store connection, then re-probes (where allowed) and runs the existing guarded read. It answers in-flight only when that wait ends first.
- Candidate-view units, candidate creation, candidate transitions and selected abandon take no lock-free read. They wait for settlement and then read under the guard, preserving candidate-ref isolation.
- A negative or open answer comes only from a guarded read that began after the worker settled. A handler counts as settled only when it returned a value; a dropped or aborted handler leaves a bounded, sticky per-generation mark, so a same-generation miss is still uncertain.
- The waiting query watches its client stream. When the client disconnects, cancels or sends another frame, the wait ends at once and the attachment is released without a reply.
- The uncertain-write fence is unchanged: a genuinely uncertain outcome still reports still uncertain and still blocks further mutation on every handle of the logical client.
- Test accommodations that existed only to avoid the early in-flight answer are removed: the 13 `replied` waits, `ReplyBarrier::wait_replied`, `ReplyPause::replied` and four 20 ms outcome polls. The two raw-frame polls in the service tests stay, because their requests are not proven registered.
- No deadline is raised, no retry is added, `startup_timeout_secs` keeps its meaning, and no test sleeps.

## Capabilities

### New Capabilities

### Modified Capabilities

- `durable-service-receipts`: "Bounded four-state outcome reconciliation" gains the requirement to report a definite outcome whenever one exists, the settlement wait and its resource limits, the client-gone release, the aborted-handler rule, and the pre-existing unverified assumption about Dolt's process list that the fence already relies on. The "Work remains in flight" scenario is narrowed and four scenarios are added.

## Impact

- `packages/kuru-memory/src/service/rpc.rs`: `ReceiptProgress` settlement state and wait, `RunningReceipt::settle`, the four outcome handlers, `respond`'s stream bound and client-gone handling, new deadline constants derived from `OPERATION_TIMEOUT`, `QUERY_TIMEOUT` and `ORDINARY_POOL_WINDOW`, and test seams.
- `packages/kuru-memory/src/store.rs` and `store/usage_ledger.rs`: unguarded receipt and proof reads split from the guarded lookups; test-only apply hooks for the publication-proof test.
- Tests in `packages/kuru-memory/src/service.rs`, `facade.rs`, `test_support.rs`, `store/migrations.rs`, and `packages/kuru-runtime/src/hook_tests.rs` and `dream.rs`.
- Docs: `docs/development.md` (lost-reply test guidance and the memory service protocol) and `docs/memory.md`.
- The client wire protocol and the client side of reconciliation do not change. A reconcile issued while a fenced write is genuinely in progress can now take up to about 23 s before answering in-flight instead of answering at once; UI-path callers (`kuru-runtime` engine, `kuru-tui` CLI) see this only in that case.

## Surfaces

None is touched. This is an internal memory-service protocol behaviour and its tests. It changes no UI, CI topology, external contract or agent behaviour; the reconcile latency above is noted under Impact.

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
