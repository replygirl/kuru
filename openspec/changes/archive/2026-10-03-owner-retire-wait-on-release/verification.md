# Verification

## 1. An elapsed retirement bound names the owner it waited behind [critical]

- [x] 1.1 @regression (agent) `service::activity::tests::an_elapsed_retirement_behind_an_unpublished_owner_names_its_open_stage`: a tokened owner held before publication at `PreparingDatabase` (owner lock held, no endpoint); paused time elapses the fixture's 10 s -> observed 2026-10-04 macOS arm64: with the old expiry text FAILED `the elapsed bound did not name the opening owner's stage: idle managed owner did not retire within 10 seconds; maintenance waiting for the owner lock for 2ms (this lock's wait 10001ms); requests without a live endpoint=100; ...` (the same text shape as the CI occurrence of a published owner's close); with the fix passed
- [x] 1.2 @regression (agent) `service::activity::tests::an_elapsed_retirement_behind_a_held_close_names_a_closing_owner`: a served tokened owner held at `ClosePoint::AfterReap` after its starter detached; paused time elapses the bound; releasing the pause ends the close and a second `retire_idle_service` completes -> observed: with the old text FAILED `the elapsed bound did not name a closing owner: ...`; with the fix passed
- [x] 1.3 @unit (agent) `test_support::tests::an_elapsed_retirement_bound_names_the_step_it_was_cancelled_in` (bare owner lock, no records) now also asserts `owner closing; last phase = endpoint and activity records retired` -> observed: failed with the old text, passed with the fix
- [x] 1.4 @integration (agent) `mise run //packages/kuru-memory:test -- an_elapsed_retirement` three times -> observed: three runs, each `3 passed; 0 failed` (3.57 s, 3.28 s, 2.10 s)

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //packages/kuru-memory:test` (full) -> observed 2026-10-04 macOS arm64: exit 0, lib 700 passed, 0 failed, 6 ignored (1158.4 s); bundle_build 10, memory 5, server_lifecycle 12, supervisor_snapshot 1 passed. The run compiled before a formatting-only rewrite of `owner_state`'s stage text; the three targeted tests were re-run after it: `3 passed; 0 failed` (3.00 s)
- [x] 2.2 @integration (agent) `cargo test -p kuru --all-features --locked --test terminal real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds` under `mise exec` with the TUI test task's env, three times -> observed: three runs, each `1 passed; 0 failed` (12.04 s, 10.33 s, 20.96 s)
- [~] 2.3 @runtime (agent) the flat 10 s replaced by an event wait on the owner's lock release -> defer: blocked; `ServiceCleanup::retire` (apps/kuru-tui/tests/support/memory.rs) and mise acceptance `retire_blocking` join their cleanup threads with no outer backstop, so the lead's rule (no raised or guessed bound) leaves the bound unchanged; candidate shapes recorded in tmp/roadmap/owner-retire-bound-2026-10-04.md

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed: each exit 0 (run with `NODE_OPTIONS` unset; the shell's preload breaks the docs toolchain step); validate 0 errors, 0 warnings
- [~] 3.2 @runtime (agent) native coverage partitions in PR CI -> defer: PR CI runs after the branch is pushed; CI reruns are not used as evidence here
