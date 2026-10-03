# Tasks

Work in `tmp/worktrees/fix-trust-approval-read-errors` (branch
`fix/trust-approval-read-errors`, from `origin/main` fb284ef7). Never commit
`mise.lock`; never bypass hk hooks. Tasks are ticked as evidence lands (see
`verification.md`).

## 1. Change artifacts

- [x] 1.1 Author proposal, blocking-changes, design, tasks and verification and run `mise run cospec -- validate trust-approval-read-errors --strict`, and verify it reports no error. Observed 2026-10-03: `0 errors, 0 warnings — validation passed` after adding the interactive surface's `## Operational surface` design section and an `@e2e` row the first strict run demanded. The regression tests and fix were drafted before these artifacts; the gate was then run and cleared before any commit.
- [x] 1.2 Run `mise run cospec -- apply trust-approval-read-errors --json` and verify the gate exit code is 0, or that every soft blocker is resolved or acknowledged. Observed 2026-10-03: first run exit 3 (two surface soft blockers, resolved in the artifacts, not acknowledged), then gate state `clear`, exit 0.

## 2. Regression tests (red before the fix)

- [x] 2.1 Add `trust::tests::an_unreadable_record_names_its_os_error_and_never_matches` (Unix, mode 000 record, non-root guard) and verify it fails on the unfixed tree because `inspect` returns `Invalid`. Observed 2026-10-03 (macOS arm64, unfixed tree with a compiling `assert_ne!(state, Invalid)` form): FAILED `assertion left != right failed: a read error is not structural; left: Invalid`.
- [x] 2.2 Add `instruction_gate::tests::an_unreadable_record_at_publication_reports_its_read_error` and verify it fails on the unfixed tree with "workspace approval changed during review". Observed 2026-10-03 (macOS arm64, unfixed tree): FAILED, panicked with `workspace approval changed during review; review current authority again`.
- [x] 2.3 Add the CLI test `an_unreadable_approval_record_reports_its_read_error_and_remedy` and verify it fails on the unfixed tree with "approval state is invalid or unsafe". Observed 2026-10-03 (macOS arm64, unfixed tree): FAILED, status printed `Status: approval state is invalid or unsafe`.
- [x] 2.4 Add `trust::tests::a_directory_in_the_records_place_never_matches` and verify which fixture reaches which state. Observed 2026-10-03 (macOS arm64): passed before and after the fix. A directory at the record path opens under `O_RDONLY|O_NOFOLLOW` and the checked filesystem rejects it with a synthesized "expected a regular disk file" error (no OS code), so it stays `Invalid`; mode 000 fails at `openat` with raw `EACCES`, the only fixture that reaches `Unreadable`.

## 3. Fix

- [x] 3.1 Add `ApprovalState::Unreadable(ReadFailure)` and classify the `Err` arms of `inspect` and `inspect_nested`; keep structural failures `Invalid`; verify the 2.1 test and the existing invalid-record tests pass. Observed 2026-10-03 (macOS arm64): `//apps/kuru-tui:test -- -- unreadable a_directory_in_the_records_place trust:: instruction_gate::` lib 20 passed, 0 failed, including the unchanged symlink, permissive-mode, hard-link, malformed and oversized `Invalid` tests.
- [x] 3.2 Propagate the read error from the instruction gate's `publish()` with context, keeping the changed-during-review check for a read that succeeds; verify 2.2 passes. Observed 2026-10-03: same run, `an_unreadable_record_at_publication_reports_its_read_error` ok.
- [x] 3.3 Add the CLI status row naming the I/O error and a permissions remedy; verify 2.3 passes. Observed 2026-10-03: same run, `tests/cli.rs` 1 passed.
- [x] 3.4 Document the behaviour in `docs/configuration.md#workspace-trust` and verify `mise run docs:check` passes. Observed 2026-10-03: `docs:check` exit 0 (run with `NODE_OPTIONS` unset; the shell preload breaks the docs toolchain).

## 4. Checks and archive

- [x] 4.1 Run `mise run //apps/kuru-tui:test`, `format:check`, `lint`, `lint:windows`, `typecheck`, `docs:check` and `cospec -- validate --all --strict`, and verify each exits 0. Observed 2026-10-03 (macOS arm64): full `//apps/kuru-tui:test` exit 0 in 405.6 s (lib 137, cli 44, terminal 43 with 1 ignored, trust 22, embedded_runtime 7, visual 12 with 1 ignored, unix_shell_turn 5, lease 4, openai_auth 4, preferences 3, update 2 with 1 ignored, directories 1, server 1, ui_runtime 1; 0 failed); `format:check`, `lint`, root `lint:windows` (includes `-p kuru` for x86_64-pc-windows-msvc), `typecheck` and `docs:check` exit 0 with `NODE_OPTIONS` unset; `cospec -- validate --all --strict` exit 0.
- [x] 4.2 Run `mise run cospec -- archive trust-approval-read-errors` and verify the archive directory exists and no active record remains. Observed 2026-10-03: archived by the archive commit on this branch; the directory is checked after the command.
