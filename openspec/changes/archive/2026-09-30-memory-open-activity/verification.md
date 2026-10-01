# Verification

Authored 2026-09-30 before implementation. Rows are ticked as evidence lands, with the date and platform. Windows behaviour is unverified until native CI. Tests wait for events (a stage received, a stderr line read, a completed frame), never sleep.

## 1. A new project shows opening then creating, with no label or ready line [critical]

- [x] 1.1 @integration (agent) T1 `kuru-memory`: real spawned owner, hold on `CreatingDatabase` -> `StartingMemoryService` before `CreatingDatabase`, `Ready` last and once, no `UpgradingDatabase`, no `WaitingForProjectOwnership`; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 1.2 @e2e (agent) T14 `kuru run --json` on a new project, non-terminal stderr, empty private `cache_dir`, holds on `ExtractingEmbeddedRuntime` and `CreatingDatabase` -> stdout parses as JSON; stderr first line S1, S2 once before S3, S3 once and last, every line one of S1-S5, no repeated line, no ready line, no `Memory:`; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 1.3 @e2e (agent) T17 PTY `smoke`, first then second run -> bytes before the first `\x1b[?1049h` hold S1 (and S3 on first run), second run has no S3, after the last sentence and its padding the line is erased with `\r`, spaces, `\r`, and no sentence bytes follow; the first-run notice may follow the erase; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 1.4 @unit (agent) T12 `next_sentence` against R1-R9 over cold cache, warm cache, configured binary, upgrade inside creation, upgrade after an engine change, waiting then start, unknown stage -> expected sentences; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)

## 2. Reopening shows only the opening sentence [critical]

- [x] 2.1 @integration (agent) T3 existing current store, no owner -> neither creating nor upgrading is received; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 2.2 @e2e (agent) T15 reopen after awaiting the previous owner's exit -> lines before the result are exactly `[S1]`; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 2.3 @integration (agent) T2 store created by `released_v1`, hold on `UpgradingDatabase` -> received before `Ready`, `CreatingDatabase` absent; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 2.4 @integration (agent) T11 `store.rs` -> uncontended open has no waiting stage; contended holds the startup lock, waiting stage received, open succeeds after release; in-process upgrade reports `UpgradingDatabase` once before `Ready`; `acquire_lock`'s existing tests unchanged; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)

## 3. The waiting sentence appears only while another copy holds the project [critical]

- [x] 3.1 @integration (agent) T6 test holds the Owner lock, start the open -> `WaitingForProjectOwnership` received, after release `StartingMemoryService` then success; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 3.2 @integration (agent) T6b read-only open through `attach_existing` -> same, then the in-process fallback stages and success; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 3.3 @e2e (agent) T16 existing project, test holds the owner lock -> stderr reaches S5, after release S1 follows and the command succeeds; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 3.4 @integration (agent) T19 in-process owner with a close pause at `AfterEndpointRetire`, then a second real `open_managed_observed` -> `WaitingForProjectOwnership` while held, then `StartingMemoryService` and `Ready` after release; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)

## 4. The activity record grants no authority and cannot fail the open [critical]

- [x] 4.1 @unit (agent) T4 unit half -> `forward_new` ignores a record written by the real publisher code under a different tag, `forwarded` unchanged; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 4.2 @integration (agent) T4 integration half: plant a foreign-tag record with `["UpgradingDatabase"]` and set the write-failure hook -> open succeeds, `UpgradingDatabase` never received; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 4.3 @integration (agent) T5 write-failure hook -> open succeeds and stages are exactly `[StartingMemoryService, Ready]`; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 4.4 @unit (agent) T9 codec -> round trip, unknown field, unknown stage name, over 4 KiB, wrong tag, raw-token tag rejected; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 4.5 @unit (agent) an owner opened with `starter_token = None` publishes no record -> record absent; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [~] 4.6 @integration (agent) T8 native on every CI platform: hold the record open, replace it, then retire while held expecting: replacement succeeds and reads back, the name is free at once for a fresh write, stage removal may be deferred; not yet run; Windows only in native CI -> defer: native CI only (Windows); not run locally

## 5. Retirement runs inside the owner's close, never detached

