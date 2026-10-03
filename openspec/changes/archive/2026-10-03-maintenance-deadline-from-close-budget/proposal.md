# Proposal

## Why

A maintenance permit acquisition (`acquire_maintenance_permit_traced` in
`packages/kuru-memory/src/service.rs`: purge, the maintenance retirement and the
test-support retirement) bounded its start-lock wait, the owner's reply and its
owner-lock wait by one deadline of `memory.startup_timeout_secs` (30 s by
default) from its start. That is shorter than the owner's own close budget,
`server::close_budget()` (first pool drain, Windows lifetime close, supervisor
reap allowance and post-reap drain: 32 s). A maintenance caller that met an
owner closing on its own, or one it had asked to retire, could therefore fail
with `memory service owner is still active; maintenance cannot proceed` (or the
owner-response deadline) while that close was still within the budget the
product allows itself. Measured from code; recorded as a separate item by the
owner-retire-bound diagnosis (lead decision (c)), whose fixture half landed in
#197.

## What Changes

- The maintenance permit's single deadline is now the longer of
  `server::close_budget()` and `memory.startup_timeout_secs`, counted from the
  acquisition's start, for both phases: the start-lock loop and the owner-lock
  wait (including the `timeout_at` around the owner's reply). Neither phase
  bounds only a startup or only a close: the start lock is held by a starter
  (its startup) or by another maintenance acquisition waiting out a close; the
  owner lock is held by an owner still opening (startup), serving (decided by
  busy replies, not the deadline) or closing (close budget). Taking the longer
  figure keeps a configured startup timeout above 32 s in force behind a
  starting owner, and gives a closing owner its whole close budget.
- No new constant: both figures are existing product figures.
  `close_budget()` and `startup_timeout_secs` are unchanged.
- The error texts are unchanged: `memory maintenance election deadline
  exceeded`, `memory maintenance owner-response deadline exceeded; <trace>` and
  `memory service owner is still active; maintenance cannot proceed; <trace>`.
- Tests that used `startup_timeout_secs = 1` to make the permit fail fast now
  wait the close budget: the purge refusal test on paused time (a bare owner
  lock, no socket) and asserting the wait reached the close budget; the
  live-owner-closing-connections test on the real clock (its replies are real
  sockets), about 32 s.
- Docs: `docs/development.md` (the retirement fixture's asking deadline is no
  longer equal to the attempt's; the "aligning it is separate work" note is
  replaced) and `docs/memory.md` (the maintenance wait bound).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The `project-memory-owner` and `project-memory-service` specs do not
state the maintenance permit's deadline.

## Impact

- `packages/kuru-memory/src/service.rs`: `maintenance_deadline`, used by
  `acquire_maintenance_permit_traced`; regression test and two deadline tests.
- `packages/kuru-memory/src/test_support.rs`: two comments only (the fixture's
  asking deadline is no longer equal to an attempt's own).
- `docs/development.md`, `docs/memory.md`.
- Behaviour: maintenance against a live owner that never releases its lock
  fails after 32 s instead of 30 s at the default configuration.
- Sibling not changed here: a starter's wait behind a closing previous owner
  (`attach_or_spawn_elected`) is still bounded by `startup_timeout_secs`, the
  same 30 s < 32 s family, specified by `project-memory-owner`.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
