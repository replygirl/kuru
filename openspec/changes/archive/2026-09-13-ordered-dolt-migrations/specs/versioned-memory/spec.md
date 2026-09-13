## ADDED Requirements

### Requirement: Ordered compatible schema evolution

Kuru SHALL retain an immutable contiguous registry of released Dolt schema
migrations and SHALL apply every missing migration in order before returning a
writable current-schema store. Committed `kuru_schema.version`, the compiled
migration definitions and one immutable receipt per completed version SHALL be
validated together. A version-1 source MUST NOT already contain the reserved
migration receipt table. Schema version SHALL remain distinct from filesystem
activation, server identity and supervisor protocol formats. Read-only opens
MUST NOT migrate, and unknown future versions, gaps, changed definitions or
mismatched receipts MUST fail closed without schema, ref or working-set
mutation. Current-schema read-only inspection SHALL retain version-specific
schema and receipt validation and bounded historical-attempt checks, reject any
working change to either `kuru_schema` or `kuru_migrations` at every supported
version, and permit unrelated dirty data or DDL without modifying it.

#### Scenario: Persisted old schema opens writable

- **WHEN** a valid committed version-1 project is opened by a binary whose current schema is version 2
- **THEN** Kuru publishes exactly one validated version-2 migration commit and receipt before returning the store, and a second open adds no migration commit.

#### Scenario: Old schema opens read-only

- **WHEN** a read-only open finds a supported schema below the current version
- **THEN** it reports the found and required versions without creating a branch, receipt, marker or working-set change.

#### Scenario: Unsupported or inconsistent schema

- **WHEN** the stable version row is newer than the binary, skips a supported transition, or disagrees with a compiled receipt or postcondition
- **THEN** opening fails before further mutation and preserves the store for inspection.

#### Scenario: Read-only working data and authority

- **WHEN** an attached or stopped current-schema store contains ordinary dirty data or unrelated DDL, or instead contains a change to either schema-authority table or malformed reserved migration refs
- **THEN** ordinary working data remains inspectable, authority/ref inconsistencies are rejected, and both outcomes preserve the exact head, ref inventory and full working status without migration or publication.

### Requirement: Isolated schema construction and publication

Each migration step MUST create or recover a strictly named migration branch at
the exact clean active-main base. DDL, bounded data transformation, version
advance and receipt SHALL form one clean validated Dolt commit whose direct
parent is that base. Active `main` MUST remain at the complete prior version
until an exact-base, fast-forward-only publication succeeds. An acknowledged or
uncertain publication SHALL be accepted only after the original SQL session is
absent and independent reconciliation observes clean main at exactly the base
or validated target; every other result MUST fail closed without reset,
deletion or another mutation.

#### Scenario: DDL session disappears before commit

- **WHEN** migration DDL leaves a dirty working root on its isolated branch and the accepted session ends before `DOLT_COMMIT`
- **THEN** active main remains clean at its prior head and version, the dirty attempt remains reachable unchanged, and a fresh exact-base attempt may be constructed.

#### Scenario: Branch creation reply is lost

- **WHEN** the reply to creating a reserved exact-base migration branch is lost
- **THEN** Kuru reconciles the exact branch name and head before either reusing it or failing, without creating an ambiguous second owner.

#### Scenario: Completed attempt survives process loss

- **WHEN** startup discovers one clean unpublished migration branch with the exact base, target schema, receipt and postconditions
- **THEN** Kuru reuses that target for checked publication without replaying its DDL or adding another migration commit.

#### Scenario: Retained attempts outlive their migration step

- **WHEN** main already contains a migration receipt and later conversation commits while clean completed or recognized dirty failed branches for that applied step remain
- **THEN** Kuru classifies those branches as historical from their committed receipt, registered step and ancestry, requires a clean completed branch's sole parent to validate as that step's source schema, leaves every branch unchanged, and continues opening or evaluating the next ordered step.

#### Scenario: Fast-forward reply is lost

- **WHEN** the real fast-forward has durably installed the validated target but its reply is lost
- **THEN** Kuru waits for the original session to end, recognizes the exact target once, and does not replay migration or publication.

#### Scenario: Attempt inventory is ambiguous

- **WHEN** reserved attempt names are malformed or excessive, multiple publishable targets exist, or an attempt has an unexpected head, receipt, ancestry or dirty shape
- **THEN** migration fails without changing main or deleting, resetting or merging any attempt.

### Requirement: Owned migration lifecycle

