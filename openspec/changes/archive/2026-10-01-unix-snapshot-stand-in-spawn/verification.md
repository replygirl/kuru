# Verification

## 1. The stand-in never exists through a descriptor held here [critical]

- [x] 1.1 @regression (agent) in a Linux container (`rust:1.98.1`, aarch64 under OrbStack), run a scratch harness (not committed; 3 threads fork `/bin/true` in a loop while the main thread creates and executes 3000 fresh stand-ins per shape) comparing the old creation shape (`std::fs::write` + `set_permissions`, then `Command::spawn`) with the new shape (waited `/bin/sh` redirection + `chmod`) -> observed 2026-10-01, two runs: old shape ETXTBSY 39 and 36 of 3000 (`ErrorKind::ExecutableFileBusy`, "Text file busy"); new shape ETXTBSY 0 and 0 of 3000. The spawn is the same `Command::spawn` that `describe_with` performs; the loop uses a script that exits at once, not the 30 s stand-in, to keep 3000 iterations short. Reproduces the CI error text and shows the fix removes it under forced concurrency; the harness is a stress measurement, not a deterministic ordering
- [x] 1.2 @unit (agent) run `snapshot_helper_is_bounded_when_ps_does_not_finish` with its Linux descriptor-table assertion, the scan self-check `descriptor_scan_sees_exactly_the_descriptors_held_here` and the control `a_write_descriptor_inherited_by_a_live_child_blocks_exec_of_the_file` -> observed in the Linux container: all pass (6 passed, 0 failed in the binary); on the macOS host the Linux-only tests are compiled out and the other 4 pass. The control shows exec of a file open for writing by a live child fails with `ExecutableFileBusy`, the kernel condition behind the CI failure
- [x] 1.3 @unit (agent) check that the macOS host cannot exercise the defect -> observed 2026-10-01: a script executed while a write descriptor to it was open ran normally (exit 0), so macOS does not enforce ETXTBSY and the host loop below is not evidence for the Linux flake; `/dev/fd/N` there reports a different device than the file, which is why the descriptor scan is Linux-only

## 2. Repeated runs

- [x] 2.1 @integration (agent) run `cargo test -p kuru-platform --test unix_snapshot` 30 times on the macOS host -> observed 30/30 passed (4 tests each, the Linux-only ones compiled out). Cannot show the Linux race: macOS does not enforce ETXTBSY
- [x] 2.2 @integration (agent) run the compiled unix_snapshot binary 30 times in the Linux container -> observed 30/30 passed (6 tests each). The unfixed race fired once in CI and in about 1.2 percent of the harness's old-shape iterations, so 30 clean runs are consistent with the fix but are not by themselves proof

## 3. Repository gates

- [x] 3.1 @integration (agent) run the kuru-platform tests, format check, lint, typecheck and Linux clippy/fmt -> observed on the macOS host 2026-10-01: `mise run //packages/kuru-platform:test` exit 0; `mise run format:check` exit 0; `mise run lint` exit 0; `mise run typecheck` exit 0. In the Linux container (the host lint compiles out the Linux-only code): `cargo clippy -p kuru-platform --all-targets --all-features --locked -- -D warnings` exit 0 and `cargo fmt --package kuru-platform -- --check` exit 0. Windows-target lint and native Windows and macOS CI legs not run: the change is Unix test code
- [~] 3.2 @e2e (agent) observe the ubuntu-latest coverage partition that runs `unix_snapshot` over many CI runs after merge -> defer: requires repeated native CI runs; a single green run does not prove an intermittent race fixed
