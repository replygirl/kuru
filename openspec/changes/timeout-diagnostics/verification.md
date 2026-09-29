# Verification

Every row is not yet run. Windows rows run natively in CI only.

## 1. Shared timeout arm names the blocked tree [critical]

- [ ] 1.1 @regression (agent) force the `command.rs` Unix timeout arm with a tiny deadline on a child that blocks and spawns a grandchild -> error text names the command, arguments and working directory and its snapshot lists the blocked child and the grandchild
- [ ] 1.2 @regression (agent) force the Windows arm the same way in native CI -> error text names the command, arguments and working directory and its Job list names the root and grandchild with image names, captured before termination
- [ ] 1.3 @unit (agent) make the snapshot fail -> the original timeout text is preserved and `snapshot unavailable: <reason>` is appended
- [ ] 1.4 @unit (agent) exercise the Windows accessor on a root plus grandchild -> both process IDs and image names are returned, and the accessor takes no action on them

## 2. Install family

- [ ] 2.1 @regression (agent) tiny-deadline blocking child through the Unix `execute` in `embedded_runtime.rs` -> text names command, arguments, working directory, lists the blocked child and grandchild from a snapshot taken before the kill, and reports survivors after the pipe grace
- [ ] 2.2 @regression (agent) tiny-deadline blocking child through the Windows `execute` and `windows_cli.rs` `success()` in native CI -> text names command, arguments, working directory and the Job list

## 3. Git child family

- [ ] 3.1 @regression (agent) tiny-deadline blocking child through each of the five git helpers and the delivery fixture -> the panic names the arguments, directory and elapsed time and includes the platform snapshot
- [ ] 3.2 @runtime (agent) Windows git helpers in native CI -> the Job list appears in the timeout text

## 4. Terminal family

- [ ] 4.1 @regression (agent) tiny-deadline `wait_exit` on a child that blocks -> text names program, arguments, pid, state and descendants
- [ ] 4.2 @unit (agent) the two negative tests -> they still assert `process exits: timed out`, `STALLED` and `child failed`

## 5. Family 4

- [ ] 5.1 @regression (agent) nested child that exits early with code 0 before writing its size report -> the error contains the complete nested PTY output, the exit status and what the nested fixture printed
- [ ] 5.2 @runtime (agent) `terminal_fixture_uses_its_requested_controlling_dimensions` on macos-latest in CI -> passes, and a future occurrence explains itself

## 6. No behaviour change

- [ ] 6.1 @equivalence (agent) diff review -> no deadline, retry, sleep, dependency or assertion change beyond added context
- [ ] 6.2 @manual (agent) `format:check`, lint with the Windows target, `typecheck`, `lint:tooling`, `docs:check`, `cospec validate --strict` -> all pass
