# versioned-memory Specification

## Purpose
Preserve private project memory through managed Dolt ownership, atomic revisions,
isolated candidates and recoverable legacy imports.

## Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official native archive and runtime licenses in its executable. It MUST provision the matching engine locally without a compiler, separate installation or runtime download, including on a first offline launch. It MUST reject unsafe archives and corrupt executables before execution and SHALL NOT silently fall back to SQLite. Existing verified caches MAY be reused; warm opens MUST verify their complete pinned payloads concurrently without acquiring the exclusive installation lock, while corrupt existing caches MUST fail explicitly without destructive repair. Exact-version verification of a managed cached engine happens once, at install time, before a freshly extracted cache is activated; a warm open does not re-probe it. An explicitly configured `dolt_binary` is not a managed cache and remains version-probed on every open. This is a deliberate trade: a cached executable whose bytes match the pinned digest yet cannot run, or reports another version, is no longer detected by a warm open itself and surfaces instead when the database server fails to start; the install-time probe and the Dolt-version-keyed cache directory are the mitigations. Missing-cache installation and publication MUST remain serialized, including an under-lock destination recheck. An explicit observed open MAY report only fixed, bounded stage values for actual startup work, including a project-ownership wait only while another process holds that ownership, creation of a project store that has no active memory, a schema upgrade of an existing active store, and a client's start of its own memory service; it MUST leave ordinary library opens silent, and an open run by the memory service itself MUST stay silent to its caller and MAY publish its stages only as the owner's activity record, must not weaken any payload digest or the install-time exact-version probe, and MUST report ready only after final store validation and activation succeed.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine under the installation lock and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using the fully digested cache, whose exact version was proven when it was installed, without waiting for an unrelated installer lock; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.

#### Scenario: Observed startup
- **WHEN** an application requests an observed memory open
- **THEN** it receives fixed stages only where a contended project-ownership wait, managed cache verification or extraction, version probing of a freshly extracted engine or an explicit `dolt_binary`, database preparation, creation of a project with no active store, schema upgrade of an existing store, the client's start of its own memory service, or opening actually begins, and receives ready only with a usable returned store.

#### Scenario: Uncontended ownership
- **WHEN** an observed open acquires project ownership on its first attempt
- **THEN** it reports no project-ownership wait stage.

#### Scenario: Read-only open of a missing project
- **WHEN** a read-only open finds a project with no active store
- **THEN** it fails without reporting creation, and a read-only open of an older store fails without reporting an upgrade.

### Requirement: Owned private server lifecycle

Kuru MUST authenticate loopback SQL from first readiness, isolate runtime config, verify store identity, bound waits and retain ownership until its child is reaped. The per-project memory service SHALL own the writable server lifetime independently of any conversation client. Service exit or crash SHALL release its server through the existing lifetime supervisor; one client's exit or crash SHALL NOT release the server while another client or accepted operation remains. Unowned ports, PIDs and held locks MUST NOT authorize destructive takeover. Writable client opens SHALL attach to the validated service and MUST NOT independently start a second writer server. Inspection MAY attach without ownership. Directory activation and recovery MUST retain the lifecycle lease through any move and revalidate directory and lock identity.

#### Scenario: Writer crash
- **WHEN** a conversation client is killed without cleanup
- **THEN** the service removes that client's attachment and continues to own its Dolt child for other clients

#### Scenario: Service crash
- **WHEN** the service itself is killed without cleanup
- **THEN** its supervisor observes lifetime EOF and reaps the owned Dolt child before another service opens the store

#### Scenario: Wrong endpoint
- **WHEN** readiness encounters wrong credentials, datadir or project identity
- **THEN** opening fails without adopting or terminating a foreign process

#### Scenario: Inspection owns the server
- **WHEN** a writable client opens while a cold inspection owns the server
- **THEN** it waits for ownership or fails within its startup deadline; inspection cleanup cannot stop a server borrowed by the writer

#### Scenario: Interrupted initializer is still running
- **WHEN** recovery finds an otherwise valid stage with an active lifecycle owner
- **THEN** it waits or fails without moving the directory until the owner has reaped its child

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

Migration MUST validate the legacy store, preserve its original and a consistent snapshot including committed WAL data, and activate only a validated committed Dolt import. It SHALL preserve current-project rows, ordering and JSON exactly; other projects SHALL remain recoverable from the preserved source. Interrupted imports SHALL be recoverable without duplicate activation or source modification. When any Unix data directory is rejected as non-owner-private, Kuru MUST give an actionable mode-0700 remedy naming that directory only if it is a real directory owned by the current user with group or other permission bits; the remedy applies with or without legacy SQLite. Kuru MUST NOT silently change permissions or advise that remedy for a link or foreign-owned path. Windows guidance MUST use the native owner privacy contract rather than a Unix mode command.