Before `DOLT_BRANCH` or any other migration mutation, Kuru MUST transfer the
checked project startup lock, owned server, current main pool, plan and expected
base to one accepted migration worker. Caller cancellation SHALL NOT abandon
accepted work. The worker MUST own branch creation, mutation connections,
session teardown, outcome reconciliation, validation, pool closure and server
reaping, and MUST retain the startup lock through that boundary. It SHALL NOT
publish a `MemoryStore` to an abandoned caller. After process loss, the
supervisor lifecycle and the next cold open SHALL establish owner/session
absence before classifying durable attempts.

The startup lock MUST enter the owned server-startup path before any child
spawn or asynchronous startup suspension, so cancellation during server startup
retains the same authority through actual reaping.

#### Scenario: Server startup is cancelled

- **WHEN** a store opener is cancelled after its owned supervisor launches but before server startup returns
- **THEN** the retained startup lock follows the independent reaper and a competing opener cannot acquire writer authority while that supervisor still owns Dolt.

#### Scenario: Open is cancelled after acceptance

- **WHEN** the caller cancels after the worker accepts migration but before publication completes
- **THEN** the worker completes or reconciles the attempt, closes and reaps Dolt while retaining the startup lock, and no later opener observes an active session or intermediate main.

#### Scenario: Process exits during migration

- **WHEN** the migration process exits after creating or committing an attempt
- **THEN** the supervisor reaps the owned server before a later writer starts, and the later open classifies preserved branch state before further mutation.

#### Scenario: Existing inspection server owns the lifecycle

- **WHEN** a writable old-schema open encounters a live server owned for inspection
- **THEN** it waits or fails within the existing startup deadline without attaching as a writer, taking over the owner or beginning migration.

### Requirement: Current-schema staging and preserved failures

Fresh and legacy-import stores SHALL reach the same current schema and receipt
chain in their unpublished private staging directory before `ready.json` is
published. A failed or interrupted stage MUST remain unactivated, be stopped
before movement, and be preserved under the checked interrupted-stage protocol
with its identity, refs, working sets and imported history intact. Recovery
MUST NOT synthesize an activation record from SQL state, reuse a dirty stage or
move a live directory. Existing format-1 activation identity and original
legacy source/snapshot SHALL remain unchanged.

#### Scenario: Fresh or imported activation

- **WHEN** a new Dolt store is initialized directly or from a preserved SQLite snapshot
- **THEN** its owned live staging session validates the complete current schema and ordered receipts, publishes the existing activation record, and then stops and reaps the server before directory publication.

#### Scenario: Staged migration is interrupted

- **WHEN** migration fails before a staging store publishes `ready.json`
- **THEN** Kuru reaps its server and preserves the whole unactivated stage before constructing another, without modifying the source import or active project directory.

#### Scenario: Previous binary left a ready stage

- **WHEN** recovery finds an already-ready stage with a supported older schema, clean captured revision, valid receipt chain and only valid historical attempts through its recorded schema
- **THEN** it activates that accepted stage under the retained locks without changing its marker, then applies missing migrations through the ordinary active-main path; a newer-step attempt, dirty state or unknown schema fails before activation.

### Requirement: Version-aware historical branch preservation

Schema migration MUST leave every prior main revision and every existing
candidate name, head, row and branch-local schema unchanged. Internal
historical inspection SHALL read the stable branch schema version first and
select the retained validator and reader for that version without migrating the
branch or applying latest-schema validation. A pre-migration candidate SHALL
remain stale against migrated main and MUST NOT overwrite later history.
Historical inspection SHALL reject working changes to either `kuru_schema` or
`kuru_migrations` at every supported version while permitting unrelated dirty
data or DDL. It SHALL retain version-specific schema and receipt validation and
leave the inspected branch unchanged.

#### Scenario: Old candidate after main upgrade

- **WHEN** main upgrades from version 1 to version 2 while a candidate contains committed version-1-only history
- **THEN** the version-1 reader returns that history unchanged, the candidate ref is unchanged, promotion is refused as stale, and a new version-2 candidate can still promote.

#### Scenario: Later migration follows retained history

- **WHEN** main has later writes after version 2 and the binary adds the registered version-2-to-version-3 step while retained version-2 attempt branches still exist
- **THEN** Kuru recognizes the older attempts as historical, constructs version 3 from the exact current main head, and preserves every earlier ref and row.

#### Scenario: Historical dirty data does not hide dirty authority

- **WHEN** a supported historical branch has unrelated dirty data or DDL, or a malformed committed version-1 receipt table is hidden by a dirty drop
- **THEN** valid ordinary history remains readable, the dirty authority is rejected even when the live receipt table is absent, and the branch head, refs and full working status remain unchanged.

#### Scenario: Historical branch has a future schema

- **WHEN** internal inspection encounters a branch version unknown to the binary
- **THEN** it refuses that branch without routing it through a current-store reader or modifying any ref.
