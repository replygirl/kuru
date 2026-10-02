# project-memory-owner Specification

## Purpose
Define the private per-project storage owner, authenticated local attachments, bounded typed requests, and engine cleanup that later memory-service clients rely on.

## Requirements

### Requirement: One project owner controls the live engine

Kuru SHALL elect at most one writable memory service for a canonical project and store. The starter SHALL retain a separate short election lock through authenticated readiness; the child SHALL retain owner authority before opening storage and through Dolt reap. Neither a stale endpoint nor a numeric PID SHALL authorize replacing or killing a live owner.

On Windows, an independent-service launch MUST first request Job breakaway. If and only if that exact create attempt fails with access denied and the current process is confirmed to belong to a Job, Kuru MAY recreate the mutable launch state and retry once without breakaway. The contained owner MUST remain independent of the starter process handle while remaining subject to the inherited nonpermitting Job. Kuru MUST NOT retry another create error, add a new owned Job, expand ordinary owned-process breakaway, or claim survival after the inherited Job closes.

#### Scenario: Two cold starters
- **WHEN** two independent processes start storage clients for the same cold canonical project at once
- **THEN** both attach to the same service generation, one owner opens Dolt, and both committed storage operations remain visible

#### Scenario: Starter exits after readiness
- **WHEN** the starter process exits while another authenticated client remains attached
- **THEN** the service and its owned Dolt supervisor remain available to that client

#### Scenario: Permitted Windows breakaway
- **WHEN** a Windows starter runs in a Job that permits explicit breakaway and exits after another client attaches
- **THEN** the service breaks away, survives closure of that starter Job, and retains the same owner generation and Dolt lifetime

#### Scenario: Inherited Windows containment
- **WHEN** a Windows starter belongs to a nonpermitting outer Job and the first breakaway create attempt is denied
- **THEN** one contained owner starts without breakaway, remains usable after the starter process exits while the outer Job is held, and retains its ordinary private endpoint, owner lock, and Dolt cleanup authority

#### Scenario: Outer Windows containment ends
- **WHEN** the nonpermitting outer Job closes after a contained owner has committed state
- **THEN** Windows terminates the contained owner and Dolt tree, and a later ordinary open recovers the committed store through the existing endpoint, owner-lock, lifecycle, and receipt rules without stale-lease takeover

### Requirement: Private generation-bound attachment

The owner SHALL accept only an authenticated local connection matching its protocol, canonical project path and scope, physical store instance, service generation, connection secret and schema version. Native endpoint permissions SHALL exclude other OS users; the system does not claim isolation from processes running as the same user that can read owner-private state. A rejected connection SHALL not gain a storage operation.

#### Scenario: Wrong identity
- **WHEN** a client presents a wrong project, store instance, generation, secret, protocol or schema
- **THEN** the owner rejects the handshake before reading a storage request and remains active

### Requirement: Bounded typed storage requests

The owner SHALL expose only enumerated storage operations over bounded request and response frames. Each response SHALL match the request ID and service generation. A broken or uncertain write response SHALL invalidate its attachment and SHALL NOT trigger automatic replay. Accepted mutations SHALL use the existing store's short serialization and durable receipt reconciliation.

#### Scenario: Concurrent typed writes
- **WHEN** two authenticated clients append typed messages concurrently
- **THEN** both committed messages remain readable from one owner without a general SQL execution endpoint

### Requirement: Attachment-bound idle cleanup

The owner SHALL retain the engine while any authenticated attachment remains or any accepted operation is unsettled. A newly started owner SHALL NOT retire for lack of attachments until the client that started it has attached, bounded by the startup budget measured from endpoint publication; it SHALL serve other clients meanwhile, and if its starter has not attached by then it SHALL retire once no attachment remains. After its starter has attached, once the last attachment has released and accepted work has settled, the owner SHALL begin shutdown immediately, with no idle interval. It SHALL stop accepting and retire its endpoint, then retire its activity record, then close the store and reap Dolt, and only then release its owner lock. Because no idle interval remains, a command that follows another pays a fresh owner start: measured on a macOS arm64 release build (10 samples; Ubuntu figure pending from CI), the median reopen right after a close was 1.207 s, a reopen after the predecessor had fully exited took 1.050 s median (the whole command: process start, owner spawn, existing-project open and demo turn), and the median close was 71 ms. A running writable client SHALL keep at least one authenticated attachment across a cancelled or failed request, without resending that request, except in three cases: the owner itself closes that client's only connection; the owner refuses the replacement connection and then ends the abandoned connection at its operation timeout before the client's next request; or the cancelled request is dropped outside an async runtime, so no replacement is opened. In those cases the owner MAY retire, and the client's next request SHALL fail with a truthful connect error and SHALL NOT be retried to hide it.

