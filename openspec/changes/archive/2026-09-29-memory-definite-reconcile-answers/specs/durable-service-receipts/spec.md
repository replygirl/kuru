# Spec Delta

## MODIFIED Requirements

### Requirement: Bounded four-state outcome reconciliation

The service SHALL accept a typed outcome query for an exact logical mutation ID, original view and request fingerprint, returning in-flight, committed with a verified typed result, definitively absent, or still uncertain. It MUST NOT report absence as noncommit until the original accepted worker has ended, or a failed owner and its Dolt child have been verified reaped and the branch recovered. The service SHALL report a definite outcome whenever one exists. Durable evidence of the exact committed result of a main-view unit write or a usage ledger record SHALL be reported as committed even while its accepted worker is still registered. Otherwise, when the worker is registered, the service SHALL wait for that worker to settle within the query's existing operation deadline, without holding the write guard, while other attachments' reads and writes proceed, and then report the outcome from evidence read after settlement; it SHALL report in-flight only when that wait ends first. A worker counts as ended for the purpose of reporting absence only when its request handler returned; a handler that was dropped or aborted SHALL leave a missing result still uncertain for the rest of that service generation. When the querying client disconnects or cancels during the wait, the service SHALL end the wait promptly and release the attachment's resources without replying. Settlement relies on an assumption the existing uncertain-write fence already makes and which is not independently verified: Dolt removes a session's process-list entry only after that session's running command returns. A missing exact candidate ref after explicit resolution/reclamation MUST NOT by itself prove that an earlier private write did not commit. An incomplete mutating reply SHALL fence further mutations through every handle of that logical client session until the exact outcome is reconciled; reconnect alone SHALL NOT clear uncertainty or authorize automatic replay. A complete authenticated rejection MAY leave the attachment usable.

#### Scenario: Owner crashes after commit before reply
- **WHEN** the owner commits a request, crashes before its reply reaches the client, and a successor opens the project
- **THEN** the successor waits for old owner/Dolt cleanup and reports the matching committed outcome from retained evidence without redispatching the request.

#### Scenario: Work remains in flight
- **WHEN** an accepted worker has neither published its result nor settled before the outcome query's settlement wait ends
- **THEN** the query reports in-flight, the affected logical client stays fenced, and noncommit is not inferred from a missing row.

#### Scenario: Committed result is visible before the worker settles
- **WHEN** an outcome query for a main-view unit write or usage ledger record finds the exact durable receipt while its accepted worker is still registered
- **THEN** it reports committed without waiting for the worker, and the client's next mutation is ordered after that worker by the owner.

#### Scenario: Query arrives before the result is published
- **WHEN** an outcome query arrives while the accepted worker is registered and no durable evidence exists yet, or the query concerns a candidate view, candidate creation or candidate transition
- **THEN** the owner answers after the worker settles, reporting a definite outcome from evidence read after settlement, while other attachments' reads and writes proceed.

#### Scenario: Querying client leaves during the wait
- **WHEN** the client that sent an outcome query disconnects or cancels while the owner waits for settlement
- **THEN** the owner ends that wait promptly and holds no frame budget, attachment slot or retirement count for it afterwards.

#### Scenario: Aborted handler cannot prove absence
- **WHEN** a registered request's handler is dropped before it returned
- **THEN** a same-generation outcome query that finds no receipt reports still uncertain, whatever other requests with the same ID do.

#### Scenario: Candidate receipt view was reclaimed
- **WHEN** an explicitly resolved candidate ref was reclaimed and its private receipt no longer appears on a reachable branch
- **THEN** a missing indexed row alone reports still uncertain rather than falsely asserting that its earlier accepted candidate write never committed.

#### Scenario: Cancelled write and sibling handle
- **WHEN** a client cancels a mutating RPC after dispatch but before a complete authenticated reply
- **THEN** its attachment is invalidated, sibling store/candidate/ledger handles cannot mutate, and a later authenticated outcome query uses the original logical ID rather than replaying the write.
