# Proposal

## Why

PR #221 run 37195862185, macOS coverage partition 2, failed the persistent foreign
Dolt regression with a bootstrap data-directory query timeout instead of retaining
the foreign-directory mismatch. `start_database` remembers a proven mismatch but
discards it when a later probe crosses the original startup deadline. The hosted
log proves the timeout branch; it does not record individual earlier probes, so
the earlier mismatch in that hosted execution remains an inference pending the
deterministic local regression.

The same signature also blocked PR #213 at `a08cde43`, run 37212256637,
Ubuntu 24.04 ARM partition 2, job 111465938860: the assertion at
`server_tests.rs:1171` received the same data-directory timeout, with 250 passed,
one failed and one ignored in 266.01 seconds. Neither hosted failure was rerun.

## What Changes

- Retain a previously observed foreign-directory mismatch when a later directory
  probe exceeds the original deadline. The error describes the retained observation;
  the last timed-out probe does not establish a new mismatch.
- Clear that observation immediately when a probe verifies the correct directory,
  before any bootstrap write, preserving later own-server bootstrap failures.
- Exercise both paths against real Dolt with task-local deterministic stalls.
  Keep the existing deadline, attempt bound, identity checks and no-write guarantee.
- Give the own-directory regression the configured product startup budget for
  its real query, then advance only its injected bootstrap stall virtually.
  Resume that isolated test runtime's clock before cleanup.
- Await the foreign fixture's teardown before reporting assertion failures.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/server.rs` and `server_tests.rs` only, plus this record.
- No public API, dependency, database format, configuration or workflow change.
- Existing port-collision documentation remains accurate; no user command changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
