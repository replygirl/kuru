# durable-service-receipts Specification

## Purpose
Define branch-scoped durable receipts and typed outcome recovery for uncertain
mutations sent through the private local memory service, while preserving
historical Dolt views and preventing duplicate effects after a lost reply.

## Requirements

### Requirement: Stable request-bound durable mutation receipt

For each accepted current-schema managed receipt-bearing unit storage mutation, Kuru SHALL bind one stable logical mutation ID to the operation kind, exact pinned view and compact fingerprint of its canonical encoded arguments. Its durable idempotency key is the store instance, pinned view and logical ID together. It SHALL retain queryable outcome evidence in the existing indexed, branch-local operation receipt mechanism in the same committed mutation; later writes MUST NOT erase that evidence. An exact repeated unit-mutation key with a different method or argument fingerprint MUST fail without an effect. A receipt on another view MUST NOT be treated as the queried view's outcome; branch-local rows do not claim global UUID uniqueness. Legacy receipts without a request fingerprint MUST remain historical and MUST NOT be presented as a matching managed request. Candidate transitions and usage-ledger operations use the distinct typed result evidence below; this indexed-row exact-retry rule does not authorize replay of a candidate creation request.

The physical indexed receipt ID MUST distinguish a candidate's pinned view from inherited main rows with the same caller logical UUID, using the persisted store instance identity rather than a service generation or mutable path label. Checked promotion MUST preserve the original view identity of a candidate receipt when its row becomes reachable from main.

#### Scenario: Lost reply followed by sibling write
- **WHEN** one client loses the reply to an accepted append and another client commits a later append
- **THEN** a query by the first logical ID and view finds its matching committed receipt, both rows remain exactly once, and neither client automatically replays the first append.

#### Scenario: Scoped ID reused with different work
- **WHEN** a caller presents a retained store/view/logical-ID key with different method or arguments
- **THEN** Kuru reports an idempotency conflict and dispatches no mutation.

#### Scenario: Same UUID on another view
- **WHEN** a candidate inherits main's receipt rows and then accepts the same caller UUID on its own view, or a candidate receipt becomes reachable after promotion
- **THEN** the view-derived indexed keys remain distinct, each query proves only its exact original view's outcome, and neither inherited nor promoted rows are returned as the other view's match.

### Requirement: Bounded four-state outcome reconciliation

The service SHALL accept a typed outcome query for an exact logical mutation ID, original view and request fingerprint, returning in-flight, committed with a verified typed result, definitively absent, or still uncertain. It MUST NOT report absence as noncommit until the original accepted worker has ended, or a failed owner and its Dolt child have been verified reaped and the branch recovered. A missing exact candidate ref after explicit resolution/reclamation MUST NOT by itself prove that an earlier private write did not commit. An incomplete mutating reply SHALL fence further mutations through every handle of that logical client session until the exact outcome is reconciled; reconnect alone SHALL NOT clear uncertainty or authorize automatic replay. A complete authenticated rejection MAY leave the attachment usable.

#### Scenario: Owner crashes after commit before reply
- **WHEN** the owner commits a request, crashes before its reply reaches the client, and a successor opens the project
- **THEN** the successor waits for old owner/Dolt cleanup and reports the matching committed outcome from retained evidence without redispatching the request.

#### Scenario: Work remains in flight
- **WHEN** an outcome query races an accepted worker or unsettled SQL session
- **THEN** it reports in-flight or still uncertain, blocks subsequent mutation for the affected logical client, and does not infer noncommit from a missing row.

#### Scenario: Candidate receipt view was reclaimed
- **WHEN** an explicitly resolved candidate ref was reclaimed and its private receipt no longer appears on a reachable branch
- **THEN** a missing indexed row alone reports still uncertain rather than falsely asserting that its earlier accepted candidate write never committed.

#### Scenario: Cancelled write and sibling handle
- **WHEN** a client cancels a mutating RPC after dispatch but before a complete authenticated reply
- **THEN** its attachment is invalidated, sibling store/candidate/ledger handles cannot mutate, and a later authenticated outcome query uses the original logical ID rather than replaying the write.

