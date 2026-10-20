# Spec Delta

## MODIFIED Requirements

### Requirement: Tagged open activity record

An owner started with a starter token, which its starter passes as the existing optional trailing service argument, SHALL publish, in one bounded owner-private record beside its endpoint, the ordered open stages its own observed open has begun, excluding ready and the retained-install stages, and a progress count that advances only when its open reaches a distinct one-shot point or completes a bounded unit of work, never on a timer, inside a sleep, a retry iteration or a wait for a lock. Each stage report of the open and of its workers SHALL advance that count, and so SHALL every engine start's supervisor spawn call, its supervisor spawn and, on Windows, its private channel accept, and, while the open creates a project, its completed legacy import probe, its staging recovery check, each completed template copy, the stage identity write, the template stage's reap and the activation, so that no completed step of a creation between its `OpeningDatabase` report and its stage engine's readiness leaves the count unchanged. The count SHALL belong to that one open and SHALL NOT be advanced by any other open in the same process. Immediately before an open that has failed closes the engine it started, and only where that failure is returned from the open, the owner SHALL mark its open failing with a failure reason taken from the error's own text; its publisher SHALL write that mark and reason into the record at once, without delaying that close, truncating the reason at a character boundary so that the record stays within its limit, and the reason SHALL carry no starter token, tag or connection secret. An owner whose store has opened but which ends before its starter has attached, because its listener, endpoint publication or serve loop failed, SHALL likewise mark its record failing with that failure's own reason before its store closes, only while it holds verified owner authority, and SHALL await that one write as its retirement already awaits the publisher; it SHALL then close the store and reap Dolt, and only then retire the record, whatever that close returns. If it cannot verify its authority or that write fails, it SHALL retire the record before its store closes, so its starter still fails at once. The record SHALL be tagged with a SHA-256 derived value of the starter token, never the raw token, because the raw token admits a starter attachment. The record SHALL grant no authority: election, attachment, recovery and retirement MUST NOT read it, and it MUST NOT be used to decide ownership or to authorize a lock, attachment, spawn, termination or recovery; a change of the record's content MAY only extend, and its retirement after it was read or its failing mark MAY only end, its own starter's readiness wait. A failed publication or observation MUST NOT fail, cancel or delay the open or endpoint publication, and MUST NOT extend the client's readiness wait. While its publisher lives, the owner SHALL replace the record only by publishing a complete staged record over its name, so that the name is never absent between the first write and the record's retirement. A client SHALL present only a record carrying the value derived from the token it passed, SHALL read it at most once per readiness poll, after that poll's attach attempt and owner-exit check, and never after a successful attach, and MUST NOT acquire a lock, connect, or read inside a store directory to observe activity. The owner SHALL retire the record by rename then removal within its own close and on every error return of its open, never by a detached task, and SHALL remove only a record carrying its own tag. An owner started without a token SHALL publish no record and open as before. A client that observes another process holding owner authority without an attachable endpoint MAY report a project-ownership wait, and SHALL report service start when it proceeds to start its own owner.

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

#### Scenario: Template creation keeps advancing until the stage engine is ready
- **WHEN** a tokened owner creating a new project from the store template is held at its first `supervisor-spawned`, after its `OpeningDatabase` report
- **THEN** its record's count is past the count that report left, from the completed legacy probe, staging recovery check, template copy, stage identity write and engine spawn call, while its stages are unchanged

#### Scenario: A creation held past one window still stalls
- **WHEN** that owner stays held for longer than `startup_timeout_secs` after its last advance
- **THEN** its starter fails with `memory service readiness deadline exceeded`, naming `OpeningDatabase`, a count past that report's and, when gated, `last=supervisor-spawned`, and the owner is not killed
