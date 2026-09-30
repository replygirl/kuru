# Verification

Ticked rows ran locally on a macOS arm64 host on 2026-09-29, with the evidence
noted on each row. Unticked rows have not run. Windows rows run natively in CI
only.

## 1. Shared timeout arm names the blocked tree [critical]

- [x] 1.1 @regression (agent) force the `command.rs` Unix timeout arm with a tiny deadline on a child that blocks and spawns a grandchild -> error text names the command, arguments and working directory and its snapshot lists the blocked child and the grandchild. Evidence: `bounded_output_timeout_names_the_command_and_its_blocked_root` and `bounded_output_failure_snapshot_lists_the_live_grandchild_before_cleanup` passed in `mise run //packages/kuru-delivery:test`.
- [ ] 1.2 @regression (agent) force the Windows arm the same way in native CI -> error text names the command, arguments and working directory and its Job list names the root and grandchild with image names, captured before termination
- [x] 1.3 @unit (agent) make the snapshot fail -> the original timeout text is preserved and `snapshot unavailable: <reason>` is appended. Evidence: `snapshot_failure_is_reported_as_text` (missing and failing `ps`) passed in `mise run //packages/kuru-platform:test`; the delivery arm appends the snapshot after the original error by construction.
- [ ] 1.4 @unit (agent) exercise the Windows accessor on a root plus grandchild -> both process IDs and image names are returned, and the accessor takes no action on them

## 2. Install family

- [x] 2.1 @regression (agent) tiny-deadline blocking child through the Unix `execute` in `embedded_runtime.rs` -> text names command, arguments, working directory, lists the blocked child and grandchild from a snapshot taken before the kill, and reports survivors after the pipe grace. Evidence: `execute_failure_tests` passed in `mise run //apps/kuru-tui:test`.
- [ ] 2.2 @regression (agent) tiny-deadline blocking child through the Windows `execute` and `windows_cli.rs` `success()` in native CI -> text names command, arguments, working directory and the Job list

## 3. Git child family

- [x] 3.1 @regression (agent) launch failure (missing working directory) through each of the five git helpers and the delivery fixture -> the panic names the arguments, directory and elapsed time and embeds the `bounded_output` error text; the timeout snapshot reaches it by composition, since that arm is covered by row 1.1. No tiny-deadline test runs through a helper. Evidence: the `git_helper(s)_name_arguments_directory_and_elapsed_time_when_the_launch_fails` tests passed in `mise run //packages/kuru-delivery:test`.
- [ ] 3.2 @runtime (agent) Windows git helpers in native CI -> the Job list appears in the timeout text

## 4. Terminal family

- [x] 4.1 @regression (agent) tiny-deadline `wait_exit` on a child that blocks -> text names program, arguments, pid, state and descendants. Evidence: `terminal_timeouts_report_the_launch_and_a_process_tree_snapshot` passed in `mise run //apps/kuru-tui:test`.
- [x] 4.2 @unit (agent) the two negative tests -> they still assert `process exits: timed out`, `STALLED` and `child failed`. Evidence: both passed in `mise run //apps/kuru-tui:test` with those assertions unchanged.

## 5. Family 4

- [x] 5.1 @regression (agent) nested child that exits early with code 0 before writing its size report -> the error contains the complete nested PTY output, the exit status and what the nested fixture printed. Evidence: `terminal_fixture_nested_early_exit_reports_the_inner_output_and_status` and the synchronized `terminal_wait_reports_output_still_queued_when_the_exit_is_seen` passed in `mise run //apps/kuru-tui:test`; the latter also passed 30 of 30 direct repeats and 20 of 20 under 28 CPU hogs. After late output moved to a separate pending queue, the late-output test also asserts that `Terminal::output` lacks the queued bytes when `wait` returns and gains them on the next read; it passed in `mise run //apps/kuru-tui:test` (2026-09-29), not re-run under repeats or CPU load.
- [ ] 5.2 @runtime (agent) `terminal_fixture_uses_its_requested_controlling_dimensions` on macos-latest in CI -> passes, and a future occurrence explains itself

## 6. No behaviour change

- [ ] 6.1 @equivalence (agent) diff review -> no deadline, retry, sleep, dependency or assertion change beyond added context
- [ ] 6.2 @manual (agent) `format:check`, lint with the Windows target, `typecheck`, `lint:tooling`, `docs:check`, `cospec validate --strict` -> all pass