#### Scenario: Existing conversations
- **WHEN** an existing project first opens with Dolt
- **THEN** sessions, preferences, topology, private histories and relationship memories remain available in the same order.

#### Scenario: Failed import
- **WHEN** validation or activation is interrupted
- **THEN** the original remains usable and no partial target becomes the active memory store.

#### Scenario: Unsafe legacy directory
- **WHEN** a legacy SQLite open rejects a real current-user-owned Unix data directory whose mode grants group or other permissions
- **THEN** Kuru refuses before provisioning or import, identifies the exact directory and mode-0700 remedy, and leaves its mode and contents unchanged.

#### Scenario: Unsafe ordinary directory
- **WHEN** an ordinary open rejects a real current-user-owned Unix data directory whose mode grants group or other permissions
- **THEN** Kuru refuses before provisioning, identifies the exact directory and mode-0700 remedy, and leaves its mode and contents unchanged.

#### Scenario: Unsafe unowned or linked directory
- **WHEN** a Unix data path is a link or is not owned by the current user
- **THEN** Kuru refuses without suggesting that changing mode alone would make the path trusted.

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

A staging directory MAY instead reach the current schema through an adopted
copy of a machine's store template rather than the migration chain. Recovery
MUST classify such a stage before any class that would start an engine, and
MUST NOT start an engine on one: a copy interrupted before its identity marker
was written, and a copy whose identity marker is present but which never
published `ready.json`, whatever point its adoption reached, are both
preserved under the interrupted-stage protocol without a server ever starting
against them. A template copy that published `ready.json` is recovered as any
ready stage is. A stage without an identity record holding anything outside
the copy remnant's allowed set remains unrecognized and fails before any
engine start.

#### Scenario: Fresh or imported activation

- **WHEN** a new Dolt store is initialized directly or from a preserved SQLite snapshot
- **THEN** its owned live staging session validates the complete current schema and ordered receipts, publishes the existing activation record, and then stops and reaps the server before directory publication.

#### Scenario: Staged migration is interrupted

- **WHEN** migration fails before a staging store publishes `ready.json`
- **THEN** Kuru reaps its server and preserves the whole unactivated stage before constructing another, without modifying the source import or active project directory.

#### Scenario: Previous binary left a ready stage

- **WHEN** recovery finds an already-ready stage with a supported older schema, clean captured revision, valid receipt chain and only valid historical attempts through its recorded schema
- **THEN** it activates that accepted stage under the retained locks without changing its marker, then applies missing migrations through the ordinary active-main path; a newer-step attempt, dirty state or unknown schema fails before activation.

#### Scenario: Copy remnant is preserved without starting an engine

- **WHEN** recovery finds a staging directory with no `identity.json`, a `data/` directory, and no top-level entry outside the allowed set of `data/`, a `staging/` directory holding only temporary record files, and a Unix `lifecycle.lock`
- **THEN** Kuru waits for quiescence within the existing bound and preserves the whole stage under the interrupted-stage protocol without starting an engine against it.

#### Scenario: Unready template copy is preserved without starting an engine

- **WHEN** recovery finds a staging directory whose `identity.json` names a source template and no `ready.json` exists, whatever point the prior adoption attempt reached
- **THEN** Kuru waits for quiescence within the existing bound and preserves the whole stage under the interrupted-stage protocol without starting an engine against it, and never runs adoption SQL as part of recovery.

#### Scenario: Ready template copy is reused

- **WHEN** recovery finds a template copy that completed adoption, validation and its template shape check and published `ready.json`
- **THEN** it is inspected and activated through the existing ready-stage path at its recorded initial revision, without adoption running again.

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

Each admitted turn SHALL retain a versioned application-state journal value
containing its original bounded ID, exact request identity, ordered lifecycle
transitions, possible-dispatch state, and any authoritative completed
`TurnOutput`. A new local CLI/TUI submission MUST atomically write the started
journal state, exactly one early user transcript row and the session-scoped
single last-submission tuple. Completion MUST atomically write the ended journal
state, exactly one assistant transcript row, and the matching session, topology,
and session-index values. Interrupted turns MUST atomically retain their user
transcript and journal history with exactly one fixed interruption transcript
marker and no fabricated assistant message. A completed-answer reconciliation
MUST add no marker. These records SHALL use the existing message and opaque state
schema and remain ordinary durable user content.

Turn-journal rows SHALL be durable no-expiry safety and idempotency history.
Their aggregate storage grows in proportion to admitted turns; completed result
data remains retained for indefinite exact retry even though the response event
does not duplicate the answer body. Kuru MUST NOT cap, expire, fold into
transcript metadata, rewrite or automatically delete these rows.

#### Scenario: Interrupted admitted prompt
- **WHEN** a turn is cancelled or fails after its admission checkpoint and before completion
- **THEN** its user prompt, one interruption marker and started/interrupted journal history remain durable with no assistant transcript row.

