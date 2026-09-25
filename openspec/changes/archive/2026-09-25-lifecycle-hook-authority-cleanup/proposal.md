## Why

The P14 lifecycle-hook review (PR #89) found authority, cleanup and calibration defects that the archived `lifecycle-hooks` change left open.

- **Tool name rewrite.** A `pre_tool` hook could rewrite a proposed tool's name as well as its arguments. The final name was never checked against the tools actually offered in that phase. In deliberation, a hook could turn a peer's `remember` call into `a2a_send`, which deliberation never offers.
- **Unawaited cleanup.** Hook process cleanup after caller cancellation ran on a detached worker that nothing awaited. A turn could complete, or the harness could shut down and release its project writer lease, while a hook tree was still being reaped.
- **Unbounded reads.** After a hook's root process exited, its stdout and stderr reads had no deadline. A descendant that escaped the owned group while holding stdout could therefore hang the turn.
- **Hook-authored text stored as user speech.** A pre-turn rewrite was stored in each part's private history as a plain `user` message, indistinguishable from what the user said.
- **Silent suppression.** An inherited `KURU_INTERNAL_LIFECYCLE_HOOK_ORIGIN=1` disabled every configured hook with no diagnostic.
- **Timing-dependent tests.**
  - The shared-budget test depended on wall-clock slack.
  - The Windows hook test hid a stall by raising each hook's timeout to the 120 s product maximum.
  - The cancellation test could pass without the hook ever starting.

## What Changes

- **`pre_tool` rewrites.** A rewrite may change only the arguments. A changed tool name fails closed and no later hook runs. The runtime admits a call it dispatches itself (every deliberation and dream call, and cognitive speaking calls), whether hook-rewritten or model-proposed, only when its name is among the tools offered for that exact request and phase. Other speaking calls pass the ToolHost admission and permission evaluation of their exact final name and arguments.
- **Awaited cleanup.** Each `HookHost` tracks its in-flight owned hook workers. A turn, dream or harness/tool shutdown awaits their bounded cleanup (signal before reap) before it completes, and unconfirmed cleanup is surfaced.
- **Bounded reads after exit.** After the root process exits, stdout and stderr are drained only within the remaining deadline. If the drain expires, the hook fails closed.
- **Pre-turn rewrite provenance.** When a pre-turn hook rewrites the input, each current input is preceded by a `kuru-hook` provenance record in durable history. The rewritten text is therefore attributable to a hook and never stands alone as user speech. The original stays in the public transcript.
- **Visible suppression.** Configured hooks suppressed by the internal origin variable are reported at every lifecycle boundary as typed `suppressed` hook observations. The documentation names the variable.
- **Windows environment.** A hook's finite environment carries an inherited `PSModulePath`, except when the command explicitly selects stock Windows PowerShell. Hooks do not receive the ToolHost `$PSHOME` bootstrap.
- **Tests.**
  - Budget accounting is tested with an injected clock, plus a real-process test synchronized by readiness markers.
  - Windows hook tests warm the exact hook launch path outside the timed budget and use the 5 000 ms product default.
  - Waits derive from configured hook budgets.
  - Runtime hook tests assert typed events rather than message text.

## Capabilities

### New Capabilities

### Modified Capabilities

- `lifecycle-hooks`:
  - `pre_tool` rewrites change arguments only, and the final call is admitted only against the tools offered for that request.
  - Cleanup is awaited before completion and before shutdown.
  - Post-exit drains are bounded.
  - Pre-turn rewrite provenance is recorded.
  - Suppression is visible.

## Impact

- **`packages/kuru-connectors`:**
  - `src/hooks.rs`: protocol, budget clock, worker tracking, bounded drain, suppression and environment.
  - `src/tools.rs`: shutdown awaits hooks.
- **`packages/kuru-runtime`:**
  - `src/engine.rs`: offered-set admission, provenance records and quiescence.
  - `src/dream.rs`, `src/actor.rs` and `src/event.rs`: the `suppressed` outcome.
  - `src/hook_tests.rs`.
- **`apps/kuru-tui`:** hook event display, if affected.
- **Docs:** `docs/configuration.md` and `docs/protocols.md`.
- **Dependencies:** no new dependencies.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
