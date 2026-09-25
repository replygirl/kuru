## Why

After a context-summary cursor advances, later request assembly needs the newest bounded raw rows strictly after that durable cursor. The existing sequenced snapshot returns the oldest forward page, while the existing newest session window omits durable sequences, so more than 1,024 eligible rows cannot be selected correctly through one bounded checked read.

## What Changes

- Add a read-only local and managed-memory query for the newest bounded sequenced suffix after an exclusive cursor.
- Bind the result to the exact session, namespace, pinned live or candidate view and captured revision, with an exact eligible-row count.
- Apply the existing 1,024-row and 32 MiB session-source bounds in one checked read, without paging, export access or mutation receipts.

## Capabilities

### New Capabilities

### Modified Capabilities

- `versioned-memory`: Add the checked session-cursor history window used by later context assembly.

## Impact

- `packages/kuru-memory` gains one typed DTO, store query, facade method and read-only managed RPC operation.
- The managed-memory protocol minor version advances because the RPC enum gains a new operation and response.
- No schema migration, user command, provider behavior or writable authority changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