#### Scenario: Atomic completed answer
- **WHEN** completion is accepted or its acknowledgement is lost
- **THEN** reconciliation observes either the complete assistant/session/journal checkpoint or its complete absence, without a marker, partial checkpoint or duplicate transcript row.

#### Scenario: Safe pre-dispatch resume
- **WHEN** a matching started or interrupted ID has no possible-dispatch marker
- **THEN** the harness may resume it without appending the existing user transcript or replacing the last-submission tuple.

#### Scenario: Retained completed history
- **WHEN** many turns complete and later exact retries are requested
- **THEN** each session-scoped journal row and completed output remains available without expiry or aggregate-cap deletion.

### Requirement: Dispatch uncertainty precedes actor work

The journal possible-dispatch marker MUST become durable before work is sent to an actor mailbox or any provider, tool, cognitive, or A2A operation can begin. Recovery MUST treat the marker conservatively and MUST NOT claim whether a particular external call occurred.

#### Scenario: Loss around actor admission
- **WHEN** cancellation or process loss occurs after possible-dispatch persistence but before or during actor work
- **THEN** recovery surfaces an incomplete possibly dispatched turn and does not duplicate private actor history or external effects through automatic replay.

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

### Requirement: Discriminated typed message persistence

Memory SHALL distinguish legacy text rows from typed block payloads using an
explicit durable content-format discriminator introduced by an ordered migration.
Legacy content bytes, sequence, namespace, role, revisions and retained candidates
MUST remain intact. Readers MUST select the codec and available columns according
to the viewed schema, including supported historical and candidate schemas. An
unknown format or malformed typed payload MUST fail explicitly without a text
fallback. Typed writes to an old-schema view MUST be refused, while supported
legacy text operations remain compatible. Typed message checkpoint publication
and journal/state publication MUST remain one reconciled operation.

#### Scenario: Upgrade with JSON-looking legacy content

- **WHEN** a released old-schema store containing JSON-looking message strings upgrades and reopens
- **THEN** each old row remains exact text, new typed rows round trip as blocks, and retained old-schema revisions/candidates remain readable.

#### Scenario: Interrupted publication

- **WHEN** typed checkpoint publication has an uncertain reply or caller cancellation
- **THEN** existing receipt reconciliation determines its outcome without duplicate messages or a split message/state commit.

#### Scenario: Unsupported stored format

- **WHEN** a reader encounters an unknown content format or malformed typed payload
- **THEN** it reports a bounded storage error rather than partial or silently flattened history.

### Requirement: Typed export and legacy import compatibility

Revision-pinned exports SHALL carry the message content-format discriminator and
exact stored content, with an explicit export-format version that prevents typed
JSON being mistaken for legacy text. Markdown SHALL describe structured blocks
truthfully. Legacy SQLite import MUST retain source bytes and import messages as
legacy text; this change MUST NOT infer typed semantics from string contents or
change explicit forgetting, purge or no-expiry guarantees.

#### Scenario: Mixed-format export

- **WHEN** a committed store containing legacy text and typed messages is exported as JSON or Markdown
- **THEN** content, format, provenance and counts are preserved, and structured content is distinguishable from literal legacy prose.

#### Scenario: Read-only legacy import

- **WHEN** an existing supported SQLite fixture is imported into the new schema
- **THEN** source bytes remain unchanged and imported strings retain their original text interpretation.

### Requirement: Session provenance schema preservation

The current Dolt schema SHALL retain nullable session provenance on raw message rows and strict context-summary records and cursors. Migration, candidates, full-memory export and recovery MUST preserve the exact optional session identity, global sequence, summary provenance and cursor state; they MUST NOT derive a session from a namespace, current process or most recent conversation.

#### Scenario: Current schema upgrade preserves unattributed history
- **WHEN** a released schema store containing typed and legacy private rows upgrades
- **THEN** existing rows retain their exact sequences and payloads with null session identity, new attributed rows validate against the current schema, and historical candidate views continue to use their committed schema.

#### Scenario: Full-memory export
- **WHEN** an owner exports a committed revision containing attributed rows, unattributed rows, summaries and cursors
- **THEN** every optional session identity and provenance coordinate is represented exactly without moving a legacy row into session continuity.

### Requirement: Bounded session cursor history projection

Memory SHALL expose a read-only newest-history suffix for one exact namespace and session strictly after an exclusive durable sequence. One checked result MUST carry the pinned live or candidate view, captured revision, requested cursor, exact count of all eligible rows and the newest complete sequenced rows that fit the requested limit and shared session-source byte bound. The result MUST preserve ascending durable order, MUST exclude rows at or before the cursor and rows from every other namespace, session or unattributed legacy history, and MUST behave identically through local and managed views without paging, export access or mutation authority.

