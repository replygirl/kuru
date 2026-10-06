# Proposal

## Why

After split topology storage, dreams own membership, undo and candidate-local notes/history while ordinary turns own reports, session records and journals. Promotion still refuses every moved live base, so unrelated live turn progress can strand an otherwise valid dream; safe later concurrent admission needs owner-checked reconciliation without replaying inference or losing either history.

## What Changes

- Serialize dream work through the existing project memory owner's dream lease, retaining candidate-before-snapshot/inference order and current mode policies.
- Reconcile a clean exact open candidate with then-latest committed live memory inside the owner, merging live into the private candidate only. Promote live only by checked fast-forward to the reconciled exact head.
- Preserve live reports/session/journal/usage/history and candidate-owned notes/tool history/membership undo. Overlapping membership or message rows produce an explicit recoverable conflict with no live publication or automatic resolution.
- Add typed receipted reconciliation and exact lost-reply/cancellation/restart recovery, adopting the proven candidate head and latest reconciled live base before another mutation, promotion or abandonment.
- Keep raw session history private; approved cross-session notes/summaries keep their existing mode policy and provenance. Verify sentinel isolation and merged unattributed dream rows below a session compaction cursor.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `session-scoped-memory-provenance`: moved-base dream reconciliation, explicit overlapping-row conflict and retained session-filtered raw context.
- `project-memory-service`: owner-internal candidate reconciliation with exact request outcome and attachment/restart recovery; no public merge command.
- `versioned-memory`: checked candidate merge into a clean private working set, exact reconciled-base fast-forward and conflict rollback.

## Impact

Owning memory candidate/store recovery, facade pending outcomes, service RPC/contracts/wire pin and real-Dolt fixtures; runtime dream/pending-publication recovery and controlled-provider fixtures; owning memory/session/development docs and capability deltas. No schema change is planned. Protocol advances by the next coordinated minor; older owners reject new calls before acceptance. B's schema-10 storage is required; hosted B acceptance remains separately pending.

This slice retains the project conversation-driver gate and existing concrete MemoryStore. It introduces no N4 admission, lineage feature, provider daemon/control plane, generic merge/field merge framework, new model-quality evaluation or provider/tool authority. C1's summary-ID metadata confirmation and compaction events are separate; shared files are integrated and reviewed under explicit source ownership before implementation.

## Surfaces

- [ ] interactive — existing explicit candidate inspection/abandon commands remain; no new UI flow or CLI command
- [ ] deploy — no CI/runtime topology or provider process change
- [x] integration — Dolt merge/ancestry and private protocol/receipt identities
- [x] agent-behavior — successful dream publication after unrelated live progress, with existing candidate prompt/tool and mode policy retained