#### Scenario: Last client disconnects
- **WHEN** the last attached client disconnects after the starter has attached and no accepted operation remains
- **THEN** the owner begins shutdown without waiting, retires its endpoint, and releases its owner lock only after owned Dolt cleanup

#### Scenario: Owner starts before its starter attaches
- **WHEN** a new owner has published its endpoint and another client attaches and detaches before the starter
- **THEN** the owner keeps serving until its starter attaches within the startup budget, and the starter attaches to that same generation

#### Scenario: Client work outlasts other clients
- **WHEN** one attachment still holds the dream lease or has an accepted operation in flight and every other attachment has released
- **THEN** the owner keeps its engine until that attachment releases and the operation settles

#### Scenario: Cancelled call on a running client
- **WHEN** a running writable client's request is cancelled and it was that client's only connection
- **THEN** the owner remains for that client and its next request succeeds on the same generation

#### Scenario: Activity record retired before the store closes
- **WHEN** an owner begins shutdown, or its open fails after the record was published
- **THEN** the record is retired by rename then removal, after the endpoint retires and before the store closes, only while the owner still holds its owner lock, and the name is free for a successor at once.

### Requirement: Tagged open activity record

An owner started with a starter token, which its starter passes as the existing optional trailing service argument, SHALL publish, in one bounded owner-private record beside its endpoint, the ordered open stages its own observed open has begun, excluding ready and the retained-install stages. The record SHALL be tagged with a SHA-256 derived value of the starter token, never the raw token, because the raw token admits a starter attachment. The record SHALL grant no authority: election, attachment, recovery and retirement MUST NOT read it, and it MUST NOT be used to decide ownership, liveness or readiness. A failed publication or observation MUST NOT fail, cancel or delay the open, endpoint publication, or the client's readiness deadline, and MUST NOT change the meaning of `startup_timeout_secs`. A client SHALL present only a record carrying the value derived from the token it passed, SHALL read it only between readiness polls after its deadline check and never after a successful attach, and MUST NOT acquire a lock, connect, or read inside a store directory to observe activity. The owner SHALL retire the record by rename then removal within its own close and on every error return of its open, never by a detached task, and SHALL remove only a record carrying its own tag. An owner started without a token SHALL publish no record and open as before. A client that observes another process holding owner authority without an attachable endpoint MAY report a project-ownership wait, and SHALL report service start when it proceeds to start its own owner.

#### Scenario: Record from a previous owner
- **WHEN** a record with another tag exists as a new owner starts
- **THEN** the starter presents none of its stages.

#### Scenario: Raw token as tag
- **WHEN** a record's tag equals the raw starter token
- **THEN** the client rejects the record and presents none of its stages.

#### Scenario: Publication fails
- **WHEN** the owner cannot write any record
- **THEN** the open and endpoint publication proceed unchanged, and the client presents only the stages it observed itself.

#### Scenario: Record replaced while read
- **WHEN** the owner replaces or retires the record while a client holds it open for reading, on any supported platform
- **THEN** the replacement or retirement succeeds or is retried, and the open is unaffected.

#### Scenario: Untokened owner
- **WHEN** an owner is started without a starter token
- **THEN** it publishes no record and opens and serves as before.

#### Scenario: Owner waits while the previous owner closes
- **WHEN** a new command starts while the previous owner has retired its endpoint but still holds its owner lock
- **THEN** the client reports a project-ownership wait, and after that lock is released reports service start and then ready.

### Requirement: Owner open timeline

