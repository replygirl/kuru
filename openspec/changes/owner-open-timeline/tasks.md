# Tasks

Work in `tmp/worktrees/perf-owner-start-split` (branch `perf/owner-start-split`, from `origin/main` 1c93476f). WP1, WP2 and WP3 own disjoint files. Never commit `mise.lock`; never bypass hk hooks. Nothing from R1-R3 is implemented. Tasks are ticked as evidence lands (see `verification.md`).

## 1. Change artifacts (WP0)

- [x] 1.1 Author proposal, blocking-changes, specs, design, tasks and verification and run `mise run cospec -- validate owner-open-timeline --strict`, and verify it reports no error. Observed 2026-10-01 (macOS): `0 errors, 0 warnings — validation passed`.
- [x] 1.2 Run `mise run cospec -- apply owner-open-timeline --json` and verify the exit code is 0, or that every soft blocker (exit 3) is resolved or acknowledged in writing. Observed 2026-10-01: gate state `clear`, exit 0, no blockers.

## 2. Timeline module and stamp sites (WP1: `src/open_timeline.rs`, `lib.rs`, `service.rs`, `service/open_timeline_tests.rs`, `service/activity.rs` helper only, `server.rs`, `store.rs`, `store/usage_ledger.rs`, `provision.rs`)

- [x] 2.1 Add `open_timeline.rs` (gate, `Timeline`, bounded sealed log, `encode`, checked `write`) declared in `lib.rs`, and verify unit tests 1-5 (non-decreasing and bounded across threads and tasks, seal, gate values, encode schema and size, write modes and failures) pass using local `Timeline` values only. Observed 2026-10-01 (macOS arm64): red with stub bodies (10 of the 11 `open_timeline::tests` failed; `an_uninstalled_process_records_nothing` passed, as an inert runner must), then green, 11 passed.
- [x] 2.2 Install from the environment as the first statement of `service_entry` and place the 18 stamps at the sites in design D2 (one site per event, no `allow(dead_code)`), changing `validate_branch` to return its row count, and verify `mise run //packages/kuru-memory:test` still passes the unmodified stage-sequence tests. Observed 2026-10-01 (macOS arm64): full package suite 514 passed, 0 failed, 4 ignored (lib), every other target ok; `a_starter_forwards_only_its_own_owners_stages`, `_the_creation_of_a_new_project` and `_the_upgrade_of_a_released_store` ok.
- [x] 2.3 Write the timeline in `close_paused` after `lock.release()` with the `ServeKnobs.timeline` test knob, and verify test 6 (no file at any close pause, one file after) and test 7 (an occupied name leaves the close `Ok`). Observed 2026-10-01 (macOS arm64): test 6 red (no file after close), then green; test 7 passed in both phases (nothing was written to fail before the write existed) and green after.
- [x] 2.4 Make the owner-environment helper crate-visible and add `FixtureLoggedOwner::exited` (unix), and verify tests 8 (complete ordered record, no forbidden bytes) and 9 (unset writes nothing) pass. Observed 2026-10-01 (macOS arm64): test 8 red (no file), then green with the 18 canonical events in order from a real child owner; test 9 passed in both phases and green after.
- [x] 2.5 Verify `mise run //packages/kuru-memory:lint`, `lint:windows`, `format:check` and `typecheck` pass, so Windows-only code compiles. Observed 2026-10-01 (macOS arm64): `//packages/kuru-memory:lint`, `//packages/kuru-memory:lint:windows`, root `lint`, `format:check` and `typecheck` exited 0.

## 3. Aged-store fixture (WP2: `src/test_support/aged_store.rs`, `src/test_support.rs`, `src/main.rs`, `packages/kuru-memory/mise.toml`)

- [x] 3.1 Add `aged_store` (argument parser, SplitMix64, `age_open_store`, `claim` and `age_claimed` in place of `run`, `main`), the feature-gated `age-store` arm and the `measure:age-store` task, and verify test 10 (determinism, counts, wrapper) passes. Observed 2026-10-01 (macOS arm64): `//packages/kuru-memory:test -- -- aged_store no_spawn_guard_encloses` 8 passed, 0 failed (the 6 `aged_store` tests, the spawn-gate scan and the scope test the filter also matches).
- [x] 3.2 Run the 100-conversation pilot in three parallel processes, apply the D7 rule, and record `r` and the chosen ladder in `tmp/roadmap/unit6b-notes-2026-10-01.md`. Observed 2026-10-01 (macOS arm64): `r` = 84.3-84.7 writes/s per process (1,000 writes in 11.91-11.97 s each), so `C_max = 270·r` ≈ 22.8k at 10 writes per conversation and the ladder is 1k, 5k, 20k; aging times are in the notes.

## 4. Documentation (WP3: `docs/development.md`)

- [x] 4.1 Document the variable, file, schema, 18 event names, accumulation, the Windows non-support and `measure:age-store`, and verify `mise run docs:check` passes. Observed 2026-10-01: the owner open timeline paragraph (WP3) and the `measure:age-store` paragraph beside `measure:lifecycle` (WP2) landed; `docs:check` exited 0 after each.

## 5. Measurement and report (WP4, WP5: outside the repository)

- [ ] 5.1 Build the release binary and run the macOS series (fresh and aged cases, 10 interleaved samples each, plus the instrument A/B), and verify every number in the notes traces to a record under `tmp/roadmap/unit6b-evidence/macos/`.
- [ ] 5.2 Append the ranking, the growing rows and the one-paragraph unit note for each growing row to `tmp/roadmap/unit6b-notes-2026-10-01.md`, with measured and inferred labelled and unrun checks named.

## 6. Archive

- [ ] 6.1 Complete the tasks above, run `mise run cospec -- validate owner-open-timeline --strict`, run `mise run cospec -- archive owner-open-timeline`, and verify the archive directory exists and no active record remains on the branch before merge.
