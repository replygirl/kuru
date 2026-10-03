# Verification

## 1. A wait that observes the exit includes the child's final output [critical]

- [x] 1.1 @regression (agent) Unix `terminal_wait_drains_the_line_written_just_before_exit` (late-output child: EARLY, then LATE written after the parent stopped reading, then exit; never-true wait) -> observed 2026-10-03 macOS arm64: with the two lines that move drained bytes into `output` and the parser removed, FAILED `the line written just before exit was dropped: late output becomes visible: process exited ExitStatus { code: 0, signal: None }` (output ended at `EARLY`); with the drain, passed (1 passed)
- [~] 1.2 @regression (agent) Windows `native_conpty_wait_drains_the_line_written_just_before_exit` (`cmd.exe /d /s /c "echo DRAIN_ON_EXIT_MARKER"`, never-true wait) -> defer: needs native ConPTY, which runs in PR CI only; compiled and clippy-clean for x86_64-pc-windows-msvc through `mise run lint:windows` (exit 0). Red by reasoning: without the close, the ConPTY pipe stays open after the exit and the wait returned before conhost delivered the final frame

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //apps/kuru-tui:test -- --test terminal --test trust --test cli` (the task's `--all-targets` keeps every kuru test binary in the run) -> observed: exit 0; terminal 43 passed, 1 ignored; trust 22 passed; cli 43 passed, 1 ignored; 282 passed, 0 failed across all binaries

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run cospec -- validate --all --strict` -> observed: each exit 0 (format:check and lint run with the session's cmux `NODE_OPTIONS` preload unset, which otherwise fails the docs toolchain step before any check); validate 0 errors, 0 warnings

## 4. PR CI correction

- [x] 4.1 @regression (agent) Windows `native_conpty_wait_drains_the_line_written_just_before_exit` -> observed in PR CI run 37112735536 (windows-latest and windows-11-arm, partition 6): FAILED `the line written just before exit was dropped` with drained output `'"echo"' is not recognized as an internal or external command`, `output EOF reached=true`. Cause: the platform quotes every argument, so `cmd.exe /d /c echo MARKER` reached cmd as `"echo" "MARKER"`, and `echo` never ran; the drain itself worked (EOF reached, last line captured). Fixed by passing the command as one argument under `/s` (`cmd.exe /d /s /c "echo DRAIN_ON_EXIT_MARKER"`). The fixed test is unrun locally (needs native ConPTY); compiled and clippy-clean through `mise run lint:windows`. The `actual console modes were not restored` line in that output is not from the drain (the sibling fixture checks modes and publishes its report before exiting, and the drain starts only after that exit); inferred to come from the stock `cmd.exe` child changing the console modes this fixture compares, not measured, and the test's assertions do not depend on the fixture's exit code
