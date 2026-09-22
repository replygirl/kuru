## MODIFIED Requirements

### Requirement: Bounded operational storage maintenance

For historical schema versions 1 through 3, after reconciling a prior uncertain mutation each branch SHALL retain only the current mutation receipt in active state and MUST replace it in the same transaction as the next mutation. For the upgraded current writable schema, each accepted mutation SHALL retain compact indexed request-bound receipt evidence in the existing operations table; a later mutation MUST NOT erase that evidence. Historical validators MUST continue to read old branches according to their committed schema, and old candidate branches MUST remain unchanged. Candidate branches MUST be reclaimed only after an explicit promotion or abandonment is durably represented by an exact Kuru-owned branch-ref transition, all accepted candidate writes have settled, and every relevant SQL session has ended. Age, process IDs, handle drops, broad name-prefix matches and old unrecorded branches MUST NOT authorize candidate deletion. Startup MUST NOT finish a merely requested promotion; it MAY reclaim a promoting candidate only when its exact head is already reachable from live history and MUST otherwise preserve it for explicit resolution. Known dirty, mismatched or unresolved candidate refs that require no startup mutation MUST NOT prevent ordinary main use; changing or ambiguous identities, database observation failures and unsettled SQL sessions MUST still stop startup.

The owned server SHALL enable the pinned engine's bounded, growth-triggered automatic garbage collection and retain its diagnostics. GC MUST run inside the owned Dolt lifecycle, MUST preserve every referenced live, candidate, historical and export view, and MUST NOT be described as expiry or secure erasure.

#### Scenario: Current-schema receipts survive later writes
- **WHEN** repeated managed mutations commit on one current writable branch, including one whose acknowledgement is lost
- **THEN** each matching compact receipt remains indexed and queryable after later writes, so the lost result can be reconciled without duplicating its effect.

#### Scenario: Current receipt replaces reconciled receipt
- **WHEN** repeated mutations commit on a historical schema-1-through-3 branch, including one whose acknowledgement is lost
- **THEN** the lost result is reconciled before the next mutation, which atomically replaces the prior receipt while preserving both committed revisions; upgraded writable branches retain indexed receipts instead.

#### Scenario: Historical receipt remains historical
- **WHEN** an old branch retains its schema-1-through-3 sole receipt while current main uses the retained format
- **THEN** its old validator reads that branch without migration or a fabricated request fingerprint, and no old candidate is promoted across the changed live base.

#### Scenario: Candidate transition is partially observed
- **WHEN** process loss or a dropped reply leaves both an ordinary candidate ref and its exact status ref
- **THEN** Kuru compares both exact names and heads after SQL-session teardown, preserves every mismatched or uncertain state, and removes an exact duplicate only while retaining the durable status ref.

#### Scenario: Promotion was requested before process loss
- **WHEN** startup finds a valid promoting ref whose head has not reached live history
- **THEN** startup preserves the candidate without merging it or inferring abandonment.

#### Scenario: Preserved candidate is ineligible for cleanup
- **WHEN** startup finds a strict status identity whose dirty or mismatched state cannot be reclaimed without risking private history
- **THEN** it preserves every ref and permits ordinary main use only after proving that no candidate mutation or SQL session remains unsettled.

#### Scenario: Candidate is explicitly resolved
- **WHEN** promotion has made the candidate head reachable from live history or settled runtime failure explicitly abandons the candidate
- **THEN** Kuru reclaims only the exact resolved ref under owned no-live-view authority, while promoted main history and unrelated unresolved candidates remain readable.

#### Scenario: Pinned engine performs garbage collection
- **WHEN** the configured growth threshold schedules GC or an actual-engine fixture invokes the same pinned collector
- **THEN** one engine-owned collection uses session-aware safepoints, preserves referenced revisions and usable connections, and ends when the owned server is reaped.
