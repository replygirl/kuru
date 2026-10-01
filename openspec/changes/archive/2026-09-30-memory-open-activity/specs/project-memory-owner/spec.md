# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
