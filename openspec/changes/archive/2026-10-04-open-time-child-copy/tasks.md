# Tasks

## 1. Fixture isolation and verification

- [x] 1.1 Replace the test parent's executable copy in `packages/kuru-delivery/tests/open_time.rs` with a child copy awaited through the existing bounded command runner, and verify the parent never holds the destination writable descriptor.
- [x] 1.2 Run the affected test and full open-time integration binary through mise; record local results and Linux evidence separately, and run required format, lint, tooling, and cospec checks without weakening retirement assertions or widening waits.

## Evidence

- Main run `37190628278`, job `111401979622`: `a_process_still_running_from_the_root_is_a_bounded_failure` panicked at the launch unwrap with `Os { code: 26, kind: ExecutableFileBusy, message: "Text file busy" }`; the other two tests in its partition passed.
- Archived `unix-snapshot-stand-in-spawn` records Linux reproduction of the same mechanism: writer-in-parent creation failed 39 and 36 times in two 3000-iteration runs; child-only creation failed zero times in both runs. Its deterministic control demonstrates that a live child's inherited write descriptor blocks exec.
- Observed 2026-10-04 in an offline local Linux `rust:1.98.1` container: a deterministic live child holding a writable executable descriptor causes `Text file busy (os error 26)`. A scratch Rust harness copying `/bin/true` and immediately executing it under three concurrent fork loops observed 137 `ExecutableFileBusy` failures in 3000 parent copies and zero in 3000 awaited `/bin/cp` copies. After each copy the parent's `/proc/self/fd` contained no destination descriptor in either shape, demonstrating why closing the parent's descriptor alone does not exclude the sibling's transient inherited copy. This is mechanism and stress evidence, not a hosted CI or full Linux integration pass.
- Local macOS: `mise exec -- cargo test -p kuru-delivery --all-features --locked --test open_time a_process_still_running_from_the_root_is_a_bounded_failure -- --exact --nocapture` passed 1/1; the full `--test open_time -- --nocapture` invocation passed 9/9. macOS does not enforce Linux's `ETXTBSY` condition.
- `mise run format:code`, `mise run //packages/kuru-delivery:lint`, `mise run lint:tooling`, and strict cospec validation passed. Product performance gates and retirement wait values were not changed. Hosted native CI remains to run on the PR head.
