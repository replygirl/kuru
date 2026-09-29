# Design

## Context

Timeout failures in four families reach a human as a bare pid or a bare
"tool timed out". On Windows the shared arm in `command.rs` terminates the Job
before returning, and `NativeChild` exposes only root state, an active-process
count and root CPU. The advisory `samples=[]` is structural: the first sample
tick is skipped and the interval is 10 s, so no sample lands inside a deadline
of 10 s or less. On Unix the install helper sets no process group and kills only
the root, so a snapshot taken after the kill finds orphans reparented to init.

## Goals / Non-Goals

**Goals:**
- Make each recorded timeout explain itself: command, arguments, working
  directory, elapsed time, root state, Job state and the live processes.
- Keep one edit per helper, and one shared edit for the Windows arm.

**Non-Goals:**
- Any deadline change, retry, sleep, new dependency or changed assertion.
- Acting on a numeric PID for any purpose other than printing it.
- Fixing the underlying hangs or the family 4 early exit.

## Decisions

- **Edit the shared `command.rs` arm (Windows option a).** One edit covers every
  Windows site in the install and git families. Rejected: per-helper timed
  spawns (about 60 duplicated lines each, still needing the accessor), and
  accepting context without a process list (leaves the recorded failures
  unexplained).
- **Read-only `NativeChild` accessor** returning process IDs and image names,
  built on the already-enabled windows-sys features. It is diagnostic data only
  and exposes no handle or signal authority, consistent with AGENTS.md.
  Rejected: ToolHelp (feature not enabled) and PowerShell `Get-CimInstance`
  (cold start is slow under the load being diagnosed).
- **Unix snapshot uses stock `ps`** with a bounded wait and drained output,
  filtered to the descendant closure in Rust (`ps -g` is legacy on macOS). `/proc`
  is not used, so macOS and Linux share one path. The snapshot is taken before
  the kill, and survivors of the recorded set are re-checked after the pipe grace.
- **Failure isolation.** A snapshot failure appends `snapshot unavailable:
  <reason>` to the original message and never replaces it.
- **Family 4.** The fixture test reads the nested child's PTY output to EOF and
  its exit status when the size report is missing, and the nested fixture prints
  a marker naming what it reached, so an early clean exit is explained.
- **Terminal helper** records program, args and whether `LLVM_PROFILE_FILE`
  reached the child at `Terminal::spawn`; the `wait_exit` bail keeps the
  substrings `process exits: timed out` and `child failed` that the negative
  tests assert.

## Risks / Trade-offs

- Shipped Windows source builds print a longer timeout message. Mitigation: text
  only, failure path only, bounded.
- The `ps` snapshot adds latency to the 100 ms negative terminal test.
  Mitigation: bounded wait; the test expects failure and asserts only substrings.
- Job PID lists race with process exit. Mitigation: printed as a point-in-time
  observation, never acted on.
- Windows evidence exists only from native CI runs; local macOS cannot exercise
  it.
