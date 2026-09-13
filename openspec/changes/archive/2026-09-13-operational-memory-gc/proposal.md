## Why

Kuru currently retains every mutation receipt in active branch state and never
reclaims a candidate branch after its lifecycle has resolved. Its managed Dolt
server also disables the pinned engine's bounded automatic garbage collection,
so storage grows from obsolete operational state in addition to the user history
that Kuru deliberately preserves.

## What Changes

- Keep only the current mutation receipt after prior uncertainty has reconciled,
  within the existing serialized transaction.
- Record explicit candidate promotion and abandonment as recoverable branch-ref
  transitions, and reclaim only exact resolved branches after owned sessions
  have settled.
- Enable the pinned Dolt engine's growth-triggered automatic GC under the owned
  server lifecycle, without adding an application maintenance service.
- Document that conversations, notes and reachable revisions do not expire and
  that GC is maintenance rather than secure erasure.

## Capabilities

### New Capabilities

### Modified Capabilities

- `versioned-memory`: Bound operational receipts, resolve candidate branch
  lifecycles conservatively and run pinned engine GC without pruning history.
- `chat-harness`: Explicitly abandoned dream candidates are settled and reclaimed
  without weakening cancellation or accepted-write reconciliation.
- `public-documentation`: Describe automatic storage maintenance and its retention
  limits in the owning memory documentation.

## Impact

The change affects `kuru-memory` store/server lifecycle code and real-Dolt tests,
the runtime dream cleanup path and focused cancellation tests, and the two owning
memory documentation pages. `Candidate` gains one explicit abandonment method.
There is no schema migration, dependency change, export format change, retention
setting or user-content deletion.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
