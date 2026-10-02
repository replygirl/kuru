# Spec Delta

## ADDED Requirements

### Requirement: Pooled SQL session reuse

Kuru SHALL return a pooled memory SQL session to its pool before the next statement of the same task acquires one, so sequential statements on a pool reuse an idle authenticated session instead of authenticating a new one. A durable write session SHALL return to its pool only after the write's receipted success; any other outcome MUST end the session before outcome reconciliation, exactly as the uncertain-write fence requires. A store write that changes session state MUST end its session instead of returning it. Returning a session MUST be bounded by the budget of the work it served: a receipted write's return by what remains of that write's own budget, and a new pool's first connection by its pool attempt's deadline. A return that exceeds its bound MUST close the connection instead of pooling it, and MUST NOT turn a receipted write's success into an error. Reuse MUST NOT skip the identity checks a new connection runs, lengthen any acquire bound or retry a timed-out acquire.

#### Scenario: Sequential statements on a fresh pool
- **WHEN** a task runs several statements and a read transaction one after another on a pool that holds one idle session
- **THEN** no new connection authenticates

#### Scenario: Receipted writes
- **WHEN** a writable store completes several receipted writes in sequence on its main, candidate or usage pool
- **THEN** each write reuses the pool's session and no new connection authenticates

#### Scenario: Uncertain or rejected write
- **WHEN** a write's outcome is not a receipted success
- **THEN** its SQL session has ended before the outcome is reconciled, and the pool no longer holds it

#### Scenario: Receipted write whose return outlasts its budget
- **WHEN** a receipted write's session return does not finish within that write's budget
- **THEN** the write succeeds once its budget ends, and its connection is closed instead of held by the pool

#### Scenario: First connection whose return outlasts its pool attempt
- **WHEN** a new pool's first connection return does not finish by its pool attempt's deadline
- **THEN** the pool still opens, that connection is closed, and identity verification authenticates its own connection under its own deadline

### Requirement: Diagnosed memory pool acquisition timeout

A memory pool acquisition that reaches its bound SHALL fail with a bounded, secret-free diagnostic that names the pool's branch; whether the wait was for connections held by Kuru work, for a new connection in Kuru's identity callback, or with no new connection reaching that callback (an idle-connection check, a release in flight, or a TCP or MySQL handshake that did not finish, which Kuru cannot tell apart); the pool's maximum, size, idle and checked-out counts; the elapsed wait and its window; the connections that entered the identity callback since the pool opened and during the wait; and, only when a connection entered the callback during the wait, its latest phase. The diagnostic MUST reach the caller's error chain and MUST NOT include SQL text, credentials, endpoints or paths.

#### Scenario: Contended pool
- **WHEN** every connection of a pool is held and another acquisition reaches its bound
- **THEN** it fails naming a wait for held connections with the pool's size, checked-out count and window, and the chain still contains SQLx's pool timeout

#### Scenario: Stalled identity callback
- **WHEN** a pool with spare capacity must open a connection whose identity callback does not finish within the bound
- **THEN** the acquisition fails naming a new-connection wait, the callback phase reached and one connection entering the callback during the wait

#### Scenario: Wait with no identity callback reached
- **WHEN** a pool with spare capacity times out while no new connection reached the identity callback during the wait, including when a TCP or MySQL handshake has not finished
- **THEN** the acquisition names a wait with no new connection reaching the identity callback and reports no connection phase

#### Scenario: Cancelled release
- **WHEN** a session's release is cancelled after the pool has taken its connection, and a later acquisition reaches its bound
- **THEN** the cancelled session is not counted as checked out, and the wait class reflects only the sessions Kuru work still holds
