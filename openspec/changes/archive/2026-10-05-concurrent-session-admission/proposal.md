# Proposal

## Why

Kuru currently excludes every second conversation in one project through the CLI's project writer lock, even though the memory owner already serves independent clients and N2/N3 preserve session-private history while serializing shared membership and dreaming. Ordinary invocations should be able to drive distinct sessions together without granting two drivers the same session or letting a lost owner leave stale work authorized.

## What Changes

- Replace ordinary conversation admission through the project writer lock with memory-owned, generation-bound session driver claims. Each ordinary invocation still starts a fresh session unless the caller explicitly selects resume or continue; a second surface for an already-driven session is refused.
- Give a driver one dedicated retained presence connection and share its authenticated client identity across its existing memory exchange connections. Owner-guarded session writes and lifecycle changes consult the same claim state; actual connection loss releases volatile ownership without an idle timer.
- Make selection a checked atomic owner operation against the old claim and the exact selected target catalog generation. A refusal preserves the old claim and local selection; cancellation or lost reply retains exact pending selection proof and fences driver work until its outcome is known.
- Cancel admitted actor/provider/tool work when claim or service generation is lost, drain existing owned cleanup, and require checked reacquisition before later work. Distinguish prevented new dispatch and refused private writes from already accepted external effects whose outcome cannot be undone or inferred from transport loss.
- Add honest live-presence metadata to existing session listings and picker controls. Standalone inspection must distinguish unavailable presence from an empty live set.
- Retain maintenance exclusion, migration ownership, exact receipts, immediate zero-client service retirement, mode-policy notes/summaries, session-private raw histories and project-wide serialized dreaming.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `project-memory-service`: checked session claims, shared client identity, dedicated presence lifetime, exact volatile transition recovery and owner-generation refusal.
- `session-lifecycle`: simultaneous distinct-session admission, checked selection/lifecycle exclusion and honest CLI/TUI live presence.
- `session-scoped-memory-provenance`: concurrent private-history isolation and driver checks at session-bound mutation boundaries.

## Impact

Memory owns claim state, service authentication/operations, facade lifetime and exact outcome handling; runtime owns staged selection and cancellation of admitted work; CLI owns narrowing its project lock to maintenance; UI owns presence and checked picker actions. Update owning docs and actual managed/native-process/PTY fixtures. No durable registration table, schema migration, new dependencies, private-history lineage, control API, scheduler, idle timer, attachment-cap expansion, provider daemon or change to inference routes.

The additive managed protocol surface and its compatibility floor/pin advance together from the accepted N3 baseline. Older writable clients must receive an actionable incompatibility refusal rather than bypass driver checks. Existing archive/provisioning/privacy and 100 MiB operation-envelope semantics stay in force.

## Surfaces

- [x] interactive — session admission, listing and picker actions
- [x] deploy — concurrent client/owner lifetime and native process cleanup
- [ ] integration — no third-party contract is added
- [x] agent-behavior — admitted work stops on ownership loss; no prompt or model-policy change
