# Tasks

## 1. Design verification

- [x] 1.1 Obtain fable's verification of design.md (mechanism citations, lock scope within platform spawns, forced-race test shape) before implementing and verify the review is recorded in the roadmap note

## 2. Platform spawn path

- [x] 2.1 Add the private platform spawn lock (`try_lock` then `lock`, poison ignored) with thread-local `cfg(test)` `blocked` and `window` seams and verify the lock guards only pipe creation and `Command::spawn`
- [x] 2.2 Add `StdioPlan`/`StdioSlot` and platform pipe creation (`pipe_with(CLOEXEC)` where available, `pipe()` + FIOCLEX on Apple) and verify by a unit test that every created end carries close-on-exec
- [x] 2.3 Change `OwnedProcessGroup::spawn(command, stdio)` to create pipes, pass child ends as explicit descriptors, drop the `Command` before releasing the lock, and keep `take_stdin/stdout/stderr` unchanged; verify the existing platform unit and integration tests pass
- [x] 2.4 Take the same lock around the platform's own snapshot `ps` spawn (a platform spawn, not an `OwnedProcessGroup`; drop this task if the lead reads "owned" strictly) and verify the snapshot tests pass
- [x] 2.5 Enable rustix's `pipe` feature on the platform's existing pinned dependency and verify Cargo.lock has no version change

## 3. Callers

- [x] 3.1 Declare stdio for hooks (`hooks.rs:1018`), the built-in shell (`unix_shell.rs:846`), MCP stdio (`rpc.rs:405`) and the coverage group (`coverage.rs:1848`) and verify no caller still configures `Stdio` on an owned `Command`
- [x] 3.2 Run the hooks, unix_shell and rpc connector tests three times and verify they pass

## 4. Regression tests

- [x] 4.1 Add the forced-window unit test on every Unix (the EOF failure without the lock is Apple-only) (A paused in the window, B reports `Blocked` or `Spawned`, A's EOF required while B's stdin is held) and verify it FAILS with the lock disabled locally and PASSES with it
- [x] 4.2 Add the concurrent `cat` sentinel to `tests/unix_process_group.rs` and verify it passes ×20 with the fix
- [x] 4.3 Rerun the formerly hanging hook test under the measurement harness and record the hang count

## 5. Documentation and checks

- [x] 5.1 Document the guarantee and limit in the `unix.rs` module doc, AGENTS.md and docs/development.md, and verify `docs:check` passes
- [x] 5.2 Run format:check, lint, lint:windows, typecheck, the full platform tests (new test ×20) and `cospec validate --all --strict`, and verify all pass
