# Spec Delta

## MODIFIED Requirements

### Requirement: Coherent per-turn topology

The runtime SHALL reload and validate one versioned topology snapshot after memory reconciliation and before each turn's actor admission. It SHALL retain that snapshot for the complete turn and SHALL NOT mix a topology committed during the turn into already admitted actor work. A small complete-inventory load MAY use one coherent versioned batch, accepting only the same full membership value and version captured by its header read. Otherwise membership, the current session and the complete report prefix SHALL use one immutable revision cut. Independent live pages or active-only reconstruction MUST NOT silently omit retained reports. Each admitted conversation turn MUST retain its checked session driver authority and existing session-private raw continuity while shared membership/reports and dreaming keep their established ownership and merge policies.

#### Scenario: Topology change appears on the next turn
- **WHEN** another accepted memory operation changes stored membership or reports after one turn captures its graph
- **THEN** the active turn completes against its retained graph and the next turn reloads the changed complete graph.

#### Scenario: Large or unknown report inventory
- **WHEN** the complete retained report inventory cannot be proven to fit one small coherent batch
- **THEN** one immutable cut assembles membership, session and every report with archived and extra identities intact.

## ADDED Requirements

### Requirement: Driver-bound private mutation and ownership-loss cancellation

Session-bound raw appends, checkpoints, journals/session records and compaction summaries MUST validate their current driver proof under the owner's mutation guard before an effect or receipt. A stale, absent, wrong-session or old-generation claim MUST produce a definite no-effect refusal. Driver loss MUST cancel the existing shared operation token before further provider/tool dispatch, preserve accepted durable receipt/journal recovery, and drain locally owned actors/hooks/tools before releasing the session's native exclusion. Kuru MUST NOT infer rollback, nonexecution or remote completion for an external request already accepted before loss. Such effects retain existing pending/uncertain semantics and MUST NOT be automatically replayed after another driver is admitted.

#### Scenario: Owner loss during provider or shell work
- **WHEN** actual presence or owner-generation loss occurs while provider or owned shell work is admitted
- **THEN** no later tool/provider dispatch is admitted under the old claim, stale private mutation refuses, existing cancellation and checked local cleanup run while native exclusion remains held, and already accepted external effects remain honest journal outcomes.

#### Scenario: Concurrent private sentinels and shared policy memory
- **WHEN** two actual processes concurrently drive different sessions containing distinct raw/opaque actor and relationship sentinels plus policy-approved notes and summaries
- **THEN** each provider context contains only its own raw sentinels and its mode's approved shared records, including after shared dreaming and undo, while full inspection retains both histories.

#### Scenario: Accepted checkpoint precedes claim loss
- **WHEN** a checkpoint is accepted and its reply is lost before the claim or owner ends
- **THEN** exact receipt recovery establishes that checkpoint once, stale replay is refused, and subsequent publication or interruption never guesses nonexecution from claim loss.
