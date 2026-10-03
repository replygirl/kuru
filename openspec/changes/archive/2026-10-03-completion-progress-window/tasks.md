# Tasks

## 1. Regression tests

- [x] 1.1 Route the turn and compaction model calls through one helper that keeps the previous flat 180 s body, add `completion_window_tests` (paused tokio clock, both call labels) and verify all three tests fail against that body: deltas past 180 s, silence after the last event, and a stream past the 600 s total

## 2. Fix

- [x] 2.1 Export `COMPLETION_TIMEOUT` and add the derived `STREAM_IDLE_TIMEOUT` in `kuru-connectors`, use it for the SSE idle bound, and verify `mise run //packages/kuru-connectors:test` passes
- [x] 2.2 Replace the helper body with the forwarding progress adapter, the no-progress watcher and the total budget (no 600/180 literal in actor.rs), and verify the three regression tests pass
- [x] 2.3 Document the silence budget and its derivation beside the 600-second budget in docs/protocols.md and verify `mise run docs:check` passes

## 3. Checks and record

- [x] 3.1 Run `mise run //packages/kuru-runtime:test`, `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` and `mise run cospec -- validate --all --strict`, and record exit codes in verification.md
