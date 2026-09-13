## Why

Persistent workspace approval can fail on Windows while replacing its pending
record. The approval store retains its private storage directory with pinned
names, and files opened through that handle inherit no-delete sharing; Windows
then rejects the store's own checked `MoveFileExW` publication while the pending
source handle is still held.

The failure prevents an explicit `trust approve` action from atomically
publishing a valid approval record even though the checked storage directory,
candidate identity, and destination are safe. The correction must preserve the
retained workspace and storage identity anchors rather than weakening name
retention or native publication checks.

## What Changes

- Keep the approval storage directory pinned for the whole approval, inspection,
  replacement, and revocation operation.
- Open a second checked owner-only movable directory capability only for pending
  record creation, checked replacement, reconciliation, and removal, and prove
  it identifies the retained storage directory before use.
- Exercise approve, inspect, replace, and revoke against real filesystem state;
  retain the existing private DACL sealing and fail-closed behavior.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The current workspace-trust and native-publication requirements already
require private atomic approval updates and held identity checks; this corrects
their Windows implementation.

## Impact

- `apps/kuru-tui/src/trust.rs`: scoped movable approval-record operations while
  retaining the pinned storage authority.
- `apps/kuru-tui/tests/trust.rs`: real approval publication, inspection,
  replacement, and revocation regression coverage.
- No platform API, public configuration, credential format, dependency, or
  release behavior changes.

## Surfaces

- [x] interactive — persistent `trust approve` and `trust revoke` behavior.
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
