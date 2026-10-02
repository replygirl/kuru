# project-memory-owner Specification

## Purpose
Define the private per-project storage owner, authenticated local attachments, bounded typed requests, and engine cleanup that later memory-service clients rely on.

## Requirements

### Requirement: One project owner controls the live engine

Kuru SHALL elect at most one writable memory service for a canonical project and store. The starter SHALL retain a separate election lock through authenticated readiness or the end of its readiness wait; the child SHALL retain owner authority before opening storage and through Dolt reap. Neither a stale endpoint nor a numeric PID SHALL authorize replacing or killing a live owner. Election, the wait for a previous owner to close, and a read-only inspection's wait for a booting owner SHALL each remain bounded by one `startup_timeout_secs` from the command's start; only the elected starter's wait for the owner it spawned is bounded by that owner's progress, as "Readiness wait bounded by owner progress" states.

On Windows, an independent-service launch MUST first request Job breakaway. If and only if that exact create attempt fails with access denied and the current process is confirmed to belong to a Job, Kuru MAY recreate the mutable launch state and retry once without breakaway. The contained owner MUST remain independent of the starter process handle while remaining subject to the inherited nonpermitting Job. Kuru MUST NOT retry another create error, add a new owned Job, expand ordinary owned-process breakaway, or claim survival after the inherited Job closes.

#### Scenario: Two cold starters
- **WHEN** two independent processes start storage clients for the same cold canonical project at once and the elected owner is ready within `startup_timeout_secs` of the second client's start
- **THEN** both attach to the same service generation, one owner opens Dolt, and both committed storage operations remain visible

#### Scenario: Second starter during a slow owner start
- **WHEN** the elected owner keeps progressing beyond `startup_timeout_secs` from a second client's start
- **THEN** the second client fails with the election deadline, the first keeps waiting and attaches, and no second owner is spawned

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

The owner SHALL retain the engine while any authenticated attachment remains or any accepted operation is unsettled. A newly started owner SHALL NOT retire for lack of attachments until the client that started it has attached, bounded by the startup budget measured from endpoint publication; it SHALL serve other clients meanwhile, and if its starter has not attached by then it SHALL retire once no attachment remains. After its starter has attached, once the last attachment has released and accepted work has settled, the owner SHALL begin shutdown immediately, with no idle interval. It SHALL stop accepting and retire its endpoint, then retire its activity record, then close the store and reap Dolt, and only then release its owner lock; an owner whose loop fails before its starter has attached orders its record's failing mark and retirement as the tagged open activity record requires. Because no idle interval remains, a command that follows another pays a fresh owner start: measured on a macOS arm64 release build (10 samples; Ubuntu figure pending from CI), the median reopen right after a close was 1.207 s, a reopen after the predecessor had fully exited took 1.050 s median (the whole command: process start, owner spawn, existing-project open and demo turn), and the median close was 71 ms. A running writable client SHALL keep at least one authenticated attachment across a cancelled or failed request, without resending that request, except in three cases: the owner itself closes that client's only connection; the owner refuses the replacement connection and then ends the abandoned connection at its operation timeout before the client's next request; or the cancelled request is dropped outside an async runtime, so no replacement is opened. In those cases the owner MAY retire, and the client's next request SHALL fail with a truthful connect error and SHALL NOT be retried to hide it.

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
- **WHEN** an owner begins shutdown after its starter has attached, after its startup budget has passed, or on a failure it could not mark
- **THEN** the record is retired by rename then removal, after the endpoint retires and before the store closes, only while the owner still holds its owner lock, and the name is free for a successor at once.

### Requirement: Tagged open activity record

