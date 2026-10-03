# Verification

## 1. An elapsed retirement bound names the owner it waited behind [critical]

- [x] 1.1 @regression (agent) `service::activity::tests::an_elapsed_retirement_behind_an_unpublished_owner_names_its_open_stage`: a tokened owner held before publication at `PreparingDatabase` (owner lock held, no endpoint); paused time elapses the fixture's 10 s -> observed 2026-10-04 macOS arm64: with the old expiry text FAILED `the elapsed bound did not name the opening owner's stage: idle managed owner did not retire within 10 seconds; maintenance waiting for the owner lock for 2ms (this lock's wait 10001ms); requests without a live endpoint=100; ...` (the same text shape as the CI occurrence of a published owner's close); with the fix passed
- [x] 1.2 @regression (agent) `service::activity::tests::an_elapsed_retirement_behind_a_held_close_names_a_closing_owner`: a served tokened owner held at `ClosePoint::AfterReap` after its starter detached; paused time elapses the bound; releasing the pause ends the close and a second `retire_idle_service` completes -> observed: with the old text FAILED `the elapsed bound did not name a closing owner: ...`; with the fix passed
- [x] 1.3 @unit (agent) `test_support::tests::an_elapsed_retirement_bound_names_the_step_it_was_cancelled_in` (bare owner lock, no records) now also asserts `owner closing; last phase = endpoint and activity records retired` -> observed: failed with the old text, passed with the fix
- [x] 1.4 @integration (agent) `mise run //packages/kuru-memory:test -- an_elapsed_retirement` three times -> observed: three runs, each `3 passed; 0 failed` (3.57 s, 3.28 s, 2.10 s)

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test` (full) -> observed 2026-10-04 macOS arm64: exit 0, lib 700 passed, 0 failed, 6 ignored (1158.4 s); bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1 passed. The run compiled before a formatting-only rewrite of `owner_state`'s stage text; the three targeted tests were re-run after it: `3 passed; 0 failed` (3.00 s)
- [x] 2.2 @integration (agent) `cargo test -p kuru --all-features --locked --test terminal real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds` under `mise exec` with the TUI test task's env, three times (the exact command run was that `cargo test` invocation, not the `mise run //apps/kuru-tui:test` task; see 4.3 for the task run) -> observed: three runs, each `1 passed; 0 failed` (12.04 s, 10.33 s, 20.96 s)
- [~] 2.3 @runtime (agent) the flat 10 s replaced by an event wait on the owner's lock release -> defer: blocked; `ServiceCleanup::retire` (apps/kuru-tui/tests/support/memory.rs) and mise acceptance `retire_blocking` join their cleanup threads with no outer backstop, so the lead's rule (no raised or guessed bound) leaves the bound unchanged; candidate shapes recorded in tmp/roadmap/owner-retire-bound-2026-10-04.md

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed: each exit 0 (run with `NODE_OPTIONS` unset; the shell's preload breaks the docs toolchain step); validate 0 errors, 0 warnings
- [~] 3.2 @runtime (agent) native coverage partitions in PR CI -> defer: PR CI runs after the branch is pushed; CI reruns are not used as evidence here

## 4. Review corrections

- [x] 4.1 @regression (agent) the three elapsed-bound tests require `managed owner retirement did not complete within 10 seconds`, no `idle managed owner`, and (opening reading) `no endpoint record present` -> observed 2026-10-03 macOS arm64, `mise run //packages/kuru-memory:test -- an_elapsed_retirement` with the old expiry text: exit 101, `0 passed; 3 failed`, e.g. `the elapsed bound did not name the opening owner's stage: idle managed owner did not retire within 10 seconds; owner still opening; last stage = PreparingDatabase; no endpoint published; ...`
- [x] 4.2 @integration (agent) the same command with the fix, three times -> observed: each exit 0, `3 passed; 0 failed` (4.02 s, 2.47 s, 2.67 s)
- [x] 4.3 @integration (agent) `mise run //apps/kuru-tui:test -- real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds` (the task's `cargo test -p kuru --all-targets ...` with that name filter; it ran in `tests/terminal.rs`) three times -> observed: each exit 0, `1 passed; 0 failed; 43 filtered out` (18.00 s, 15.26 s, 14.97 s)
- [x] 4.4 @unit (agent) after 4.2, `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed 2026-10-03: each exit 0 (run with `NODE_OPTIONS` unset); validate 0 errors, 0 warnings
- [~] 4.5 @integration (agent) full `mise run //packages/kuru-memory:test` after 4.2 -> defer: not rerun; the change after 2.1's full run is two error-string literals and their assertions, covered by 4.2
