# Proposal

## Why

The terminal test fixtures return their "child exited" error from `wait` as soon
as `try_wait` reports the exit, without first reading what the child wrote just
before exiting. On PR #188 run 37109318921 (windows-latest coverage partition 1,
job 111164109682) `native_conpty_trust_refusal_and_persistent_choice_precede_the_alternate_screen`
then failed its later `terminal.output` assertion with "approved launch did not
reach its missing-key refusal" and a screen ending at the trust prompt, although
the child had printed that refusal; the same assertion failed on 2026-09-29 run
36601116359 job 109518773743. The Unix fixture has the same shape: its exit arm
collects output still queued at the exit into a report-only `pending` queue and
leaves `output` and the parser unchanged, so `trust.rs` assertions on
`terminal.output` after "process exited" race the reader the same way.

On ConPTY, the output pipe does not reach EOF when the child exits: portable-pty
0.9.0 shares the pseudoconsole between master and slave, and `ClosePseudoConsole`
runs only when the fixture drops its master. The fixture's existing `finish`
already closes the console before its final read for this reason.

## What Changes

- Windows `Terminal::wait` (test support): when the child has exited, close the
  console (`ClosePseudoConsole` on the existing worker thread, keeping the output
  pump alive) bounded by `EXIT`, then drain every queued message into `output`
  and the parser, re-check the predicate once against the completed output, and
  only then fail with "child exited", the exit status and whether EOF was reached.
  `EXIT` (30 s) moves from the `windows_terminal` test binary into the support
  module unchanged; no new budget is introduced.
- Windows `Terminal::wait_exited` (test support): observes the exit without
  reading or closing, for the one test that wraps the live master after the exit.
  `native_conpty_error_drop_returns_while_console_close_is_delayed` observes its
  exit through it; its assertions are unchanged.
- Unix `Terminal::wait` (test support): the exit arm records the diagnostics seen
  at the exit, then drains queued output into `output` and the parser until the
  reader closes (EIO/EOF) within the existing `LATE_OUTPUT_WINDOW`, re-checks the
  predicate once, and only then fails with "process exited" and whether EOF was
  reached. The report-only `pending` queue is removed.
- `read_for` needs no change on either platform: the reader channel is ordered, so
  every byte written before the terminal read error has already been processed
  when that error ends the read.
- Regression tests: Unix `terminal_wait_drains_the_line_written_just_before_exit`
  (replacing the old late-output test, whose final assertion encoded the gap) and
  Windows `native_conpty_wait_drains_the_line_written_just_before_exit`.
- No product code changes, and no change to what the ConPTY trust test asserts.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `apps/kuru-tui/tests/support/windows_terminal.rs`, `apps/kuru-tui/tests/support/terminal.rs`
- `apps/kuru-tui/tests/windows_terminal.rs`, `apps/kuru-tui/tests/terminal.rs`
- Test support only; no runtime, documentation or dependency change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
