# Spec Delta

## ADDED Requirements

### Requirement: Pooled SQL session reuse

Kuru SHALL return a pooled memory SQL session to its pool before the next statement of the same task acquires one, so sequential statements on a pool reuse an idle authenticated session instead of authenticating a new one. A durable write session SHALL return to its pool only after the write's receipted success; any other outcome MUST end the session before outcome reconciliation, exactly as the uncertain-write fence requires. A store write that changes session state MUST end its session instead of returning it. Reuse MUST NOT skip the identity checks a new connection runs, lengthen any acquire bound or retry a timed-out acquire.

#### Scenario: Sequential statements on a fresh pool
- **WHEN** a task runs several statements and a read transaction one after another on a pool that holds one idle session
- **THEN** no new connection authenticates

#### Scenario: Receipted writes
- **WHEN** a writable store completes several receipted writes in sequence on its main, candidate or usage pool
- **THEN** each write reuses the pool's session and no new connection authenticates

#### Scenario: Uncertain or rejected write
- **WHEN** a write's outcome is not a receipted success
- **THEN** its SQL session has ended before the outcome is reconciled, and the pool no longer holds it

### Requirement: Diagnosed memory pool acquisition timeout

A memory pool acquisition that reaches its bound SHALL fail with a bounded, secret-free diagnostic that names the pool's branch; whether the wait was for connections held by Kuru work, for a new connection's authentication, or for an idle-connection check or release with no new connection started; the pool's maximum, size, idle and checked-out counts; the elapsed wait and its window; the connections authenticated since the pool opened and during the wait; and, only when a connection entered authentication during the wait, its latest phase. The diagnostic MUST reach the caller's error chain and MUST NOT include SQL text, credentials, endpoints or paths.

#### Scenario: Contended pool
- **WHEN** every connection of a pool is held and another acquisition reaches its bound
- **THEN** it fails naming a wait for held connections with the pool's size, checked-out count and window, and the chain still contains SQLx's pool timeout

#### Scenario: Stalled authentication
- **WHEN** a pool with spare capacity must open a connection whose authentication does not finish within the bound
- **THEN** the acquisition fails naming a new-connection wait, the authentication phase reached and one connection authenticated during the wait

#### Scenario: Wait with no new connection
- **WHEN** a pool with spare capacity times out while no connection started authenticating during the wait
- **THEN** the acquisition names an idle-check-or-release wait and reports no connection phase
