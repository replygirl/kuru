## Why

The archived `lifecycle-hook-authority-cleanup` change (9fdfd30d) added pre-dispatch admission: the runtime now settles a call whose name was not offered for that request and phase, or that a `pre_tool` hook denied, failed or invalidly rewrote, before it reaches the ToolHost. That settlement path records the public tool outcome but emits no `kuru.tool` operational record. Before the change, an unoffered call reached the ToolHost, failed there, and left a `status: error` tool record in the private diagnostics ring.

The loss is deterministic. The pre-push coverage run of #89 failed `unix_shell_turn::debug_cli_rotates_the_fixed_private_diagnostic_ring` (`logs.contains("\"status\":\"error\"")`), and it fails 3 of 3 isolated runs on e167d96b. A ring dump from a failing run holds 504 provider stream-reconciliation records, provider/actor/turn completions and no `kuru.tool` record at all. The same test passes 10 of 10 on `fix/restore-green-main`. Beyond the test, someone running `--debug` to diagnose a misbehaving pre-tool hook finds no trace of its denials.

## What Changes

- **Settled admissions are recorded.** At both runtime admission sites (speaking and deliberation), a call settled before dispatch emits one `kuru.tool` event with `operation: admission`, the tool category (`external` or `cognitive`), its status and its admission latency. It carries no tool name, arguments or hook reason.
- **One status mapping.** External, cognitive and settled records share one status function (`ok`, `error`, `cancelled`; a failed shell receipt is `error`), so ring status agrees with the public tool outcome.
- **Test states its source.** The rotation test now also requires an `admission` record and asserts that the model-chosen tool name stays out of the ring.
- **Docs.** The diagnostics ring description in `docs/usage.md` and `apps/kuru-docs/reference/commands.md` names these records and what they omit.
- Dream proposals and budget-exhausted calls stay as they were: they had no tool record before `lifecycle-hook-authority-cleanup` either.

## Capabilities

### New Capabilities

### Modified Capabilities

- `operational-diagnostics`: tool observations cover calls settled by pre-dispatch admission, not only external tool execution.

## Impact

- `packages/kuru-runtime/src/engine.rs`: `tool_diagnostic_status` and `trace_settled_admission`, used at both settled admission sites and by the external and cognitive records.
- `apps/kuru-tui/tests/unix_shell_turn.rs`: stricter rotation assertions.
- Docs: `docs/usage.md` and `apps/kuru-docs/reference/commands.md`.
- No schema, dependency or public output change. The ring's field allowlist is unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
