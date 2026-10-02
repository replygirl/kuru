# Verification

Authored 2026-10-02 before implementation; rows are ticked as evidence lands, with date and platform, measured and inferred kept apart. Tests wait for events or a paused clock, never sleep. R1-R4 spawn the prepared `test-support` snapshot as a stand-in owner through the task-local owner environment, so they run natively on Linux, macOS and Windows. CI reruns are not used as evidence.

## 1. An advancing owner is waited for beyond one window [critical]

- [ ] 1.1 @integration (agent) R1 `an_advancing_owner_is_waited_for_beyond_one_window`: paused clock, `startup_timeout_secs = 1`, the poll hook advances the record's count at polls 99, 198 and 297 and publishes a real listener and endpoint at poll 300 -> recorded poll instants are spawn + (k - 1) x 10 ms, the client attaches at poll 301, at least three windows elapsed from spawn, no error; red on the unchanged loop at the attach assertion
- [ ] 1.2 @integration (agent) R1 on Windows CI -> the same assertions on the named-pipe listener; if its accept does not complete under the resumed clock, the design's fallback (release the stand-in at poll 300, assert `OwnerExited` at poll >= 301) is applied and recorded here
- [ ] 1.3 @e2e (agent) `cli_memory_progress_is_bounded_and_keeps_json_on_stdout` and `cli_new_project_shows_*` through the real binary -> unchanged assertions pass (every stderr line is one of the five sentences)

## 2. A stalled or silent owner is abandoned one window after its last progress, naming its stage [critical]

- [ ] 2.1 @integration (agent) R2 `a_stalled_owner_is_abandoned_naming_its_stage`: paused clock, window 1 s, record `[ExtractingEmbeddedRuntime], progress=1` written at poll 50 -> `ReadinessFailure::Stalled { last_stage: Some(ExtractingEmbeddedRuntime), since: 1000 ms }` at poll 151 with the leading text `memory service readiness deadline exceeded`; red on the unchanged loop
- [ ] 2.2 @integration (agent) `readiness_deadline_reports_the_client_phase_split` and both held-lock variants (silent owner) -> pass with only the design's edits: readiness >= 1 s from spawn, phases sum within observed, `child=running`, `last-attach=no-endpoint`
- [ ] 2.3 @unit (agent) a stale same-tag record -> counted once and never again; a foreign, unreadable or concurrently replaced record moves nothing

## 3. An exited, retired or failing owner fails at once with its evidence [critical]

- [ ] 3.1 @integration (agent) R3 `an_exited_owner_fails_at_once_with_its_stage`: real clock, window 300 s, record `[CreatingDatabase], progress=1` at poll 5, release with status 3 at poll 6 -> `ReadinessFailure::OwnerExited { status 3, last_stage: Some(CreatingDatabase) }` far below the window; red on the unchanged loop at `last_stage`
- [ ] 3.2 @integration (agent) R4 `a_retired_record_fails_at_once`: paused clock, window 1 s, record written at poll 5, retired with the real `retire_record` at poll 7, stand-in alive -> `ReadinessFailure::OwnerRetired { last_stage: Some(CreatingDatabase) }` at poll 8 (70 ms); red on the unchanged loop
- [ ] 3.3 @integration (agent) failing mark: a real owner whose open fails after starting its engine -> the record carries `"failing": true` and the reason before `close_failed_open` closes the engine, and is retired after it; a client reading it fails at once with `memory service open failed before readiness` and that reason
- [ ] 3.4 @manual (agent) audit with file:line that every `close_failed_open` caller and the creation and migration worker failure paths always propagate their `Err` out of the open -> recorded here

## 4. Progress advances only at bounded one-shot points [critical]

- [ ] 4.1 @manual (agent) site audit with file:line: every `report` site, every milestone stamped inside the open, extraction and verification byte ticks, migration steps; and the audited non-sites (supervisor port retry, pool acquire loop, project startup lock wait after its one report, test hold loop) carry no advance -> recorded here
- [ ] 4.2 @unit (agent) two concurrent in-process opens -> independent counters; extraction -> one tick per 8 MiB
- [ ] 4.3 @integration (agent) the publisher through the `Writes` gate seam -> progress-only writes at most every 250 ms, stage changes at once, the trailing value always written
- [ ] 4.4 @benchmark (agent) a cold fresh open on the Windows runner -> the number of record writes and their total time, recorded here (design: about 100 advances per cold open, at most 4 writes per second)

## 5. Record format 2 and mixed versions

- [ ] 5.1 @unit (agent) `decode` -> format 2 accepted with `progress`, `failing` and `reason` round-tripping; format 1 and 3, unknown fields and an oversized record rejected
- [ ] 5.2 @manual (agent) `docs/release.md` -> both mixed-version directions stated

## 6. Documentation

- [ ] 6.1 @manual (agent) `docs/configuration.md` and `apps/kuru-docs/reference/configuration.md` -> the new meaning, the definition of progress, all four reports and the per-start bound; `docs/development.md` -> the progress field, the progress-point rule and the stand-in mode; `mise run docs:check` exits 0

## 7. Static checks and suites [critical]

- [ ] 7.1 @regression (agent) `mise run //packages/kuru-memory:test`, the `//apps/kuru-tui:test` starter binaries, `mise run //packages/kuru-core:test`, `format:check`, `lint`, `lint:windows`, `typecheck`, `docs:check`, `cospec:managed:check` on macOS -> all exit 0
- [ ] 7.2 @regression (agent) the PR's CI on Linux, macOS and Windows with the 90% line gate -> green without reruns
