# Proposal

## Why

The Responses provider already owns a `reqwest::Client`, whose connection pool can reuse a completed HTTP/1.1 connection while that provider instance remains alive. The repository currently lacks a fixture that counts actual accepted TCP sockets across multiple completed requests, so the intended warm-connection behavior is not demonstrated and can regress unnoticed.

## What Changes

Add a loopback HTTP fixture that completes two ordinary provider requests on one provider instance, counts accepted connections, and verifies distinct request/actor state. Compare it with the measured fresh-provider-per-request baseline. Preserve current request and SSE completion semantics; do not add a drain worker, connection-retention timer, retry, timeout, or provider-wide lifecycle abstraction. Document the measured HTTP behavior separately from the memory service attachment and its zero-client retirement behavior.

## Benchmarks

| Metric | Baseline | Reused provider | Measurement |
|---|---:|---:|---|
| Accepted TCP connections for two completed requests | 2 (one fresh provider/client per request) | 1 (one provider/client for both requests) | Counted accepted loopback sockets while asserting all four responses completed and each captured actor input remained distinct; 50% fewer accepted sockets in this fixture, with no latency claim |

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This is a performance and evidence change; observable provider behavior remains identical.

## Impact

The change is expected to touch `packages/kuru-connectors/src/providers/subscription_tests.rs` and narrowly scoped connector or development documentation. No dependency, provider API, memory/runtime implementation, or configuration change is planned. The separate scripted-memory-service evidence will use the already accepted N4 process/lifecycle proof and will not claim service retention after the final client exits.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
