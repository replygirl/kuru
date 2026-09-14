## Why

The final PR verification reached Ubuntu bundle setup, then the pinned archive
response body timed out after roughly 127 seconds and failed the job before any
coverage assertion ran. The delivery helper retries selected temporary HTTP
statuses, but a transient connection timeout or interrupted response body exits
immediately even though the bounded pinned-archive GET is safe to repeat.

## What Changes

- Retry transient connection, timeout, and response-body interruption failures
  for the pinned archive GET within the existing three-attempt and 120-second
  overall download bounds.
- Give each attempt a 15-second connect bound and 30-second read-idle bound,
  truncate and rewind the private staging file before another attempt, and
  create a fresh SHA-256 state for every attempt.
- Keep permanent HTTP responses, retry-advised responses, size or digest
  failures, private publication, and the total download deadline fail-closed.
- Add bounded local HTTP fixtures for an interrupted body followed by success,
  persistent transport failure, and staging reset between attempts.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

The implementation and focused recovery fixtures are owned by
`packages/kuru-delivery/src/bundle.rs` and
`packages/kuru-delivery/src/bundle/recovery_tests.rs`; `docs/development.md`
describes the corrected preparation policy. No runtime API, dependency,
workflow topology, bundle manifest, size/hash rule, or publication policy
changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