#### Scenario: More eligible rows than one source page

- **WHEN** one session has more than 1,024 raw rows after its summary cursor and sibling namespaces and sessions have interleaved global sequences
- **THEN** a bounded request returns only that session's newest requested sequenced suffix in ascending order and reports the exact count of all eligible rows.

#### Scenario: Exact cursor has no later rows

- **WHEN** the cursor equals the newest eligible sequence or a zero-limit count is requested
- **THEN** the projection returns no rows, retains the exact view, revision and cursor, and reports the exact eligible-row count without treating omitted rows as absent.

#### Scenario: Candidate history stays isolated

- **WHEN** a candidate appends session rows after its captured base while live memory advances independently
- **THEN** the candidate projection returns only its pinned candidate suffix and revision while the live projection excludes unpromoted candidate rows.

### Requirement: Truthful versioned context provenance

The main memory schema MUST advance through a forward v6 migration that allows a context summary and its private reasoning records to carry exactly one real provenance kind: a conversation turn or an internal operation. An operation-attributed context record MUST identify its producing actor and invocation. Existing turn-attributed rows, serialized values, deterministic IDs and private state keys MUST retain their exact pre-v6 representation and identity, and the independently versioned usage registry MUST remain at v4.

#### Scenario: Existing turn records survive the forward migration

- **WHEN** a v5 store containing context summaries and private reasoning records is upgraded to v6
- **THEN** every historical row remains readable and exportable with its original turn attribution, serialized record format, deterministic ID and private state key, without an inferred operation or producer actor.

#### Scenario: Internal compaction records use operation provenance

- **WHEN** an accepted internal compaction checkpoints a context summary and private reasoning sidecar
- **THEN** the context record and every sidecar record carry the same real session, producer actor, invocation and operation, carry no fabricated turn, and use domain-separated operation identities.

### Requirement: Atomic context summary and private sidecar checkpoint

The memory owner MUST validate and publish an accepted context summary, its cursor advance and its bounded private reasoning sidecar in one revision-checked SQL transaction under one logical receipt. Any stale revision or cursor, invalid binding, identity conflict, cancellation before dispatch or rejected input MUST leave all three effects absent. An accepted reply loss MUST reconcile the whole checkpoint without redispatching provider work or publishing a prefix.

#### Scenario: Stale checkpoint has no side effects

- **WHEN** a checkpoint carries a stale source revision, moved cursor or source range that no longer matches its pinned view
- **THEN** the owner returns the typed definite stale result and writes no context summary, cursor or private reasoning key.

#### Scenario: Sidecar conflict has no prefix

- **WHEN** any operation-attributed sidecar key already exists with a different settled payload
- **THEN** the owner returns the typed definite reasoning-summary conflict and the transaction publishes none of the checkpoint, cursor or sidecar records.

#### Scenario: Accepted lost reply reconciles one atomic outcome

- **WHEN** the owner accepts and commits the combined checkpoint but the client loses its reply
- **THEN** the existing logical receipt proves the entire transaction committed, an exact retry is idempotent, and no second provider invocation or partial sidecar write occurs.

### Requirement: Combined checkpoint request bounds

The facade MUST validate the complete serialized checkpoint envelope before selecting or acquiring a local or remote mutating backend, and the owner MUST repeat the same validation before mutation. The complete typed payload MUST be limited to 64 MiB, remain below the managed 100 MiB frame after fixed RPC wrapping and reject any one-byte excess; component limits alone MUST NOT authorize a larger combined request.

#### Scenario: Worst-case escaping fits the managed frame

- **WHEN** the maximum accepted record count and aggregate payload use JSON control characters with worst-case escaping
- **THEN** the complete serialized checkpoint request remains within 64 MiB and below the managed frame limit.

#### Scenario: Invalid or oversized request never attaches mutably

- **WHEN** a checkpoint violates provenance binding, record invariants, batch count or the complete serialized size by one byte
- **THEN** the facade returns a definite validation error before backend selection or mutating attachment, and owner-side direct callers receive the same no-effect refusal.

### Requirement: Template-born stage identity and adoption

A staging directory copied from a machine's store template SHALL carry an
optional `template` field in its activation identity record naming the
template it was copied from. A template key is one portable path component of
lowercase ASCII letters, digits, `_` and `-`, at most 128 bytes; an identity
record naming any other key is invalid. A store that was not copied from a template
SHALL serialize its identity record with no such field present, byte-for-byte
as before this requirement existed, and the identity record format SHALL
continue to reject an unrecognized field so that a binary predating this
requirement fails closed on a template-born store's identity record instead of
misreading it.

