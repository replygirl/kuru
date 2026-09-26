## Why

The archived fix `2026-09-26-memory-open-pool-startup-budget` built every pool requested while a store was opening with an SQLx `acquire_timeout` of up to the remaining startup deadline. In SQLx 0.9 that value is a property of the pool's whole lifetime, used by every later `acquire`, so the retained active main pool and usage-ledger pool kept a roughly 30-second acquire window after `Ready`. Later reconnects on those pools could also retry an identity rejection for that long. This contradicts that record's claim that post-open pool behavior is exactly the ordinary two seconds.

## What Changes

- Every pool `Server::pool` builds keeps the ordinary two-second `acquire_timeout` for its whole lifetime.
- The opening phase extends only the pool's first acquisition: the pool is created lazily and its first `acquire` is retried after each ordinary-window timeout until the later of the ordinary window and the remaining startup deadline. The first authored identity rejection ends it.
- The post-readiness probe keeps its single remaining-budget attempt; its transient pool is closed immediately after verification.
- This corrects the archived record's "unchanged after open" guarantee; identity fail-fast during open, the Windows accept bound and `finish_opening` placement are unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The living spec is unchanged; only the implementation and the archived change record's claim were wrong.

## Impact

- `packages/kuru-memory/src/server.rs`: lazy pool construction and bounded first-acquire retry; test seam models a handshake stalled until one instant.
- `packages/kuru-memory/src/store/open_pool_budget_tests.rs`: retained-pool lifetime test.
- No public API, configuration, documentation or schema change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
