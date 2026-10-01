# Proposal

## Why

Since the project memory owner retires as soon as no client is attached, a CLI command's service is closing for a short window after the command process exits. A test fixture that then calls the direct `MemoryStore::open` read-only reads only the store's own Dolt endpoint record, which stays published until the retiring supervisor reaps Dolt, and borrows that Dolt without any lifetime guarantee. Main run 36789916109 (ubuntu-latest coverage partition 6) failed this way: `apps/kuru-tui/tests/preferences.rs` `an_explicit_framework_override_is_validated_against_its_own_part_budget` panicked at `preferences.rs:73` with `error communicating with database: expected to read 4 bytes, got 0 bytes at EOF`.

The direct open consults no service authority; the owner lock and lifecycle lease are the authority and endpoint records are discovery hints. The product's managed read-only path (`open_managed_observed` through `attach_existing`) already waits on a held owner lock and decides local versus attached from the locks, so the defect is in the fixture's choice of API and in a false facade contract comment ("The project store lock excludes a live owner"; the store lock is a startup lock only). Before the owner stopped idling for 30 s the borrowed Dolt stayed alive, which masked this.

## What Changes

- The `Sandbox::preferences()` fixture in `apps/kuru-tui/tests/preferences.rs` inspects through the product path (`open_managed_observed`, read-only, after awaiting owner exit), the pattern `trust.rs` already uses, so it is ordered after the prior generation's Dolt reap and owner-lock release.
- Same-class read-only direct opens that run after a CLI command or TUI exit are converted the same way; the deliberate borrow of a live attached owner (fault injection at `terminal.rs` and the in-TUI polling loop) is kept.
- The `MemoryStore::open` facade documentation states the true contract: it consults no service authority, read-only borrows any live published Dolt endpoint without a lifetime guarantee, and callers must order it after owner exit, use `open_managed_observed`, or hold an attachment.
- A deterministic `kuru-memory` test pins the contract the fixture relies on: a managed read-only open meeting a retiring owner waits for its reap and reads its own generation.
- No change to `store.rs` or `server.rs`; no retry, sleep or raised deadline.

## Capabilities

### New Capabilities

### Modified Capabilities

The living specs (`project-memory-service`, `project-memory-owner`, `memory-store-lifecycle`) already require immediate shutdown after the last attachment, endpoint retirement before lock release, and cleanup before lock release; they say nothing wrong about read-only opens, attached inspection or endpoint records, so no delta spec is authored. Only the implementation (fixture and a code comment) was wrong.

## Impact

- `apps/kuru-tui/tests/preferences.rs` (fixture), other test fixtures found by the same-class audit (`terminal.rs`, `embedded_runtime.rs`, `packages/kuru-delivery/tests/support/mise_acceptance.rs`), `packages/kuru-memory/src/facade.rs` (doc comment), `packages/kuru-memory/src/service.rs` (new test).
- No public API, configuration, dependency, or user documentation change; no product behavior change.
- Residual hazard recorded, out of scope: a new starter can elect in the gap between `attach_existing` returning no owner and the managed fallback's local open, so that reader borrows a serving owner's Dolt.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