While a staging directory's identity record is not yet marked initialized and
names a source template, Kuru MUST, before any other bootstrap mutation,
compare the named template against the supervisor's own compiled template key
and refuse to proceed if they differ, without treating that refusal as a
judgement on the template's bytes. Once initialized, a template-born store's
identity record MUST NOT be compared against any compiled key again, so that a
store already adopted from a template continues to open under a later
compiled key.

Adoption from a verified template copy MUST happen at most once, only from a
staging directory whose identity record is not yet marked initialized and
names a source template, and MUST NOT be attempted again once that mark is
set or from any recovery path. Adoption MUST verify that the placeholder
instance and project-scope row the template publishes is present, unchanged
and the only row, and that the working set is clean, on both the copy's usage
branch and its main branch before rewriting either one, MUST rewrite that row
to the copy's own new instance and project scope and commit the change on the
usage branch and then on main, each as its own Dolt commit, and MUST verify
the rewritten row on both branches before the identity record is marked
initialized. A copy's credentials MUST be newly generated and MUST NOT be
usable against any other copy of the same template or against the template
itself.

Before a template copy publishes `ready.json`, its own engine MUST validate
the store and check the template shape with the adopted identity: the exact
branch set of `main`, the usage branch and one clean retained migration
branch per executed schema step; both refs at their current schema versions
with clean working sets; commit counts on both refs derived from the compiled
migration registries plus one adoption commit each; the adopted identity as
the only identity row on both refs; no rows in any project-data table; and no
views, triggers, routines, stored schema objects, stored procedures or ignore
rules. A branch or table list that holds more entries than the check reads is
a verdict, never a silently truncated set. The same check, with the
placeholder identity and no adoption commits, SHALL be the one a template
build runs. A completed shape query that returns another value is a verdict
against the template's bytes; a query error or deadline is not. The shape
check classifies each retained `main` migration branch from `main`; retained
usage-branch migration branches are only counted there, and are classified by
the usage-branch validation that a template build runs and that every
writable open runs when it establishes the usage ledger. The compiled usage
registry retains no such branch while it ends at the schema a usage branch is
anchored at; a usage schema step beyond it MUST first make the template build
and a copy's first engine classify those branches before `ready.json`.

A completed comparison that finds the row, the working set, the count of
rows a rewrite affected, or the template shape to be something other than
what adoption expects is a verdict against the template's bytes, and Kuru
MUST report it through a response type distinct from an ordinary failure and
from a client-side startup error, which the opening client MUST surface as
the same typed verdict. An
engine failure, a lost or malformed reply, a deadline, an authentication
failure, a SQL error unrelated to the expected row or count, or a mismatched
compiled template key MUST NOT be reported through that verdict response.
A copy whose `kuru` database or usage branch is missing fails adoption with a
SQL error, so it is an ordinary failure rather than a verdict; the template
cache's structural check before any stage is copied, not adoption, MUST be
what detects that corruption of a template.

Every adoption failure, verdict or not, fails the copy's engine start. That
failure, or a failure to open the started engine's main pool, stops and reaps
the engine and leaves the unready stage in place, as a failed first staging
start is left today, and the next open's recovery
preserves it under the interrupted-stage protocol without starting an engine
against it. A failure after the engine has started and adopted the copy
(validation, the template shape check or publication of `ready.json`) is
preserved by the staging job itself, as any failed validation of a stage is.
In neither case is adoption retried.

Stores copied from the same machine's template for the same template key
SHALL share that template's pre-adoption history with identical commit
hashes: the schema-initialization commit, which holds only the fixed
placeholder identity, and the schema migration commits and their retained
branch heads. Those shared commits MUST contain only schema, migration
receipts and the placeholder identity, never project data, another project's
identity or any credential. Each copy's own history SHALL begin at its
adoption commits on main and on the usage branch, which record its own
instance and project scope, and its activation record's initial revision
SHALL be that main adoption commit.

#### Scenario: Cold store identity record is unchanged

- **WHEN** a store is created directly or by legacy import, without any source template
- **THEN** its identity record serializes with no template field present, byte-identical to the record a store without this requirement would have produced.

#### Scenario: Adoption gives a copy its own identity

- **WHEN** two staging directories are independently copied from the same verified template and each completes adoption
- **THEN** each ends with a distinct instance identifier, distinct project scope, distinct credentials and its own initial revision, neither copy's reader credential is accepted by the other's server, and both copies' pre-adoption commit hashes are identical to each other and to the template's.

#### Scenario: A verdict against the template's bytes is typed and distinct

- **WHEN** the placeholder row adoption expects is not present unchanged on either branch, either working set is dirty, or a rewrite affects a count of rows other than the one expected
- **THEN** Kuru reports the distinct verdict response before either branch is rewritten where the comparison precedes the rewrites, the unready staging directory is left in place and preserved by the next open's recovery without an engine start, and adoption is never retried.

#### Scenario: A template shape violation prevents the ready marker

