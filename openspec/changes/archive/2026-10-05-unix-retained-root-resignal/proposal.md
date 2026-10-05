# Proposal

## Why

A Unix process-group SIGKILL sweep can miss a descendant forked during the
kernel's group walk. Cleanup callers currently reap the root after that single
sweep, losing the retained wait identity needed to safely signal a survivor.
Native macOS diagnosis reproduced surviving original-group members; merged
PR #207 fixes pipe inheritance but does not fix this cleanup readiness defect.

## What Changes

Add one nonblocking platform pre-reap step driven by existing caller loops. It
retains one read-only membership worker, observes only after root exit, and
freshly checks the exact unreaped root before each additional ordered signal
sweep. Snapshot admission and polling use the caller's absolute deadline.
The retained worker publishes one completion wake so existing caller loops
can observe finished work before their poll timer expires. Final coverage
disposal exact-reaps an already-exited root while preserving unconfirmed cleanup.
Existing root reap, post-reap absence, pipe gates, primary errors and retained
cleanup remain in their owning callers. Immediate termination and Drop remain
immediate; no signal is permitted after reap or disarm.

## Capabilities

### New Capabilities

### Modified Capabilities

- `native-platform`: permit freshly authorized repeated pre-reap cleanup under
  the existing deadline and require retained, bounded read-only observation.

## Impact

Platform changes are limited to `packages/kuru-platform/src/unix.rs`,
`src/unix/snapshot.rs`, `src/lib.rs`, `tests/unix_process_group.rs` and
`tests/unix_snapshot.rs`. Caller integration changes are limited to connector
`src/hooks.rs`, `src/rpc.rs`, `src/unix_shell.rs` and delivery `src/coverage.rs`.
Public cleanup documentation changes are limited to `docs/protocols.md` and
`apps/kuru-docs/reference/tools.md`. No dependency, workflow, memory/runtime
implementation or broad launch audit is included.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — native cleanup and coverage-helper process ownership
- [x] integration — Unix wait/signal/process snapshot contracts
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