An owner started with a starter token, which its starter passes as the existing optional trailing service argument, SHALL publish, in one bounded owner-private record beside its endpoint, the ordered open stages its own observed open has begun, excluding ready and the retained-install stages, and a progress count that advances only when its open reaches a distinct one-shot point or completes a bounded unit of work, never on a timer, inside a sleep, a retry iteration or a wait for a lock. The count SHALL belong to that one open and SHALL NOT be advanced by any other open in the same process. Immediately before an open that has failed closes the engine it started, and only where that failure is returned from the open, the owner SHALL mark its open failing with a failure reason taken from the error's own text; its publisher SHALL write that mark and reason into the record at once, without delaying that close, truncating the reason at a character boundary so that the record stays within its limit, and the reason SHALL carry no starter token, tag or connection secret. An owner whose store has opened but which ends before its starter has attached, because its listener, endpoint publication or serve loop failed, SHALL likewise mark its record failing with that failure's own reason before its store closes, only while it holds verified owner authority, and SHALL await that one write as its retirement already awaits the publisher; it SHALL then close the store and reap Dolt, and only then retire the record, whatever that close returns. If it cannot verify its authority or that write fails, it SHALL retire the record before its store closes, so its starter still fails at once. The record SHALL be tagged with a SHA-256 derived value of the starter token, never the raw token, because the raw token admits a starter attachment. The record SHALL grant no authority: election, attachment, recovery and retirement MUST NOT read it, and it MUST NOT be used to decide ownership or to authorize a lock, attachment, spawn, termination or recovery; a change of the record's content MAY only extend, and its retirement after it was read or its failing mark MAY only end, its own starter's readiness wait. A failed publication or observation MUST NOT fail, cancel or delay the open or endpoint publication, and MUST NOT extend the client's readiness wait. While its publisher lives, the owner SHALL replace the record only by publishing a complete staged record over its name, so that the name is never absent between the first write and the record's retirement. A client SHALL present only a record carrying the value derived from the token it passed, SHALL read it at most once per readiness poll, after that poll's attach attempt and owner-exit check, and never after a successful attach, and MUST NOT acquire a lock, connect, or read inside a store directory to observe activity. The owner SHALL retire the record by rename then removal within its own close and on every error return of its open, never by a detached task, and SHALL remove only a record carrying its own tag. An owner started without a token SHALL publish no record and open as before. A client that observes another process holding owner authority without an attachable endpoint MAY report a project-ownership wait, and SHALL report service start when it proceeds to start its own owner.

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

#### Scenario: Failed open marks its record before closing its engine
- **WHEN** a tokened owner's open fails after it started its engine and that failure is returned from the open
- **THEN** the owner marks its open failing with a bounded reason from that failure before the engine's close begins, the record carries that mark and reason without the close waiting for the write, and the record is retired after that close and before the owner exits

#### Scenario: Owner ending before its starter attached marks its record before its store closes
- **WHEN** a tokened owner's store has opened and its endpoint publication or serve loop then fails before its starter has attached
- **THEN** the owner marks the record failing with a bounded reason from that failure, keeping its final stages and count, before its store closes, the close waits only for that one write, and the record is retired after the close and before the owner releases its owner lock

### Requirement: Owner open timeline

Only when the environment variable `KURU_OPEN_TIMELINE` is exactly `1` in the memory service owner process, the owner SHALL record named open milestones as nanosecond offsets from one monotonic anchor per process, in a bounded in-memory log sealed at endpoint publication. It SHALL write that log once, after it has released its owner lock in close, as a non-durable owner-private file named for its service generation beside its endpoint, created through the checked private-file handle so that it never follows a link, truncates or replaces an existing file. The file SHALL carry only event names, offsets, one wall-clock anchor, the service generation, the binary version and counts, and never a path, scope, SQL text, store identity, connection secret, credential or content. Recording or writing MUST NOT fail, cancel, delay or reorder the open or serve, MUST NOT change the close's result or the order of its steps, and MUST NOT lengthen any successor's owner-lock wait; after the lock release a gated close adds exactly one create-only, unsynced write of at most 8 KiB before it returns, and a failed open writes nothing. With the variable unset nothing SHALL change, including the supervisor protocol, the client's open stages and markers, and the owner-private directory contents. No product code SHALL read, enumerate or remove these files. The variable MUST be documented only in the developer documentation. Support is not claimed on Windows, whose owner does not inherit the variable.

#### Scenario: Unset changes nothing
- **WHEN** an owner opens, serves and closes without `KURU_OPEN_TIMELINE`
- **THEN** it creates no `open-timeline-*` file, the client's observed stage sequence is unchanged, and no supervisor frame or marker differs