Only when the environment variable `KURU_OPEN_TIMELINE` is exactly `1` in the memory service owner process, the owner SHALL record named open milestones as nanosecond offsets from one monotonic anchor per process, in a bounded in-memory log sealed at endpoint publication. It SHALL write that log once, after it has released its owner lock in close, as a non-durable owner-private file named for its service generation beside its endpoint, created through the checked private-file handle so that it never follows a link, truncates or replaces an existing file. A gated owner started with a starter token SHALL also stream each stamp, starting with the stamps already logged, to a non-durable owner-private file named from that token's activity tag beside its endpoint, created through the same checked create-only handle and written one unsynced line per stamp; when that name is already taken it SHALL leave streaming off, and it SHALL NOT append to or remove a stream file it did not create. It SHALL remove its own stream file while it holds its owner lock, after endpoint publication and before it serves, or on a failed open, and never in close. The close-time file SHALL carry only event names, offsets, one wall-clock anchor, the service generation, the binary version and counts; each stream line SHALL carry only an event name, its offset and its wall-clock time; neither SHALL carry a path, scope, SQL text, store identity, connection secret, credential or content. Recording, streaming or writing MUST NOT fail, cancel or reorder the open or serve, MUST NOT change the close's result or the order of its steps, and MUST NOT lengthen any successor's owner-lock wait. A gated open adds at most one unsynced write of at most 62 bytes per stamp plus one removal before it serves, with no sync, wait or retry; after the lock release a gated close adds exactly one create-only, unsynced write of at most 8 KiB before it returns; and a failed open leaves no timeline file it created, except a stream file whose removal a concurrent reader made uncertain on Windows, which is not retried. A starter whose own environment has the variable exactly `1` MAY read only the stream file named from the token it passed to its owner, once, at its readiness deadline, without waiting or retrying; it SHALL then place an owner timeline clause before its client phase split that names the streamed events in file order and the last event, or reports the file absent, empty, stale (its first stamp precedes the starter's spawn) or unreadable, and that never carries a path, token, scope or tag. With the variable unset nothing SHALL change, including the supervisor protocol, the client's open stages and markers, and the owner-private directory contents. No other product code SHALL read, enumerate or remove these files. The variable MUST be documented only in the developer documentation. A Windows starter SHALL forward the variable to its owner only when it is exactly `1`.

#### Scenario: Unset changes nothing
- **WHEN** an owner opens, serves and closes without `KURU_OPEN_TIMELINE`, and a starter without it reaches its readiness deadline
- **THEN** the owner creates no `open-timeline-*` or `open-stream-*` file, the client's observed stage sequence is unchanged, no supervisor frame or marker differs, and the readiness deadline error carries no owner timeline clause

#### Scenario: Gated owner writes after releasing its lock
- **WHEN** an owner with `KURU_OPEN_TIMELINE=1` opens an existing project, serves and closes
- **THEN** no `open-timeline-*` file exists while the owner lock is held at any close step, and after the lock is released exactly one file named for the service generation exists with the canonical events in non-decreasing order, one wall-clock anchor, the binary version and the usage row count, and none of the data directory, project path, scope, scope hash, connection secret or store instance

#### Scenario: Write failure leaves the close unchanged
- **WHEN** the timeline file cannot be created because its name is occupied or its directory is unusable
- **THEN** the close still succeeds, the endpoint stays retired, the owner lock is free, and the occupying entry is untouched

#### Scenario: Successor owner during predecessor's write
- **WHEN** a successor owner starts after the predecessor released its owner lock but before the predecessor's timeline write finishes
- **THEN** the successor's open and its own generation-named file are unaffected, because each file's name is distinct and the write is exclusive-create

#### Scenario: Deadline names the held event
- **WHEN** a gated starter's gated owner is held after streaming `create-start` until the starter's readiness deadline passes
- **THEN** the deadline error keeps its leading text and carries `owner timeline:` with the owner's events in file order, `owner-main`, `owner-lock`, `startup-lock` and `create-start` among them, `last=create-start` and no later event, before a `client phases:` split that still parses

#### Scenario: Ungated starter reads nothing
- **WHEN** a starter without the variable reaches its readiness deadline while its owner's stream file exists
- **THEN** it reads no stream file and its error carries no owner timeline clause

#### Scenario: Stream absent after publication
- **WHEN** a gated owner started with a starter token publishes its endpoint and serves
- **THEN** its stream file was removed before it began serving, while it still held its owner lock, and no `open-stream-*` file remains

#### Scenario: Foreign tag never read
- **WHEN** a gated starter reaches its deadline while another token's stream file is present beside the endpoint
- **THEN** its clause is built only from the file named from its own token, or reports that file absent

#### Scenario: Taken stream name
- **WHEN** a gated owner's stream name is already taken by a file it did not create
- **THEN** the owner opens with streaming off and its in-memory log unchanged, the occupying file's bytes are unchanged and it is not removed, and a gated starter whose spawn followed that file's first stamp reports `owner timeline: stale`