- **WHEN** an adopted copy holds an extra commit, an extra branch, a project-data row, a view or more tables than the check reads
- **THEN** its engine reports the typed verdict before `ready.json`, the staging job preserves the unready stage under the interrupted-stage protocol, and no active project directory appears.

#### Scenario: An engine failure during adoption is not a verdict

- **WHEN** adoption meets an engine crash, a lost commit reply, a bootstrap deadline, an authentication failure, a missing `kuru` database or usage branch, or a mismatch between the staging directory's named template and the supervisor's own compiled template key
- **THEN** Kuru reports an ordinary failure, not the distinct verdict response, the unready staging directory is left in place, and the next open's recovery preserves it under the interrupted-stage protocol without starting an engine against it.

#### Scenario: An initialized template-born store opens under a later compiled key

- **WHEN** a store already adopted from a template is opened by a supervisor whose compiled template key differs from the one the store's identity record names
- **THEN** the open proceeds without any key comparison or adoption attempt, because the comparison applies only while the store's identity record is not yet marked initialized.

### Requirement: Per-machine store template cache

A machine's engine cache MAY hold, under its current engine version
directory, a private `templates/` area keyed by content: a permanent lock
file per key, a published template directory per key holding only a manifest
and a captured `data/` tree, and transient build, capture-stage and
quarantine directories distinguished from a published template by name. A
published template directory MUST hold only its manifest and `data/`, and its
`data/` MUST hold only schema, migration receipts and a fixed placeholder
identity: no `identity.json`, `config/`, `home/` or engine secret, so the same
published template MAY be copied by every project that shares the cache, and
an explicit project purge MUST NOT remove it.

The key naming a published template MUST be computed, once per process, from
exactly the inputs that determine the bytes a fresh chain-built store would
hold: a fixed format constant, the pinned engine version, the pinned engine
executable's verified digest, the target triple, the current schema version,
the current usage-schema version, the digest of every compiled migration and
usage-migration registry definition in declared order, and the creation-time
SQL statements, placeholder literals, commit-message formats and supervisor
configuration behavior that shape an empty store's bytes. The key MUST NOT
depend on the supervisor executable's own bytes or on any source file's
digest, so an instrumented and an ordinary supervisor build interchangeable
templates for the same key. A template built under one key MUST NOT be read,
copied or treated as valid under a different key.

A template MUST be built only under its key's own exclusive lock, by running
the real schema-migration chain and the real usage-branch upgrade against a
fixed placeholder identity on one engine, then validating that engine's
active schema and the same shape assertion a copied stage validates after
adoption, parameterized with the placeholder identity in place of an adopted
one. That shape assertion MUST read every commit on the main branch and on the
usage branch and compare each commit's committer, committer email, author,
author email and message, and the history's length, with the history derived
from the compiled registries: the engine's own first commit under its fixed
system account, then commits under Kuru's fixed author with the
initialization message, one upgrade message per executed step naming its
retained attempt's operation, and the adoption message once adopted. A
completed history read that differs is a verdict against the bytes. A build
MUST refuse to publish, and MUST leave any previously published template for
its key untouched, unless every one of those assertions passes.
Before publication, the captured `data/` tree MUST be scanned for occurrences
of the build directory's own absolute path, either engine secret used during
the build, or the local host name, and publication MUST be refused if any
occurrence is found. The scan finds literal occurrences only, because the
engine stores its data compressed, and it MUST skip a host name shorter than
four bytes, which occurs by chance in compressed storage and would refuse
sound builds. The shape assertion above, which reads the database itself, is
the guarantee that a template holds no project data, recorded host or user
identity or credential; the byte scan is an additional precaution. Publication MUST be a rename that does not replace an
existing published directory for the same key, verifying the moved
directory's identity after the rename; on a publication failure the build's
own verified, unpublished stage MAY still be copied by its own caller, and
MUST be left in place for the next exclusive-lock holder to resolve. A holder
of a key's exclusive lock that builds or quarantines MAY remove that same
key's own abandoned build and capture-stage directories, without waiting for
an engine that has not yet stopped: an entry whose engine lease is still held,
or whose removal fails, MUST be left for a later holder.

A verified copy of a published template MUST be obtained only under that
key's shared lock, and MUST independently verify, while copying, that the
manifest parses to the expected format and key, that the top-level entries
present are exactly the manifest's declared entries, and that every copied
file matches the manifest's declared size and SHA-256 digest; a symbolic link,
an extra hard link, or an entry of a different type than the manifest declares
MUST cause the copy to fail rather than succeed silently. No part of this
verification depends on the copying process's own trust of the manifest's
origin: the manifest only names what to check, and the checks that decide
pass or fail run only against the compiled key, the compiled placeholder
identity and the shape assertion described above.

