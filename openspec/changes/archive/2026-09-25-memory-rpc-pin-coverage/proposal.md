## Why

Independent review of `memory-rpc-operation-contract` confirmed that behavior is
preserved exactly, but found gaps in the protocol pin. Some nested wire types
had only one sampled shape: the export cursor's phase, three of the four usage
proofs, every content block except text, most session-turn transitions and the
legacy transcript position. The invocation price schedule was sampled as null.
A wildcard also remained on the unit-receipt view path. Messages carry most of
the traffic, so their nested shapes must be pinned per variant. Regeneration
also needs a legitimate route for a fixture introduced by the same unmerged
change, while still refusing an unchanged or lower version.

## What Changes

- Add per-variant samples for the export cursor phase, `UsageProof`,
  `ContentBlock`, `SessionTurnCheckpoint`, `PublicTranscriptPosition`,
  `PriceBasis` and `CacheWriteTerms`. Each set is checked against serde's
  variant list. Populate `price_at_invocation` and every other optional field.
- Pin the phase, price basis and cache-write enum tag lists.
- Replace the wildcard in `unit_receipt_view` with an explicit arm for every
  `ServiceCall` variant.
- Use the contract reply budget in the test-support paused exchange.
- Read the fixture at test time. Regeneration writes it only when no fixture
  exists or the current protocol version is strictly greater than the recorded
  one.
- Regenerate the fixture at 1.7 through that tooling. It was introduced by the
  same unmerged branch, so no released surface changes.
- Document one sampled shape per variant, the nested-enum limits and the
  residual golden-file risks.

Invariants: wire format, protocol version 1.7, the contract classification,
receipts, locks, deadlines and refusals are unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None.

## Impact

Affects `packages/kuru-memory/src/service/rpc.rs` (the explicit arm list and the
paused reply budget), `service/rpc/contract_tests.rs`, `service/rpc/protocol-surface.txt`
and `docs/development.md`. No public API, dependency or wire change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
