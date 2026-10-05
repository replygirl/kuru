# Proposal

## Why

The memory owner serializes mutations, but callers cannot condition a state write
on the version they read. Phase 2 topology safety needs an atomic compare and
publication primitive before separate sessions may write shared membership.

## What Changes

- Add memory-owned versioned state reads, bounded ordered value-only and versioned batch reads, and
  all-or-nothing conditional state publication with typed stale refusals.
- Schema 9 adds a nonnegative state row version. Existing state writes advance
  versions without changing their unconditional semantics; candidate views retain
  their own versions and exact fast-forward/recovery discipline.
- Extend the existing facade and private service operation/receipt/fault contract,
  with one protocol minor increment and pin update.
- Keep topology keys and user behavior unchanged. Topology materialization belongs
  to the following runtime split migration, because current legacy writers would
  otherwise leave early split values stale.

## Capabilities

### New Capabilities

### Modified Capabilities

- `versioned-memory`: revision-conditional state publication and bounded reads.
- `project-memory-service`: typed conditional state operations and stale responses.

## Impact

Memory store, versioned-state module, migration registry/validation, facade,
private RPC contract and tests; owning memory/development docs. No new dependency,
provider, runtime admission or core topology type. Older executables refuse the
new schema at writable open; historical read-only views remain preserved.
Export stays value-only and does not expose row versions.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
