# Tasks

## 1. Drain output at an observed exit

- [x] 1.1 Windows `wait`: on an observed exit, close the console bounded by `EXIT`, drain the channel into `output` and the parser, re-check the predicate, then fail with "child exited", the status and whether EOF was reached; verify `mise run lint:windows` passes
- [x] 1.2 Add Windows `wait_exited` and move the drop test's exit observation onto it, and verify its assertions are unchanged
- [x] 1.3 Unix `wait`: on an observed exit, record diagnostics, drain into `output` and the parser until reader close within `LATE_OUTPUT_WINDOW`, re-check the predicate, then fail with "process exited" and whether EOF was reached; remove `pending`; verify the terminal binary passes

## 2. Regression tests

- [x] 2.1 Unix: a child that prints one line after the parent stops reading and exits; a never-true wait must report the exit and `output` must contain the line; verify it fails with the drain reverted and passes with it
- [x] 2.2 Windows: `cmd.exe` prints a marker and exits; a never-true wait must report "child exited" and `output` must contain the marker; verify it compiles through `lint:windows` (it runs natively in CI only)

## 3. Checks

- [x] 3.1 Run the checks in verification.md and record the evidence
