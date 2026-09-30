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
- **Family 4.** When `Terminal::wait` observes an exit before its condition, it
  reports the exit status, then collects output still queued for up to 500 ms
  (`LATE_OUTPUT_WINDOW`, stopping early when the reader closes) and appends
  `Terminal::report()`: launch, child state, the complete escaped PTY output and
  a process-tree snapshot. The nested fixture wraps the inner failure with that
  report, so the outer `child failed` text carries the inner status and output.
  The nested fixture prints no progress marker: its only marker is
  `INNER_SIZE_OK` on success, and the inner output itself shows what it reached.
  The window holds what it collects in a separate pending queue for the report
  only: `Terminal::output` and the screen parser are unchanged when `wait`
  returns, and a later read consumes the queued messages first, in order. Callers
  that use this error as their exit wait and then assert on `output` (for
  example `wait_for_refusal` in `apps/kuru-tui/tests/trust.rs`) observe exactly
  what they did before. Inference, not measured: the CI failure may be a
  drain race in which the inner child wrote `SIZE:` but its exit was seen before
  the reader delivered it; the queued-output section distinguishes that from a
  child that never wrote a size. A reader that stays open past the window is
  named as such rather than read to EOF.
- **Terminal helper** records program, args and whether `LLVM_PROFILE_FILE`
  reached the child at `Terminal::spawn`; the `wait_exit` bail keeps the
  substrings `process exits: timed out` and `child failed` that the negative
  tests assert.

## Risks / Trade-offs

- Shipped Windows source builds print a longer timeout message. Mitigation: text
  only, failure path only, bounded.
- The `ps` snapshot adds latency to the 100 ms negative terminal test.
  Mitigation: bounded wait; the test expects failure and asserts only substrings.
- The shipped Unix `bounded_output` arm runs the `ps` snapshot before
  `stop_and_reap` on every capture failure (timeout, overflow or read error), so
  cleanup of a failed tool can start up to `SNAPSHOT_TIMEOUT` (2 s) later.
  Mitigation: failure path only, bounded, the process group stays owned and
  unreaped throughout, and no deadline or outcome changes.
- Unix callers of `command::output` (the Tokio timeout with `tool timed out`)
  get no snapshot or command context; only `bounded_output` and the test helpers
  built on it do. This gap is accepted for now; the recorded families use
  `bounded_output`, the embedded-runtime `execute` helper or `Terminal`.
- Job PID lists race with process exit. Mitigation: printed as a point-in-time
  observation, never acted on.
- Windows evidence exists only from native CI runs; local macOS cannot exercise
  it.