### Requirement: Branch- and domain-specific result proof

Outcome reconciliation SHALL query the receipt on the branch that accepted the mutation and SHALL preserve its identity through checked promotion. Unit storage writes MAY resolve from their compact operation receipt. Usage-ledger operations MUST use their existing invocation-keyed durable records to confirm the exact admitted, observed or settled value. Candidate creation MUST bind its exact owned ref to a caller-retained logical UUID before dispatch. Begin is a single attempt: after an incomplete reply the supported client MUST NOT send Begin again, and a repeated read-only outcome/reattach query MUST inspect only that exact ref without creating a branch. A missing or resolved ref MUST NOT be silently recreated by recovery. Candidate creation, promotion and abandonment MUST reconcile exact owned refs and base/target revisions, preserving a conflict or uncertain candidate without silently promoting, deleting or manufacturing a usable handle from a retired service generation. Attachment loss MUST NOT abandon a candidate.

When an accepted candidate unit write loses its reply, the client MUST keep its mutation fence after proving the indexed unit receipt until a read-only query reattaches the same still-open candidate ref by its original creation identity. The typed recovery result MUST supply a fresh authenticated candidate handle for subsequent reads, writes and transition; generic Boolean reconciliation MUST NOT present the old attachment-local handle as usable. Recovery MUST NOT replay the write or recreate a missing or resolved candidate ref.

#### Scenario: Candidate unit reply is lost
- **WHEN** an accepted candidate write commits but its attachment loses the reply
- **THEN** the client proves its original view-bound unit receipt, returns a fresh checked handle for the exact retained candidate before clearing the mutation fence, and can read, write and explicitly promote through that handle without replaying the first write.

#### Scenario: Candidate begin reply is lost
- **WHEN** candidate creation is accepted but its reply or attachment is lost before the client receives a handle
- **THEN** the caller's retained UUID identifies only the original exact ref, which remains durable across owner restart until checked explicit resolution; repeated outcome queries do not create a second candidate, including after the exact ref is missing or resolved.

#### Scenario: Candidate promotion reply is lost
- **WHEN** the promotion reply is lost while another writer may have moved live main
- **THEN** Kuru compares the retained candidate's exact base, target and live refs, reports committed promotion or explicit preserved conflict only when proven, and returns a fresh authenticated candidate handle before releasing the client's mutation fence when that exact ref remains open; it never returns a stale generation's handle as current.

#### Scenario: A reclaimed no-op candidate lacks unique result proof
- **WHEN** a candidate's target equals its base and all exact status refs have been reclaimed before its lost transition reply is reconciled
- **THEN** Kuru reports uncertainty if the surviving revision history cannot distinguish the proposed transition from another resolution; it does not infer promotion or abandonment from missing refs alone.

#### Scenario: Usage settlement is retried
- **WHEN** a usage settlement with the same invocation identity is queried or explicitly retried after a lost reply
- **THEN** its durable natural-key record proves the exact outcome and no second charge is added.

### Requirement: Versioned writable-branch receipt compatibility

The current writable main and permanent usage branch SHALL expose and validate the retained receipt format before managed writes. Main migration SHALL use the registered exact-base, staged publication protocol; the permanent usage branch SHALL upgrade at a checked clean base before its next write. Pre-upgrade candidate branches MUST retain their names, heads, rows and historical schemas unchanged, remain inspectable, and be stale/nonpromotable against upgraded main. New writable candidates SHALL inherit the current receipt format. An older binary or an unmigrated writable branch MUST reject incompatible writes without deleting retained receipts.

#### Scenario: Upgrade with old usage and candidate branches
- **WHEN** current main upgrades while the permanent usage branch and a pre-upgrade candidate exist
- **THEN** usage is upgraded and validated before another ledger write, the old candidate is unchanged and inspectable but cannot promote, and a new candidate uses the retained receipt format.

#### Scenario: Old binary opens new receipts
- **WHEN** a binary that knows only the prior sole-receipt schema opens a store with retained current-schema receipts
- **THEN** it refuses the future schema before a writable operation and does not replace or discard those receipts.
