# Proposal

## Why

Ordinary runtime writes still replace a whole project topology blob, so a stale
report or session update can overwrite membership and other identities' reports.
The conditional-state foundation now permits separating shared membership,
per-identity reports and session focus while retaining the complete public
Topology and the existing single-driver behavior.

## What Changes

- Materialize membership and every retained report from the then-latest legacy
  topology during schema migration; keep original legacy bytes and ambiguous
  project focus unassigned.
- Add memory-owned immutable committed state read cuts with explicit close,
  attachment/generation ownership and bounded pages. Preserve larger inventories
  and previously legal large rows under the existing exact service envelope.
- Integrate coherent small-batch loading with immutable-cut fallback; publish
  membership conditionally, reports per identity and focus with its session.
- Preserve last-report semantics, exact pending-publication recovery, candidate
  isolation and the conversation-driver lease. Dream candidates write membership
  without overwriting session focus or reports.

## Capabilities

### New Capabilities

### Modified Capabilities

- `versioned-memory`: immutable state cuts and compatible split migration.
- `project-memory-service`: checked cut handles and exact bounded envelopes.
- `session-scoped-memory-provenance`: membership/report/session ownership.

## Impact

Memory store, schema registry/compatibility transform, facade, private RPC and
protocol pin; core state-key policy and runtime topology loading/publication,
dream/undo and their fixtures; owning user/development documentation. The next
schema and private protocol minor refuse older writable owners through existing
compatibility checks. No dependency update, new storage hierarchy, admission,
provider change, candidate merge or public Topology shape change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — durable schema, checked identity encoding and typed memory protocol reconciliation
- [x] agent-behavior — candidate snapshot prompt inputs and owned state-report tool publication; existing provider/model/output contracts remain unchanged
