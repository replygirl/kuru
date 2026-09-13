# versioned-memory Specification

## Purpose
Preserve private project memory through managed Dolt ownership, atomic revisions,
isolated candidates and recoverable legacy imports.

## Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official native archive and runtime licenses in its executable. It MUST provision the matching engine locally without a compiler, separate installation or runtime download, including on a first offline launch. It MUST reject unsafe archives and corrupt executables before execution and SHALL NOT silently fall back to SQLite. Existing verified caches MAY be reused; corrupt existing caches MUST fail explicitly without destructive repair. An explicit observed open MAY report only fixed, bounded stage values for actual startup work; it MUST leave ordinary library opens silent, must not weaken any payload digest or exact-version probe, and MUST report ready only after final store validation and activation succeed.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using that verified cache; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.

#### Scenario: Observed startup
- **WHEN** an application requests an observed memory open
- **THEN** it receives fixed stages only where project ownership, managed cache verification or extraction, version probing, database preparation or opening actually begins, and receives ready only with a usable returned store.

### Requirement: Owned private server lifecycle

Kuru MUST authenticate loopback SQL from first readiness, isolate runtime config,
verify store identity, bound waits and retain ownership until its child is reaped.
Parent exit or crash SHALL release its server through a lifetime supervisor.
Unowned ports, PIDs and held locks MUST NOT authorize destructive takeover.
Writable opens SHALL own their server lifetime. Inspection MAY attach without
ownership. Directory activation and recovery MUST retain the lifecycle lease
through any move and revalidate directory and lock identity.

#### Scenario: Writer crash
- **WHEN** a writer is killed without cleanup
- **THEN** its supervisor observes lifetime EOF and reaps the owned Dolt child before another writer opens the store.

#### Scenario: Wrong endpoint
- **WHEN** readiness encounters wrong credentials, datadir or project identity
- **THEN** opening fails without adopting or terminating a foreign process.

#### Scenario: Inspection owns the server
- **WHEN** a writer opens while a cold inspection owns the server
- **THEN** it waits for ownership or fails within its startup deadline; inspection cleanup cannot stop a server borrowed by the writer.

#### Scenario: Interrupted initializer is still running
- **WHEN** recovery finds an otherwise valid stage with an active lifecycle owner
- **THEN** it waits or fails without moving the directory until the owner has reaped its child.

### Requirement: Isolated durable revisioned memory

Memory SHALL preserve byte-sensitive namespace/key identity, message order, opaque JSON values and existing validation. Mutations SHALL be atomic versioned batches with operation identity for uncertain-response reconciliation. Views MUST remain pinned to their branch, and candidate promotion MUST require its live base. A writable app may record one project-scoped first-run notice version only after the notice has been successfully presented; that state records presentation, not reading or consent. Missing or lower versions are pending, current or higher integer versions are settled, and malformed values MUST fail without a new mutation. Memory and conversation history SHALL NOT expire automatically.

#### Scenario: Transaction failure
- **WHEN** a batch fails after one row change
- **THEN** no partial batch is visible and prior history remains readable.

#### Scenario: Lost acknowledgement
- **WHEN** a committed operation loses its response
- **THEN** reconciliation identifies its committed result without duplicating the mutation.

#### Scenario: Candidate divergence
- **WHEN** live memory changes after a candidate captures its base
- **THEN** promotion fails without overwriting either history.

#### Scenario: First-run presentation is durable
- **WHEN** a writable project successfully presents the pending notice
- **THEN** one ordinary committed state revision records its version and a reopened store does not present that version again.

#### Scenario: Presentation fails
- **WHEN** stderr output or the first completed TUI draw fails before the notice is visible
- **THEN** the notice version remains pending and no provider or peer history receives notice text.

### Requirement: Preserved SQLite migration

Migration MUST validate the legacy store, preserve its original and a consistent snapshot including committed WAL data, and activate only a validated committed Dolt import. It SHALL preserve current-project rows, ordering and JSON exactly; other projects SHALL remain recoverable from the preserved source. Interrupted imports SHALL be recoverable without duplicate activation or source modification. When a legacy Unix data directory is not owner-private, Kuru MUST refuse before opening or importing it with an actionable remedy naming that directory; it MUST not silently change permissions. Windows guidance MUST use the native owner privacy contract rather than a Unix mode command.

