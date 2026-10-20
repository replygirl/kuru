# Tasks

## 1. Progress across the creation span

- [x] 1.1 Make `supervisor-spawned` (Unix and Windows), `channel-accepted`, `template-copied` and `activated` progress milestones, keeping their stamps, and verify by review and host/Windows lint.
- [x] 1.2 Advance the open's count when a template copy completes and when the stage identity is written, and give the creation worker's stage start a counting reporter without a stage sender; verify the client's stage sequence tests still pass.
- [x] 1.3 Add `legacy-prepared`, `recovery-checked`, `copy-end` and `spawn-call` open-timeline milestones and update the canonical event lists and timeline tests; verify `open_timeline` and `open_timeline_tests` pass.

## 2. Regression and documentation

- [x] 2.1 Add `creation_progress_continues_until_the_held_stage_engine_spawn`: hold a gated owner at `CreatingDatabase` for a baseline and at its first `supervisor-spawned`, assert the starter's count passes the `OpeningDatabase` report's and that the held owner still stalls naming that event; observe it fail before the fix and pass after.
- [x] 2.2 Update `docs/development.md` (progress points, canonical events, reading `last`) and run the package tests, lint, typecheck, docs and cospec checks.
