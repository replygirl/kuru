# versioned-memory Specification

## Purpose
Preserve private project memory through managed Dolt ownership, atomic revisions,
isolated candidates and recoverable legacy imports.

## Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official
native archive and runtime licenses in its executable. It MUST provision the
matching engine locally without a compiler, separate installation or runtime
download, including on a first offline launch. It MUST reject unsafe archives
and corrupt executables before execution and SHALL NOT silently fall back to
SQLite. Existing verified caches MAY be reused; corrupt existing caches MUST fail
explicitly without destructive repair.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using that verified cache; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.

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

Memory SHALL preserve byte-sensitive namespace/key identity, message order,
opaque JSON values and existing validation. Mutations SHALL be atomic versioned
batches with operation identity for uncertain-response reconciliation. Views MUST
remain pinned to their branch, and candidate promotion MUST require its live base.

#### Scenario: Transaction failure
- **WHEN** a batch fails after one row change
- **THEN** no partial batch is visible and prior history remains readable.

#### Scenario: Lost acknowledgement
- **WHEN** a committed operation loses its response
- **THEN** reconciliation identifies its committed result without duplicating the mutation.

#### Scenario: Candidate divergence
- **WHEN** live memory changes after a candidate captures its base
- **THEN** promotion fails without overwriting either history.

### Requirement: Preserved SQLite migration

Migration MUST validate the legacy store, preserve its original and a consistent
snapshot including committed WAL data, and activate only a validated committed
Dolt import. It SHALL preserve current-project rows, ordering and JSON exactly;
other projects SHALL remain recoverable from the preserved source. Interrupted
imports SHALL be recoverable without duplicate activation or source modification.

#### Scenario: Existing conversations
- **WHEN** an existing project first opens with Dolt
- **THEN** sessions, preferences, topology, private histories and relationship memories remain available in the same order.

#### Scenario: Failed import
- **WHEN** validation or activation is interrupted
- **THEN** the original remains usable and no partial target becomes the active memory store.

### Requirement: Memory inspection

Kuru SHALL expose project memory status and revision history and document managed
runtime configuration, migration, offline use and stopped-store backup/recovery.

#### Scenario: Revision inspection
- **WHEN** the user requests memory history after a committed update
- **THEN** the command reports the durable revision without exposing credentials or unrelated projects.