#### Scenario: Existing conversations
- **WHEN** an existing project first opens with Dolt
- **THEN** sessions, preferences, topology, private histories and relationship memories remain available in the same order.

#### Scenario: Failed import
- **WHEN** validation or activation is interrupted
- **THEN** the original remains usable and no partial target becomes the active memory store.

#### Scenario: Unsafe legacy directory
- **WHEN** a Unix legacy SQLite layout is in a non-owner-private data directory
- **THEN** Kuru refuses before import and identifies the actual directory and its owner-only repair command.

### Requirement: Memory inspection

Kuru SHALL expose project memory status and revision history, and SHALL expose
provider-free selected-mode notes for one resolved part or relationship identity.
Each current note row MUST retain its stored signed sequence and role in the
projection. An explicit selected-note forget command MUST resolve the same
identity and exact `/notes` namespace, remove only the requested current row by
that namespace and sequence through an atomic versioned mutation, and preserve
unrelated notes, conversations, identities, and prior Dolt revisions. It MUST
refuse an absent store before legacy import or state creation and MUST disclose
that the operation does not erase historical revisions or other related text.
Kuru SHALL document managed runtime configuration, migration, offline use and
stopped-store backup/recovery.

#### Scenario: Selected current note is forgotten
- **WHEN** a user requests an existing selected identity's stored note sequence
- **THEN** the active `/notes` namespace omits only that row after one committed mutation and the result discloses retained history

#### Scenario: Unknown or non-note sequence is rejected
- **WHEN** a user requests a missing sequence or an identity whose current notes namespace does not contain it
- **THEN** Kuru reports the exact request failure without deleting a transcript, another identity's note, or any other current note

#### Scenario: Revision inspection
- **WHEN** the user requests memory history after a committed update
- **THEN** the command reports the durable revision without exposing credentials or unrelated projects.

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

### Requirement: Revision-pinned active memory export

Kuru SHALL expose a provider-free, read-only export of every application
`messages` and `state` row from one captured committed `main` revision. The
memory reader MUST reject a candidate view, capture the active revision once,
open and validate an exact commit-qualified pool, verify that pool resolves to
the captured revision and a supported schema, and retain that pool through all
pages. A concurrent writer MAY advance `main` after capture without changing
any page of the export. The export MUST exclude dirty working data,
candidate-only data, prior revisions, and operational or authority tables.

The reader SHALL preserve a signed message sequence, exact UTF-8 namespace,
role, content, exact state key, and the parsed JSON state value. It MUST page
messages in signed storage-key order and state in binary key order, use a
distinct first-page cursor rather than a numeric sentinel, release SQL
connections between bounded queries, and verify final emitted counts against
the captured committed counts. It MUST fail the complete export on unsupported
schema, invalid stored JSON or identifier, cursor/snapshot mismatch, query
failure, or count mismatch; it MUST NOT silently omit a record or dynamically
dump internal tables.

#### Scenario: Captured main stays coherent while it advances
- **WHEN** an export captures active `main`, a writer then advances `main`, and
  an unpromoted candidate or dirty working state exists
- **THEN** every export page and count comes from the captured committed hash,
  with no later, candidate, or dirty record included

#### Scenario: Signed and unknown application records survive pages
- **WHEN** stored messages include signed sequence boundaries and unknown
  namespaces while state includes unknown JSON fields and keys across page edges
- **THEN** each `messages` and `state` row appears exactly once in stable order
  with its unchanged storage identity and payload

#### Scenario: Export reader cannot cross snapshots
- **WHEN** an application reuses a page cursor with another captured snapshot
  or a later page/schema read fails
- **THEN** the read fails before a successful complete export is reported

### Requirement: Atomic durable turn checkpoints

Each admitted turn SHALL retain a versioned application-state journal value containing its original bounded ID, exact request identity, ordered lifecycle transitions, possible-dispatch state, and any authoritative completed `TurnOutput`. Admission MUST atomically write the started journal state with exactly one early user transcript row. Completion MUST atomically write the ended journal state, exactly one assistant transcript row, and the matching session, topology, and session-index values. Interrupted turns MUST retain their user transcript and journal history without fabricating an assistant message. These records SHALL use the existing message and opaque state schema and remain ordinary durable user content.

