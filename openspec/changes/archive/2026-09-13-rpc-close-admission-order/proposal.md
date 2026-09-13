## Why

An MCP RPC worker currently publishes a successful close reply before it stops
accepting commands. A caller awakened by that reply can enqueue a second close
that no worker will service, then incorrectly report that already-confirmed
cleanup remains unconfirmed after the bounded wait.

The native macOS connector run exposed this ordering race after the peer reached
EOF and the first close succeeded. Close needs a clear admission boundary, and a
timed-out reply wait needs to consult the worker's authoritative completion state.

## What Changes

- Close RPC command admission before owned cleanup publishes its result.
- Resolve a timed-out close-reply wait from the existing authoritative completion
  state, while retaining an error whenever cleanup is still unconfirmed.
- Add a deterministic wake-order regression and retain the real double-close and
  uncertain-cleanup coverage.

## Capabilities

### New Capabilities

### Modified Capabilities

- `provider-tools`: Specify that confirmed MCP close publication closes command
  admission first and that repeated close observes the same confirmed result.

## Impact

- `packages/kuru-connectors/src/rpc.rs`: MCP close ordering, timeout fallback and
  focused lifecycle regression.
- Public APIs, cleanup deadlines, process ownership and retained uncertainty
  behavior remain unchanged.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
