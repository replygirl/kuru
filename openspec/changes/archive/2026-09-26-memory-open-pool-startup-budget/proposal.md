## Why

A fresh memory store open relaunches Dolt several times (staged initialization, staged migration, migrated staged activation and the active server). Each `Server::open_with_guard` bounds readiness, its first authenticated probe and that probe's identity check by one startup deadline (`startup_timeout_secs` plus the two-second supervisor-transport allowance), but the very next pools the open sequence requests from that just-started server (`open staged main pool`, `open migrated staged main pool`, the migration-attempt, historical-classification, candidate-recovery and usage-ledger branch pools) use the flat two-second ordinary acquisition window and a flat two-second identity query window. On a loaded native Windows runner a just-relaunched, healthy Dolt server misses those windows, so `MemoryStore::temporary()` failed in #91's Windows coverage shard (run 36218920182) with `open migrated staged main pool` / `verify memory branch pool identity` / `pool timed out while waiting for an open connection`, and with a bare `authenticate memory branch pool` / `connection phase: after_connect not entered` / the same pool timeout. The flaw has existed since #82 introduced the remaining-budget probe and was exposed on this stack by #89's PowerShell hook launches loading the runner.

## What Changes

- A server started by the owned open path remembers the exact startup deadline its post-readiness probe used. Until the store open that created it completes, every branch pool the server authenticates acquires and identity-verifies within the later of the ordinary two-second window and that remaining startup deadline. The ordinary window stays the floor, so a slow but healthy start cannot leave the next open-sequence pool with less than it had before.
- The store ends the opening phase immediately before reporting `Ready`; afterwards pool acquisition and identity verification are exactly the ordinary two-second windows. A borrowed read-only endpoint never enters the opening phase.
- During the opening phase, the two authored identity rejections (data-directory mismatch, SQL project/instance mismatch) end acquisition immediately instead of being retried by SQLx for the extended window. Credential and handshake rejections are already terminal in SQLx and remain so. Ordinary post-open attempts keep their existing failure mode.
- On Windows, the private supervisor pipe accept during owned startup is bounded by the same remaining startup deadline instead of a separate flat five seconds.
- No new timeout constant: the existing two-second ordinary literals are named once and reused.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The living `versioned-memory` requirement already requires bounded waits and verified store identity; only the implementation used the wrong bound for open-sequence pools.

## Impact

- `packages/kuru-memory/src/server.rs`: opening-phase deadline on the server, pool window selection, opening-phase identity fail-fast, Windows accept bound, test seam for pool authentication delay.
- `packages/kuru-memory/src/store.rs`: end the opening phase before `Ready`; test-only option to delay the migrated staged reopen's main pool; regression test.
- No public API, configuration, documentation or schema change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
