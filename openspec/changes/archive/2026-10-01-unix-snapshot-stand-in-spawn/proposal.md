# Proposal

## Why

`snapshot_helper_is_bounded_when_ps_does_not_finish` built its stalled `ps`
stand-in with `std::fs::write` and had `snapshot::describe_with` execute it. On
PR #166 (run 36940362105, ubuntu-latest coverage partition 6) it failed with
`spawn "<tmp>/stalled-ps": Text file busy (os error 26)`.

The cause is in the test. Sibling tests in the binary fork children from other
threads; `fork` copies the descriptor table and `FD_CLOEXEC` closes the copy
only at the child's `exec`. A child forked while this thread's write descriptor
was open keeps the file open for writing, and Linux refuses to execute such a
file (rust-lang/rust#114554). A retry or wider bound would only hide it.

## What Changes

- The stand-in is created entirely by a short-lived `/bin/sh` child that writes
  the script through a shell redirection and sets its mode, and the test waits
  for that child to exit before using the file. The test process never opens a
  write descriptor to the stand-in, so no concurrently forked sibling can
  inherit one.
- On Linux only, the test then scans the process's own descriptor table
  (`/proc/self/fd`) and asserts that none refers to the stand-in's device and
  inode, which pins the invariant the fix relies on. The scan is compiled out
  elsewhere: macOS does not enforce ETXTBSY, and its `/dev/fd` entries report a
  different device than the file. The comment records why creation must not go
  through a descriptor held here.
- A control test inherits a write descriptor into a live child with safe
  `Stdio` plumbing and shows that exec of the file is refused while the child
  holds it, so the mechanism is demonstrated rather than only described. It is
  Linux-only, where the kernel enforces the condition.
- The original bounded-helper assertions (`did not finish within` text, the
  elapsed lower bound `SNAPSHOT_TIMEOUT`, the upper bound of three seconds
  beyond it) are unchanged. No retry on ETXTBSY, no sleep, no widened bound,
  `SNAPSHOT_TIMEOUT` and the product helper are untouched.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-platform/tests/unix_snapshot.rs` only. No product code,
dependency, timeout, workflow or documentation change. No durable spec
describes how this test builds its stand-in, so no capability changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