A published template MUST be moved aside into a quarantine location, and
never read as valid again under its key, only when a completed comparison
against its own bytes returns a value other than the one expected: the
manifest's declared format or key, the per-file verification above, the
placeholder identity row checked during adoption, or the shape assertion.
Every other failure reachable while building, publishing or copying a
template — an engine crash, a lost or malformed reply, any deadline, an
authentication failure, a lock acquisition that would block, a lock or
manifest read error, or an I/O error during the copy — MUST leave every
existing published template, for every key, untouched, and MUST be treated as
ordinary failure rather than as a verdict against the template's bytes. The
quarantine action MUST be bound to the identity of the specific template
directory that was judged: it MUST re-check that the directory holding the
published name still has the judged identity under the key's own exclusive
lock before moving it aside, and MUST skip the quarantine, with a recorded
warning and no error raised to the caller, if the identity no longer matches,
the exclusive lock cannot be acquired without waiting, or the move fails. At
most one quarantined directory per key is kept; an older one MUST be removed,
best-effort, before a new one is quarantined.

Code that creates or opens a project MUST NOT wait for another process's
template build or quarantine, MUST NOT remove another key's published template
or quarantined directory, and MUST NOT remove any template lock file: lock
files for this cache are permanent for the life of the cache directory. Any error
encountered opening the `templates/` area, acquiring or verifying a key's own
lock, or reading its manifest, MUST be treated the same as "no usable
template for this key": the caller proceeds without one, not as an open
failure, except in test-fixture warm-up, where the same errors MUST be
reported rather than silently treated as absence, because fixture warm-up has
no caller to send down a fallback path.

#### Scenario: A template is shared by every project on the machine

- **WHEN** two different projects, on the same machine and engine cache, each
  have a compiled template key that matches a currently published template
- **THEN** each project's creation is free to copy that same published
  `data/` tree, and copying it for one project changes nothing a later copy
  for a different project reads.

#### Scenario: A template build refuses to publish on a failed assertion

- **WHEN** a template build's own validation, shape check or byte scan fails
  before publication
- **THEN** no new directory is published under that key, any previously
  published template for that key is left exactly as it was, and the next
  attempt to use that key starts a fresh build under the same exclusive lock.

#### Scenario: Concurrent new projects never wait on a template build

- **WHEN** one process holds a key's exclusive lock to build or publish a
  template and a second process, for a different project, needs a template
  under the same key at the same time
- **THEN** the second process's attempt to acquire the key's lock does not
  block it: it proceeds immediately as if no template were available for
  that key, without reading any unpublished build or capture-stage directory.

#### Scenario: A verdict against the bytes quarantines; an engine failure does not

- **WHEN** a copy's per-file verification finds a digest or size mismatch
  against the manifest, or the manifest's declared format or key is wrong
- **THEN** the published template for that key is quarantined, identity-bound
  to the directory that was judged, and never read as valid again under that
  key; but when the same copy instead meets a lock contention, an I/O error,
  or any other failure that is not a completed comparison against the
  template's own bytes, the published template is left untouched and the
  failure is treated as ordinary.

#### Scenario: No open path garbage-collects another key or any lock file

- **WHEN** a process builds or copies a template for one key, in a cache
  whose `templates/` area also holds a published template, a quarantined
  directory and a lock file for at least one other key
- **THEN** every other key's published template, quarantined directory and
  lock file remain byte-identical and present afterward, and no lock file is
  ever removed by this process.

#### Scenario: A classifier refusal is engine-side evidence, not a verdict

- **WHEN** the shared template-shape check or the main-pool classification
  it depends on refuses to classify a retained migration branch found on a
  copy or on the build engine
- **THEN** that refusal is reported as an ordinary engine-side failure and
  MUST NOT, by itself, be treated as a verdict against a template's bytes.

#### Scenario: An older client fails closed on an unrecognized rejection

- **WHEN** a supervisor reports the typed template-rejection response and the
  connected client's build predates that response variant
- **THEN** the client fails closed with a parse error on the unrecognized
  reply rather than treating it as success or silently discarding it.

### Requirement: Store creation path selection

When a writable open finds the active store absent and recovery has found no
completed stage to reuse, Kuru MUST choose how to create the store before
starting any engine for it, and MUST NOT run more than one creation attempt
from the template in that open.

Kuru MUST take the cold staged build (initialization, an optional legacy
import, every schema step, validation and `ready.json` in a private staging
directory), never reading, building or copying a template, when a legacy
import is present, when the open is configured with an explicit engine binary,
or when a test fixture explicitly requests the cold path. For every other new
store Kuru MUST create the store from the per-machine store template of its
engine cache and compiled key:

- When a template is published for that key, Kuru MUST copy it into a new
  staging directory under the key's shared lock, write the stage's identity
  record last, and run the stage's one engine start, which adopts the copy,
  validates it, checks the template shape and publishes `ready.json`, before
  the unchanged quiescence, move onto the active path and active start: two
  engine starts in all, with no schema step run for this project.
