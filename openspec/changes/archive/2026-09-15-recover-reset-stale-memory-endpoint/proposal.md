## Why

Windows lifecycle CI observed a reaped database Job whose old TCP endpoint still accepted a raw connection but reset the authenticated SQL connection before SQLx entered its `after_connect` callback. Kuru treated that narrow pre-authentication connection reset as proof that the published memory endpoint was live, so it returned the reset instead of continuing through the existing lifecycle lease and owned startup path.

The endpoint owner and reason for the reset are unknown. The implementation can still distinguish the observed unavailable endpoint from authentication, identity, data-directory, protocol and SQL failures by using SQLx's typed error and callback boundary.

## What Changes

- Treat only `sqlx::Error::Io` whose error kind is `ConnectionReset`, observed before the connection's `after_connect` callback runs, as an unavailable published endpoint.
- Continue that result through the existing lifecycle-lease and owned-start path; retain all other connection and validation errors as terminal.
- Add a deterministic owned loopback reset-listener regression that first accepts the raw probe and then resets authentication, proving the pre-fix failure and post-fix recovery of committed state.
- Preserve all production retry, timeout, authentication, identity, storage and process-ownership policy.

## Capabilities

### New Capabilities

### Modified Capabilities

- `memory-store-lifecycle`: Clarify the exact unavailable-endpoint observation that permits an owned memory server restart after a prior owner has been reaped.

## Impact

The change is limited to `packages/kuru-memory/src/server.rs`, `packages/kuru-memory/tests/windows_lifecycle.rs`, and the `memory-store-lifecycle` contract. It adds no dependency, public API, schema, retry, timeout or platform-policy change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
