# Design

## Context and decision

Use `anyhow::Result<std::process::ExitCode>` for ordinary and Windows main, leaving the Windows async dispatch worker's Result<()> and checked join intact. `finish_dispatch` prints the same typed error then returns its checked u8 status; all existing typed statuses fit u8 (including130/141), while other errors remain Err for Rust's existing reporting. No custom Termination implementation, unsafe interop or profiling callback is needed.

## Operational surface

All authority cleanup already precedes finish_dispatch. Returning from main must retain bounded exit when a plain stdin/stdout thread remains blocked; existing real command interruption and backpressure tests verify that contract. On Unix the outer Tokio runtime now reaches its ordinary drop, so those tests are required alongside Windows native behavior. Existing native x64/ARM support, stdin/stdout behavior, connection limits and secret handling remain; no network, deployment or binary-version change.

## Integration contract

Primary source evidence: https://doc.rust-lang.org/stable/src/std/sys/exit.rs.html uses ExitProcess on Windows; LLVM22.1.8 `InstrProfilingFile.c` registers writeFileWithoutReturn through lprofAtExit, and `InstrProfilingUtil.c` implements that as atexit. https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/compiler-rt/lib/profile/InstrProfilingFile.c and https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/compiler-rt/lib/profile/InstrProfilingUtil.c. Rust recommends normal main termination: https://doc.rust-lang.org/std/process/fn.exit.html.

Existing Windows doctor tests preserve LLVM_PROFILE_FILE and all pass at76a076ea, but doctor.rs records only235/461 mapped group lines; error branches remain unhit. Re-run those same real commands under native instrumentation after correction and compare their branch coverage. This is the regression observation; increasing another module's tests is not evidence of fixing doctor profile collection.