- When no template is published for that key and the key's exclusive lock is
  free without waiting, Kuru MUST first build and publish the template on one
  engine, running the schema chain once, and then copy this project's stage
  from the template it built, or from the build's verified stage when
  publication failed, before the same adoption start and active start: three
  engine starts in all, and the schema chain runs once for the machine and
  key.
- The cold staged build keeps its own engine starts.

A creation worker MUST perform the copy, the build and the stage's engine
start. It MUST own the project's startup lock for that work, and the
template key's lock while a copy or build runs (the build's exclusive lock as
the build engine's reap guard, and every copy's lock held by the thread that
writes it), in the ownership shape of the migration worker: the opening frame
MUST NOT hold either lock while that work is in flight, a cancelled open MUST
leave the worker to finish, and the startup lock MUST return only after every
engine the worker started has been reaped. A later opener of the same project
waits for the startup lock within its own startup deadline, then reuses the
stage the worker left ready or finds it preserved.

Kuru MUST create the store cold instead, in a new staging directory and
without surfacing an error, when the template cannot be used now: the key lock
is held by another process, the template root or the key's lock file cannot be
opened, locked or verified, the published template fails its structural check
(it is then quarantined, identity-bound and best-effort), or the copy fails
with a verdict against the template's bytes or an I/O error. A copy that wrote
anything MUST first be preserved under the interrupted-stage protocol without
an engine start; only a verdict quarantines the template. A different project
opened while a build for the same key is in progress MUST take the cold path
at once and MUST NOT read any unpublished build or capture stage.

Any failure of a template build the open started (its engine, its own
validation and shape assertions, its capture and byte scan, or its
publication when no verified stage remains to copy from), and any failure of
the copied stage's own engine start, MUST fail the open with its error and
MUST NOT be retried in that open, on the template path or the cold path, so
the schema chain never runs twice in one open. When that
failure is a verdict against the template's bytes (adoption's placeholder,
working-set or rewrite comparison, or the template shape), Kuru MUST also
quarantine the published template the copy was taken from, bound to the
identity it had when it was judged; every other failure, including a
mismatched compiled template key, MUST leave every template untouched. The
unready stage is preserved: by the next open's recovery without an engine
start when the failure came before the stage's engine was serving, and by the
staging job itself after.

#### Scenario: An ordinary new project copies a warm template in two starts

- **WHEN** a new project is opened on a machine whose engine cache holds a
  published template for the process's compiled key
- **THEN** the project's stage is copied from that template, adopted,
  validated and marked ready on one engine start, then activated with the
  active start, so the open makes two engine starts and runs no schema step.

#### Scenario: The first project for a key builds the template once, in three starts

- **WHEN** a new project is opened with an engine cache that holds no
  published template for the process's compiled key, and no legacy import,
  configured engine binary or cold request applies
- **THEN** Kuru builds and publishes the template on one engine before
  copying this project's stage from it, the open makes three engine starts,
  the schema chain runs once, and a later new project copies the published
  template in two.

#### Scenario: Legacy import and a configured engine binary stay cold

- **WHEN** a new project's open carries a legacy import, or the open is
  configured with an explicit engine binary
- **THEN** Kuru takes the cold staged build without reading, building or
  copying any template.

#### Scenario: A busy, unreadable or damaged template sends the opener cold

- **WHEN** a new project's open meets a key lock another process holds, a
  lock or manifest error, a template that fails its structural check, or a
  verdict or I/O error while copying
- **THEN** the open completes on the cold staged build without a
  template-specific error, any partial copy is preserved without an engine
  start, and only a verdict against the template's bytes quarantines it.

#### Scenario: A verdict on the copy's own engine fails the open and quarantines

- **WHEN** a copied stage's engine start refuses the copy's placeholder
  identity or template shape
- **THEN** the open fails with the typed template verdict and no retry, no
  store appears at the project's active path, the unready stage is preserved,
  and the template it was copied from is quarantined; an engine failure of
  that start instead leaves the template untouched.

#### Scenario: A failed template build fails the open without a cold retry

- **WHEN** a new project's open builds the template for its key and that
  build's validation, shape assertion or byte scan refuses its result
- **THEN** the open fails with that error after the build engine's single
  start, makes no other engine start, runs no cold staged build, publishes
  and quarantines nothing, and leaves no store at the project's active path.

#### Scenario: A concurrent new project never waits on another project's template build

- **WHEN** one project's open is building and has not yet published a
  template for a key, and a second, different project is opened at the same
  time
- **THEN** the second project's open does not wait for the build: it takes
  the cold staged build at once and completes with its own engine starts,
  independently of when, or whether, the first project's build publishes.