#### Scenario: Gated owner writes after releasing its lock
- **WHEN** an owner with `KURU_OPEN_TIMELINE=1` opens an existing project, serves and closes
- **THEN** no `open-timeline-*` file exists while the owner lock is held at any close step, and after the lock is released exactly one file named for the service generation exists with the canonical events in non-decreasing order, one wall-clock anchor, the binary version and the usage row count, and none of the data directory, project path, scope, scope hash, connection secret or store instance

#### Scenario: Write failure leaves the close unchanged
- **WHEN** the timeline file cannot be created because its name is occupied or its directory is unusable
- **THEN** the close still succeeds, the endpoint stays retired, the owner lock is free, and the occupying entry is untouched

#### Scenario: Successor owner during predecessor's write
- **WHEN** a successor owner starts after the predecessor released its owner lock but before the predecessor's timeline write finishes
- **THEN** the successor's open and its own generation-named file are unaffected, because each file's name is distinct and the write is exclusive-create

### Requirement: Readiness wait bounded by owner progress

By the maintainer's ruling of 2026-10-02, no readiness wait SHALL abandon an owner that is still making progress. This supersedes the earlier rule that the open activity record never changes the meaning of `startup_timeout_secs`: for the elected starter's wait for the owner it spawned, `startup_timeout_secs` SHALL mean "no progress for this long", while its value, documented range and parse are unchanged. After spawning its owner, the starter SHALL wait for that owner's authenticated readiness while the owner's own tagged record changes. It SHALL take the time of progress from its own monotonic clock at the poll that first observes a record whose stages, progress count or failing mark differ from the last record it read; a missing, foreign, unreadable or unchanged record SHALL move nothing. The first window SHALL start when the owner is spawned. The wait SHALL fail with a typed error, whose leading texts are distinct and none a prefix of another:

- `memory service readiness deadline exceeded` after `startup_timeout_secs` with no change, naming the last stage seen, the advances seen and the time since the last change, followed by the client phase split;
- `memory service exited before readiness` at once when its owner has exited, naming the exit status and the last stage seen;
- `memory service ended its open before readiness` at once when a record it has read is no longer present before it has attached, naming the last stage seen;
- `memory service open failed before readiness` at once when the record it reads is marked failing, naming the recorded reason and the last stage seen.

Stage names SHALL be the record's stage names; no path, tag or token SHALL appear in the progress part of the error. Each engine start SHALL remain bounded by the owner's own deadline, so a single engine start longer than `startup_timeout_secs` still fails. The wait SHALL have no overall cap, SHALL NOT retry, and SHALL NOT kill, replace or signal its owner.

#### Scenario: Advancing owner is waited for
- **WHEN** the owner's record changes at least once in every `startup_timeout_secs` window and the owner becomes ready several windows after spawn
- **THEN** the starter attaches to that owner without error

#### Scenario: Stalled owner is abandoned naming its stage
- **WHEN** the owner's record last changed while naming a stage and then stays unchanged for `startup_timeout_secs`
- **THEN** the starter fails with `memory service readiness deadline exceeded`, naming that stage and a time since progress of one window, and the owner is not killed

#### Scenario: Exited owner fails at once
- **WHEN** the owner exits before readiness after the starter read its record naming a stage
- **THEN** the starter fails at the poll that observes the exit with `memory service exited before readiness`, the exit status and that stage, without waiting for the window

#### Scenario: Retired record fails at once
- **WHEN** a record the starter has read is retired before the starter attaches while the owner process remains alive
- **THEN** the starter fails at the poll that finds the record gone with `memory service ended its open before readiness`, naming the last stage seen, without waiting for the window

#### Scenario: Failing owner fails at once with its reason
- **WHEN** the owner marks its open failing with a reason before closing its engine, and its record carries that mark
- **THEN** the starter fails at the poll that reads that record with `memory service open failed before readiness`, naming the reason and the last stage seen, without waiting for the engine reap or the window

#### Scenario: Silent owner is abandoned one window after spawn
- **WHEN** the owner stays alive and never publishes a record
- **THEN** the starter fails with `memory service readiness deadline exceeded` one `startup_timeout_secs` after spawn, naming no stage
