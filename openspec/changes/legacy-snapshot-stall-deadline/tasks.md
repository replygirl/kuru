# Tasks

## 1. Progress-restarted snapshot bound

- [x] 1.1 Factor the backup loop into `run_backup(step, now)` with the named `SNAPSHOT_STALL_BOUND`, restart the deadline on every `More` and format the stall message from the constant, and verify `prepare` passes `|| backup.step(256)` and `Instant::now`

## 2. Regression tests

- [x] 2.1 Add mock-clock tests: a progressing copy past 30 s in total completes, and a busy source past the bound fails with the new message; verify both fail against the old fixed-deadline loop and pass with the fix

## 3. Checks

- [x] 3.1 Run the memory package's migration and legacy import tests, `format:check`, `lint`, root `lint:windows`, `typecheck`, `docs:check` and `cospec -- validate --all --strict`, and verify each exits 0
