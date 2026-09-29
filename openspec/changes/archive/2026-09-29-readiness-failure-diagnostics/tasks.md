# Tasks

## 1. Reproduce before changing

- [x] 1.1 Add `readiness_deadline_reports_the_client_phase_split` and run it against unchanged product code, and verify it fails because the error lacks `client phases:`
- [x] 1.2 Add the shared starter wait helper and `starter_wait_surfaces_the_exited_child_stderr`, run the test with the helper's stderr tail removed, and verify it fails on the missing child cause

## 2. Diagnostics

- [x] 2.1 Append the client phase split to the readiness deadline error in `attach_or_start` using the existing monotonic clock, and verify no deadline, poll, sleep or success-path IO changes in the diff
- [x] 2.2 Split `try_attach` into `try_attach_observed` with an attach outcome, keeping `try_attach` behaviour, and verify every existing caller compiles unchanged
- [x] 2.3 Give the Windows contained starter fixture a private stderr file and route both contained starter tests through the shared helper, and verify `lint:windows` compiles them
- [x] 2.4 Add `readiness_split_text_is_stable` and `contained_starter_failure_reports_its_stderr`, and verify the first passes locally

## 3. Documentation and verification

- [x] 3.1 Add one sentence on the deadline error next to the startup timeout range in both configuration pages, and verify `docs:check` passes
- [x] 3.2 Rerun the regression tests after the change and record them green
- [x] 3.3 Run `//packages/kuru-memory:test`, `lint`, `lint:windows`, `typecheck`, `format:check`, `lint:tooling`, `docs:check` and `cospec validate --strict`, and record observed results

## 4. Review round

- [x] 4.1 Correct the refactor type name in `proposal.md` to `AttachMiss`, and verify no stale name remains in the change or docs
- [x] 4.2 Add owner-lock and start-lock hold variants of the readiness test on the real clock through one shared stalled-owner fixture, keeping every assertion of the unheld test, and verify that swapping `elected` and `probed`, anchoring both at `started`, or anchoring only `elected` at `started` each fails at least one of them
- [x] 4.3 Open each starter child's stderr file append-only with a separate parent read handle, and verify the Unix and Windows starter tests use it
- [x] 4.4 Synchronise the stalled half of `starter_wait_surfaces_the_exited_child_stderr` on a first-write marker within its existing 2 s bound, adding no sleep, and verify it passes
- [x] 4.5 Rebase onto `origin/main` `a8ce468b`, and rerun the memory suite and static checks

## Audit: fixtures that launch a memory service or starter child

- `service.rs` `windows_starter_fixture` (held-client starter): stderr was the platform default null. Fixed in this change.
- `service.rs` `separate_cold_starters_share_one_owner_and_preserve_both_writes` client-fixture children: stderr is already piped, collected with `wait_with_output` and printed on failure. No change.
- `service.rs` `spawn_logged_owner_fixture`: stderr already goes to a caller-owned private file. No change.
- Production `spawn_service` discards the owner's stderr by design unless the test-support stage diagnostic is enabled. It is not a fixture and is unchanged.
- Out of the stated scope because they launch the Dolt supervisor or a parent fixture, not a memory service or starter: `server/windows_fixture.rs` (both launches; stderr null), `tests/server_lifecycle.rs:615` (`Stdio::null()`), `tests/windows_lifecycle.rs` parent fixture (stderr null). Listed for a later change.
