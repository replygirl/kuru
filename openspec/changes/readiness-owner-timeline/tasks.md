# Tasks

Work in `tmp/worktrees/feat-readiness-owner-timeline` (branch `feat/readiness-owner-timeline`, from `origin/main` `1752e076`). Never commit `mise.lock`; never bypass hk hooks; every command runs through mise. Tasks are ticked as evidence lands (see `verification.md`).

## 1. Change artifacts

- [x] 1.1 Author proposal, blocking-changes, specs, design, tasks and verification, run `mise run cospec -- validate readiness-owner-timeline --strict`, and verify it reports no error. Observed 2026-10-02 (macOS arm64): `0 errors, 0 warnings — validation passed`, after adding the `## Operational surface` section the checked deploy surface requires.
- [x] 1.2 Run `mise run cospec -- apply readiness-owner-timeline --json` and verify the exit code is 0, or that every soft blocker (exit 3) is resolved or acknowledged in writing. Observed 2026-10-02: exit 0, gate state `clear`, no hard or soft blockers.

## 2. Red tests

- [ ] 2.1 Add T3 `service::tests::owner_timeline_clause_text_is_stable` against a stub clause, and verify it fails on its exact-text assertion, with the message recorded in the commit body.
- [ ] 2.2 Add T4 `service::tests::readiness_deadline_names_the_owner_event_it_was_held_at` (Unix, real owner) with the `create-start` FIFO hold, and verify it fails at its inhibitor's bound with the "never reached the create-start hold" diagnostic, recorded in the commit body.

## 3. Owner stream and events (`open_timeline.rs`, `service.rs`, `store.rs`, `provision.rs`, `server.rs` stamps)

- [ ] 3.1 Add `gate_set`, `Timeline::stream_to`, per-stamp stream lines and `end_stream`, and verify T1 `the_stream_mirrors_the_log_line_by_line` and T2 `the_stream_is_create_only_private_and_removed` pass.
- [ ] 3.2 Add the eleven events at their single sites and the stream lifecycle in `open_hooked` (after `lock.verify()`, removal after publication and on both failed-open paths), and verify T8 (extended `a_gated_owner_writes_one_complete_record`) and the extended event-list fixtures pass.
- [ ] 3.3 Add the Unix test-support FIFO hold under `KURU_TEST_MEMORY_TIMELINE_HOLD_DIR`, consulted only by gated owners, and verify T4 passes.

## 4. Starter clause and Windows forwarding (`service.rs`)

- [ ] 4.1 Implement `owner_timeline_clause` and the gated deadline read (spawn instant taken before `spawn_service`), and verify T3 passes and the unchanged stalled-owner split tests pass.
- [ ] 4.2 Extract `owner_environment` and forward the gate only when exactly `1` through `merge_environment` layers, and verify T6 `the_owner_environment_forwards_only_an_exact_gate` passes in `lint:windows` compilation and on Windows CI.

## 5. Supervisor readiness part (`server.rs`)

- [ ] 5.1 Split the supervisor readiness deadline by part under the unchanged outer cause and classify a Windows accept `TimedOut` at or after the deadline as the accept part, and verify T5 (Unix end-to-end and the pure classifier on every OS) and T7 (extended `startup_log_capture_is_opt_in_exact_and_bounded`) pass.

## 6. Enablement, fixtures and text

- [ ] 6.1 Pin `KURU_OPEN_TIMELINE=0` in the ungated timeline and usage-scan tests, set the gate in the `coverage:shard` task environment, and forward it in the env-clearing kuru-tui fixtures, and verify the affected suites pass gated and ungated.
- [ ] 6.2 Update `docs/development.md` and the in-code text that calls the timeline inert on Windows or close-only, and verify `mise run docs:check` passes.

## 7. Verification and archive

- [ ] 7.1 Run the package and app suites, `format:check`, `lint`, root `lint:windows`, `typecheck`, `docs:check`, `cospec:managed:check`, and the full `kuru-memory` suite with `KURU_OPEN_TIMELINE=1` exported, and verify each exits 0, recording results in `verification.md`.
- [ ] 7.2 Complete tasks, validate strictly and run `mise run cospec -- archive readiness-owner-timeline`, and verify the archive directory exists before the final commit.
