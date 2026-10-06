# Proposal

## Why

The fake stdio peer can be forcibly terminated by the existing MCP failure tests while its exit-time LLVM profile is being written. Windows strict profile export repeatedly rejects a tiny corrupt profile; timestamp and compiler-layout evidence point strongly to this fixture, while PID reuse prevents exact historical writer attribution.

## What Changes

- `packages/kuru-connectors/src/test_support.rs`: compile the existing std-only peer without coverage instrumentation on Windows as on Unix, retaining platform-owned compiler completion and checked immutable snapshots.
- Extend the existing Windows snapshot fixture to prove that the compiled fake peer writes no raw profile even with `LLVM_PROFILE_FILE` present; preserve existing protocol-failure, timeout and cleanup assertions.
- Document the test-scaffolding boundary and qualified recurrence evidence in `docs/development.md`.

## Impact

Only fake-peer preparation changes. The pinned compiler's coverage annotation remains unstable, so no feature/bootstrap workaround is used. Existing default coverage scope excludes `tests/fixtures`; all application modules, real child profile destinations, strict export, the 90% gate, MCP forced cleanup, tools and pins remain unchanged. Native Windows acceptance remains a hosted check; local host evidence and Windows compilation are reported separately.
