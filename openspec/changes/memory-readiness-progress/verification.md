# Verification

## 1. Creation span reports progress [critical]

- [x] 1.1 @regression (agent) `cargo test -p kuru-memory --lib creation_progress_continues_until_the_held_stage_engine_spawn` with a real gated owner and Dolt, before and after the fix -> before (unchanged src at b1339bdc, macOS arm64): FAILED in 61.11 s with `the owner held at its first supervisor-spawned advanced no further than the OpeningDatabase report's count 20 (CreatingDatabase at 19): the creation span reports no progress within 30 s; last record read: Some(Activity { stages: [VerifyingRuntimeCache, PreparingDatabase, CreatingDatabase, OpeningDatabase], progress: 20, failure: None })`; after: passes (with `readiness_deadline_names_the_owner_event_it_was_held_at`, the timeline and progress tests: 32 passed in 13.61 s), asserting the starter's `Stalled` count is past the `OpeningDatabase` report's, stage `OpeningDatabase`, `since >= window`, `last=supervisor-spawned` with `create-start`, `legacy-prepared`, `recovery-checked`, `copy-end`, `spawn-call` before it and one `supervisor-spawned`.
- [x] 1.2 @integration (agent) `mise run //packages/kuru-memory:test` on the host -> lib 821 passed, 0 failed, 6 ignored (1358.20 s), integration binaries 10/5/12/1 passed, on macOS arm64 with the shared verified Dolt 2.3.5 cache. `//apps/kuru-tui:test` not run: nothing the TUI consumes changed (no public API, stage or record format change).
- [ ] 1.3 @integration (agent) PR CI native Linux, macOS and Windows memory jobs and the 95% coverage gate -> green on the final head.

## 2. Boundary preservation

- [x] 2.1 @unit (agent) `mise run lint:rust`, `mise run lint:windows`, `mise run typecheck` -> all exit 0; the Windows `spawn-call`, `supervisor-spawned` and `channel-accepted` sites compile as milestones.
- [~] 2.2 @integration (agent) hold a gated owner at `channel-accepted` -> defer: the event is stamped only by a Windows supervisor start and the fifo event hold is Unix-only.
- [x] 2.3 @manual (agent) `mise run docs:check`, `mise run format:check`, `mise run cospec -- validate memory-readiness-progress --strict`, `mise run cospec:managed:check` -> all pass.