#### Scenario: Interrupted admitted prompt
- **WHEN** a turn is cancelled or fails after its admission checkpoint and before completion
- **THEN** its user prompt and started/interrupted journal history remain durable with no assistant transcript row.

#### Scenario: Atomic completed answer
- **WHEN** completion is accepted or its acknowledgement is lost
- **THEN** reconciliation observes either the complete assistant/session/journal checkpoint or its complete absence, without a partial checkpoint or duplicate transcript row.

#### Scenario: Safe pre-dispatch resume
- **WHEN** a matching started or interrupted ID has no possible-dispatch marker
- **THEN** the harness may resume it without appending the existing user transcript again.

### Requirement: Dispatch uncertainty precedes actor work

The journal possible-dispatch marker MUST become durable before work is sent to an actor mailbox or any provider, tool, cognitive, or A2A operation can begin. Recovery MUST treat the marker conservatively and MUST NOT claim whether a particular external call occurred.

#### Scenario: Loss around actor admission
- **WHEN** cancellation or process loss occurs after possible-dispatch persistence but before or during actor work
- **THEN** recovery surfaces an incomplete possibly dispatched turn and does not duplicate private actor history or external effects through automatic replay.

### Requirement: Bounded operational storage maintenance

After reconciling any prior uncertain mutation, each branch SHALL retain only
the current mutation receipt in active state and MUST replace that receipt in
the same transaction as the next mutation. Candidate branches MUST be reclaimed
only after an explicit promotion or abandonment is durably represented by an
exact Kuru-owned branch-ref transition, all accepted candidate writes have
settled, and every relevant SQL session has ended. Age, process IDs, handle
drops, broad name-prefix matches and old unrecorded branches MUST NOT authorize
candidate deletion. Startup MUST NOT finish a merely requested promotion; it
MAY reclaim a promoting candidate only when its exact head is already reachable
from live history and MUST otherwise preserve it for explicit resolution.
Known dirty, mismatched or unresolved candidate refs that require no startup
mutation MUST NOT prevent ordinary main use; changing or ambiguous identities,
database observation failures and unsettled SQL sessions MUST still stop startup.

The owned server SHALL enable the pinned engine's bounded, growth-triggered
automatic garbage collection and retain its diagnostics. GC MUST run inside the
owned Dolt lifecycle, MUST preserve every referenced live, candidate, historical
and export view, and MUST NOT be described as expiry or secure erasure.

#### Scenario: Current receipt replaces reconciled receipt

- **WHEN** repeated mutations commit, including one whose acknowledgement is lost
- **THEN** the lost result is reconciled before the next mutation, which atomically replaces the prior receipt while preserving both committed revisions.

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

### Requirement: Explicit managed-project purge

Kuru SHALL expose an explicitly confirmed, canonical-project-scoped purge that
removes the verified managed Dolt store and all of its managed revision history.
It MUST acquire and verify the command writer/startup/lifecycle authority before
publishing any destructive intent, MUST NOT provision an engine, import legacy
SQLite, construct a provider, or kill another owner, and MUST retain stable lock
objects. One checked control record MUST retain legacy-import suppression and any
incomplete original/quarantine identity inventory until removal is verified. An
ordinary open MUST reject incomplete purge authority; after completed purge it MAY
create an empty fresh store but MUST NOT automatically import the suppressed
project from legacy SQLite.

#### Scenario: Confirmed project purge
- **WHEN** the user confirms purge for a quiescent managed project
- **THEN** Kuru removes only that project's verified active and retained managed
  recovery trees, verifies their absence, and records completed legacy-import
  suppression without altering another project, shared legacy source, export,
  engine cache, or stable lock.

#### Scenario: Live owner or interrupted removal
- **WHEN** lifecycle authority cannot be acquired or removal is interrupted
- **THEN** no unverified path is removed, a live-owner refusal leaves no new
  purge record, and a retry uses the recorded identities rather than a
  replacement occupying an original pathname.

#### Scenario: Legacy reopen after purge
- **WHEN** a project with importable legacy SQLite is purged and later opened
- **THEN** the project opens as a fresh empty managed store without resurrecting
  the suppressed project's legacy rows while other project imports remain
  available.
