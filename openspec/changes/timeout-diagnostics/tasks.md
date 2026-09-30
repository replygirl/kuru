# Tasks

## 1. Platform accessor and shared timeout arm

- [ ] 1.1 Add a read-only `NativeChild` accessor in `packages/kuru-platform/src/windows/process.rs` that lists the Job's process IDs with image names, using `QueryInformationJobObject(JobObjectBasicProcessIdList)`, `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and `QueryFullProcessImageNameW`, and verify with a Windows-native test that a spawned root and grandchild both appear and that nothing acts on a PID.
- [ ] 1.2 Extend the Windows timeout arm in `packages/kuru-delivery/src/command.rs` to add root state, Job state and the process list to its error text before it terminates the tree, and verify with a tiny-deadline test on a blocking child that the text names the command, arguments, working directory and the blocked child and grandchild.
- [x] 1.3 Add the bounded, drained Unix `ps` descendant snapshot before the kill in the `bounded_unix` timeout arm, with `snapshot unavailable: <reason>` on failure, and verify with a tiny-deadline Unix test that the text lists the blocked child and grandchild.
- [x] 1.4 Regression test: a snapshot failure (for example an unusable `ps` path in a test seam) appends `snapshot unavailable: <reason>` and keeps the original timeout error text.

## 2. Install family helpers

- [x] 2.1 In `embedded_runtime.rs` `execute` (Unix), take the descendant snapshot before `child.kill()`, re-check the recorded processes after the pipe grace, and add the command, arguments and working directory; verify with a tiny-deadline negative test.
- [ ] 2.2 Add step and argument context to the nine bare `execute` sites and the Windows `execute`, and fix the misleading "installed Kuru" text at the pre-install selected-binary site; verify no deadline or assertion changed.
- [ ] 2.3 Add context to `windows_cli.rs` `success()` and the six direct `.output()` sites; verify by a Windows-native tiny-deadline test in CI.

## 3. Git child family helpers

- [x] 3.1 Replace the `unwrap()` in `src/advisory.rs` `tests::git` and `git_with_environment` with a panic carrying arguments, directory and elapsed time; verify with a tiny-deadline negative test.
- [x] 3.2 Do the same in `tests/advisory.rs` `git` and `git_with_environment`, the `src/coverage.rs` test `git`, and the `tests/fixtures/delivery.rs` fixture binary; verify each helper's message.

## 4. Terminal family helper

- [x] 4.1 Record program, arguments and `LLVM_PROFILE_FILE` presence in `Terminal::spawn` in `apps/kuru-tui/tests/support/terminal.rs`, and add a bounded `ps` snapshot of the child and its descendants to the `wait_exit` timeout arm (and `wait`); verify the two negative tests still assert `process exits: timed out` plus `STALLED` and `child failed`.

## 5. Family 4: nested terminal fixture

- [x] 5.1 In `terminal_fixture_uses_its_requested_controlling_dimensions`, capture the nested child's complete PTY output and exit status in the error and make the nested fixture record what it printed; verify by a negative test in which the nested child exits early that the error contains the output and status.

## 6. Verification

- [ ] 6.1 Run `//packages/kuru-platform`, `//packages/kuru-delivery` and `//apps/kuru-tui` tests, `format:check`, lint (including the Windows-target lint), `typecheck`, `lint:tooling`, `docs:check` and `cospec validate --strict`, and record observed results and any unrun checks with reasons.
- [ ] 6.2 Confirm in CI that the Windows tests ran natively on windows-latest and windows-11-arm, and record the run ids.

## Audit

- Windows sites: the install family has 13 `execute` sites, 3 direct `command::output` calls and 8 `BlockingCommand::output` sites behind 3 helpers; the git family has 31 sites behind 6 helpers; the terminal family has 45 sites behind 1 helper.
- Excluded on purpose: `mise_acceptance.rs`, `unix_shell_turn.rs` and `repository_environment.rs`, which have no occurrences in these families.

## Observed evidence

2026-09-29, work package P (macOS arm64 host, commits `875d7a4d`, `7b91d257` and the follow-up test commit):

- 1.1 and 1.2 are implemented (`NativeChild::diagnostic_snapshot`, `TreeSnapshot`;
  the Windows arm appends `command=… arguments=… directory=…; tree before cleanup: …`
  before `terminate`). Their tests (`diagnostic_snapshot_lists_job_members_without_changing_the_tree`,
  `quiescence_failure_lists_the_live_descendant_before_owned_cleanup`,
  `timeout_lists_the_running_root_before_owned_cleanup`) compile under the Windows
  target lint but have not run: Windows behaviour is unverified until native CI.
  These boxes stay open until CI records them.
- 1.3: `bounded_output_timeout_names_the_command_and_its_blocked_root` and
  `bounded_output_failure_snapshot_lists_the_live_grandchild_before_cleanup` passed
  in `mise run //packages/kuru-delivery:test` (348 passed, 0 failed, 1 ignored).
  The snapshot is Unix-wide stock `ps`; Linux procps output was only format-checked
  in an `ubuntu:24.04` container, not run through Rust.
- 1.4: `snapshot_failure_is_reported_as_text` (missing `ps` path and a failing `ps`
  stand-in) and the snapshot unit tests passed in `mise run //packages/kuru-platform:test`
  (52 passed, 0 failed, including `snapshot_helper_is_bounded_when_ps_does_not_finish`). The delivery arm appends the snapshot after the original
  error and cleanup text by construction.
- Sampling inside short deadlines: `SAMPLE_INTERVAL` and the skipped first tick are
  unchanged; the snapshot adds one final root sample and one sample per Job member
  at failure time instead.

2026-09-29, work package H (macOS arm64 host):

- 2.1 and 2.2: `embedded_runtime.rs` `execute` is now `execute_within` on both platforms
  (`execute` passes the unchanged `COMMAND_TIMEOUT`). Unix names the command, arguments
  and directory, records the process tree before `child.kill()`, and after the pipe
  grace lists which recorded processes are still listed. The nine bare sites carry a
  `step:` context; the misleading "installed Kuru" text is gone from the shared helper.
  `execute_failure_tests` passed on macOS. 2.2 stays open: the Windows `execute` context
  and its test compile under `lint:windows` but are unrun.
- 2.3: `windows_cli.rs` gained `launch()`; `success()` and four direct `.output()` sites
  use it. The other two direct sites already panic with their stages. Windows-only,
  unrun until native CI.
- 3.1 and 3.2: five helpers and the fixture binary panic with arguments, directory and
  elapsed time; each has a launch-failure test that passed locally.
- 4.1: `Terminal` records its launch; timeout, exit and failure arms append `report()`.
  `terminal_timeouts_report_the_launch_and_a_process_tree_snapshot` passed.
- 5.1: `terminal_wait_reports_output_still_queued_when_the_exit_is_seen` and
  `terminal_fixture_nested_early_exit_reports_the_inner_output_and_status` passed.
