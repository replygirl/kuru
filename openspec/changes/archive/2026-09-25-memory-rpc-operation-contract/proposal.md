## Why

The memory service classifies every typed operation in three places that are
not exhaustive. `ServiceCall::may_mutate` uses a `matches!` list for view
operations, `unit_receipt_method` ends in `_ => None`, and `receipt_progress_key`
derives from both. A new operation therefore defaults to "read-only, no
receipt": the client takes no mutation lock or lost-reply fence and the owner
records no logical receipt, so a lost reply can no longer be proven. Nothing
also ties a wire change to a `PROTOCOL_MINOR` bump, so a newer client can reach
an older idle owner with a variant it rejects as an opaque decode failure
instead of the clear incompatible-protocol diagnostic. Session claims and
backup calls are about to add operations, so the contract must become a compile
and test failure before they land.

## What Changes

- Add one exhaustive, wildcard-free operation contract in
  `packages/kuru-memory/src/service/rpc.rs`: `ServiceCall::contract()`, which
  delegates to `ViewOperation::contract()` and `LedgerOperation::contract()`.
  Each entry states mutation class, receipt class (none, a named unit receipt,
  candidate creation, candidate transition, selected abandonment or usage
  proof) and reply budget class.
- Make `may_mutate`, `unit_receipt_method` and `receipt_progress_key` read that
  contract. Share one unit-receipt view helper between progress registration
  and dispatch. The attached client's reply read uses the contract's reply
  budget, which is the existing 35-second operation deadline for every call.
- Add `AttachmentState::holds_handles()` and `holds_resources()` and use them at
  the existing idle-retirement, dream-lease and selected-abandon checks, with
  exactly today's predicates at each site.
- Add a table-driven test pinning today's classification of every
  `ServiceCall`, `ViewOperation` and `LedgerOperation` variant, and an invariant
  test that every mutating call has a receipt class except `retire_if_idle`.
- Add a golden protocol-pin test with a checked-in fixture of the wire surface:
  `PROTOCOL_MAJOR.PROTOCOL_MINOR`, one serialized sample shape per request
  variant, and the variant tags of the response enums, with request coverage
  taken from serde's own variant list. It fails with bump-and-regenerate
  instructions when the surface changes, and its regeneration path refuses to
  record a changed surface under an unchanged protocol version.
- Document the protocol-change procedure in `docs/development.md`.

Invariants that must not change: the serialized wire format and protocol
version (1.7); which calls take the client mutation lock and lost-reply fence;
every durable receipt method name and receipt fingerprint; which requests
register receipt progress and under which view key; retirement, dream-lease and
selected-abandon refusals; error texts; reply deadlines.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. Existing `project-memory-service` and `durable-service-receipts`
requirements remain unchanged; this change enforces them structurally.

## Impact

Affects `packages/kuru-memory/src/service/rpc.rs`, a new checked-in fixture
under `packages/kuru-memory/src/service/`, and `docs/development.md`. No public
API, dependency, schema, persisted format, wire format or protocol version
changes. `facade.rs` and `service.rs` callers keep their current methods.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
