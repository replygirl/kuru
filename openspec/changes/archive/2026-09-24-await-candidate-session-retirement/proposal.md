## Why

Candidate abandon and promotion can reach Dolt's branch rename while another server session still uses the source branch. Exact #71 native macOS coverage failed both a managed candidate-abandon recovery and a cancelled promotion: the owner classified the failure at `branch_rename` with SQLSTATE `HY000` and vendor code `1105`, and the runtime fixture exposed Dolt's explicit active-session rename refusal. A local four-connection retirement probe saw prompt teardown, so the failure is conditional; the earlier Windows selected-abandon `StorageFailed` remains unclassified rather than presumed identical.

## What Changes

- Retire the candidate pool and await server-side disappearance of every session on the exact source branch before attempting a candidate status rename. A source-local admission fence prevents a new Kuru connection during the transition; other branches remain available.
- Preserve the current checked source/head, bounded deadline, typed uncertain outcome and exact recovery behavior. Timeout refuses to begin a rename; there is no force option, blind retry, detached cleanup or replay of an uncertain request.
- Retain the test-only owner stage/error diagnostic and add focused real-Dolt proof for multiple owned sessions, a controlled held-session refusal and the prior abandon/promotion paths.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The existing candidate isolation and checked-transition requirements were correct; this fixes their implementation.

## Impact

`kuru-memory` candidate-pool retirement and branch-transition sequencing, plus its native fixtures; runtime/CLI candidate acceptance supplies cross-package verification. No public RPC, storage schema, user command, or dependency change is intended.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
