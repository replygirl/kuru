# Proposal

## Why

`apps/kuru-tui/src/cli.rs` runs each command's cleanup (tool host shutdown,
harness shutdown, project memory close) and then propagates the primary
result before the cleanup result. Six inner sites (`kuru tools`, `kuru mcp`,
`kuru tool`, `kuru file undo`, the provider turn and `kuru dream`) do
`result?; cleanup?;`, and the outer combine matches
`(Err(primary), _, _)` while surfacing only the diagnostic cleanup. When the
primary operation fails and its cleanup also fails (an MCP child or shell
descendant that cannot be confirmed reaped, a wedged Dolt engine during
`memory.close()`), the cleanup cause is dropped, so the operator never learns
a process or lease was left behind. Found by the fixed-wait audit of
2026-10-02, §3B rank 2 (unit U4a).

## What Changes

- One free combinator in `cli.rs`,
  `fn finish<T>(primary: Result<T>, cleanup: Result<()>, what: &str) -> Result<T>`,
  with the same shape as `packages/kuru-runtime/src/engine.rs` shutdown:
  when both fail the error reads `"{primary:#}; {what} cleanup failed: {cleanup:#}"`
  (primary first). A single failure is returned unchanged, so existing
  error text, error chains and asserted messages are preserved.
- The six inner sites use it. Sites that print a successful value print it
  before combining, so a success whose cleanup fails still writes stdout and
  then returns the cleanup error. A serialization failure of the printed value
  now also combines with cleanup instead of dropping it. Cleanup itself runs
  at the same point and does the same work.
- The outer combine folds `memory.close()` through the same combinator
  (label `project memory`, distinct from the harness's own `memory cleanup`
  text), then applies the diagnostic-cleanup arms with their exact existing
  wording (`diagnostic cleanup also failed; diagnostics may be incomplete`
  as context; the `eprintln!` on otherwise successful runs).
- Unchanged by design (adjacent, out of scope): the provider turn's JSON
  serialization `?` before `harness.shutdown` (a serialization failure skips
  shutdown entirely), and `kuru serve`, whose `serve(..).await?` returns
  before `shutdown(false)`. Neither is one of the audited combination sites;
  changing them alters when cleanup runs.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The specification was right; only the implementation dropped errors.

## Impact

`apps/kuru-tui/src/cli.rs` only (command dispatch and a new unit test
module). No public API, configuration, protocol, constant or budget changes.
Only effect outside the process: when a command and its cleanup both fail,
the error printed on exit names both causes. No interactive surface, flow or
success output changes, so the interactive surface is not declared.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
