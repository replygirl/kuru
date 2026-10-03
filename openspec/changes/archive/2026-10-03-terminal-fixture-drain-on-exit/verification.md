# Verification

## 1. A wait that observes the exit includes the child's final output [critical]

- [x] 1.1 @regression (agent) Unix `terminal_wait_drains_the_line_written_just_before_exit` (late-output child: EARLY, then LATE written after the parent stopped reading, then exit; never-true wait) -> observed 2026-10-03 macOS arm64: with the two lines that move drained bytes into `output` and the parser removed, FAILED `the line written just before exit was dropped: late output becomes visible: process exited ExitStatus { code: 0, signal: None }` (output ended at `EARLY`); with the drain, passed (1 passed)
- [~] 1.2 @regression (agent) Windows `native_conpty_wait_drains_the_line_written_just_before_exit` (`cmd.exe /d /c echo DRAIN_ON_EXIT_MARKER`, never-true wait) -> defer: needs native ConPTY, which runs in PR CI only; compiled and clippy-clean for x86_64-pc-windows-msvc through `mise run lint:windows` (exit 0). Red by reasoning: without the close, the ConPTY pipe stays open after the exit and the wait returned before conhost delivered the final frame

## 2. Designed behaviour unchanged

- [x] 2.1 @integration (agent) `mise run //apps/kuru-tui:test -- --test terminal --test trust --test cli` (the task's `--all-targets` keeps every kuru test binary in the run) -> observed: exit 0; terminal 43 passed, 1 ignored; trust 22 passed; cli 43 passed, 1 ignored; 282 passed, 0 failed across all binaries

## 3. Static checks

- [x] 3.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run cospec -- validate --all --strict` -> observed: each exit 0 (format:check and lint run with the session's cmux `NODE_OPTIONS` preload unset, which otherwise fails the docs toolchain step before any check); validate 0 errors, 0 warnings