- [x] 5.1 @integration (agent) `close_paused` ordering: with `ClosePause` at `AfterEndpointRetire` -> the record is retired after the endpoint and before the store closes while the Owner lock is still held; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 5.2 @integration (agent) both error returns of `ServiceOwner::open` (store-open failure, `prepared` error branch) -> no record of this owner's tag remains; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 5.3 @unit (agent) `activity::retire` with a foreign-tag record present -> the foreign record is left and only the own-tag record is removed; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)

## 6. Sentence rendering on terminals and pipes

- [x] 6.1 @unit (agent) T13 renderer against a buffer -> `\r` rewrite with `width_cjk` padding; truncation keeps the ellipsis and no line exceeds `cols - 1` at 20, 3 and 2 columns; nothing when no room; erase on complete and abandon; non-terminal one line per change, whole lines, nothing at ready; failing sink switches off; N2 versus N2'; no `Memory:`; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 6.2 @e2e (agent) T18 Unix PTY via `/bin/sh -c 'exec "$0" 2>"$1"'` -> PTY bytes before the alternate screen contain S1 and the stderr file contains none of S1-S5; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [~] 6.3 @manual (human) run `kuru` on a terminal with an empty cache and a new project, resize narrow expecting: the sentence rewrites in place, stays on one line with its ellipsis and is gone before the interface appears; not yet run -> defer: manual human terminal check; not run

## 7. Marker lines for the open-time harness [critical]

- [x] 7.1 @e2e (agent) child test `KURU_OPEN_MARKERS=1 kuru run --json` on a new project, non-terminal stderr -> `open-start` is the first stderr line, `ready` appears once after every sentence, no `waiting-ownership`, stdout parses as JSON; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 7.2 @e2e (agent) same command with the variable unset -> no `kuru-open-marker` byte anywhere; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 7.3 @unit (agent) T13 marker cases -> three events in order with non-decreasing ns, `waiting-ownership` only when R1 fired and once, a terminal buffer never has a marker on a sentence's line and the last visible line after ready is empty; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)
- [x] 7.4 @e2e (agent) held owner lock with markers on -> exactly one `waiting-ownership` marker; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)

## 8. Documentation matches the behaviour

- [x] 8.1 @regression (agent) `mise run docs:check` and `mise run docs:build` -> pass, and a grep of `docs/` and `apps/kuru-docs/` finds no `Memory:` progress string and no `KURU_OPEN_MARKERS` outside `docs/development.md`; observed 2026-09-30 on macOS (unit2-implementation-notes-2026-09-30.md)

## 9. Open time is not slower

- [~] 9.1 @benchmark (agent) release-smoke medians for first launch and existing project, each iteration awaiting the previous owner's exit expecting: within noise of 6173 ms and 628 ms; not yet run -> defer: release-smoke medians need a release build and quiet host; not run
- [x] 9.2 @regression (agent) `//packages/kuru-memory:test`, `//apps/kuru-tui:test`, `lint` (host and Windows target), `format:check`, `typecheck` -> pass; observed 2026-09-30 on macOS: kuru-memory 449 lib tests and the other targets, kuru-tui 274 tests, lint, format:check, typecheck, all exit 0 (Windows-target lint last run at WP-D; no Rust changed since)

## 10. Windows hook forwarding

- [~] 10.1 @runtime (agent) native Windows CI: the two test-support variables reach the owner and T8 passes expecting: observed in CI only; not yet run, unverified on macOS -> defer: native Windows CI only; unverified on macOS

## Rebased onto origin/main 52f9869d (2026-09-30, macOS, MISE_LOCKED=1)

Measured: `//packages/kuru-memory:test` 469 passed, 0 failed, 4 ignored, exit 0; `//apps/kuru-tui:test` exit 0, 0 failed; `lint`, `lint:windows`, `format:check`, `typecheck`, `docs:check` exit 0; `cospec validate --strict` 0 errors, 0 warnings. Not run: native Linux/macOS/Windows CI, Windows behaviour, release-smoke medians (9.1), manual terminal check (6.3). Rows 4.6, 6.3, 9.1 and 10.1 stay open and are deferred at archive.

## Review follow-up (2026-10-01, macOS, MISE_LOCKED=1)

Docs only: the output rule in `apps/kuru-docs/concepts/memory.md`, `apps/kuru-docs/reference/configuration.md` and `docs/configuration.md` now states the non-terminal line case and the interactive redirected-stderr case, matching `docs/memory.md`. Measured: `docs:check`, `format:check`, `lint`, `typecheck` exit 0. No Rust changed; package tests not rerun.
