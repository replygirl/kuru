# Spec Delta

## MODIFIED Requirements

### Requirement: Pooled SQL session reuse

Kuru SHALL return a pooled memory SQL session to its pool before the next statement of the same task acquires one, so sequential statements on a pool reuse an idle authenticated session instead of authenticating a new one. A durable write session SHALL return to its pool only after the write's receipted success; any other outcome MUST end the session before outcome reconciliation, exactly as the uncertain-write fence requires. A store write that changes session state MUST end its session instead of returning it. Returning a session MUST be bounded by the budget of the work it served: a receipted write's return by what remains of that write's own budget, and a new pool's first connection by its pool creation's budget. A return that exceeds its bound MUST close the connection instead of pooling it, and MUST NOT turn a receipted write's success into an error. Reuse MUST NOT skip the identity checks a new connection runs, change any acquire bound other than as the diagnosed memory pool acquisition timeout requirement states, or retry a timed-out acquire.

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
- **WHEN** a pool created after memory is open has a first connection return that does not finish within its pool creation's budget
- **THEN** the creation fails at that budget, the connection is closed, and no pool is retained for the branch
- **WHEN** a pool created while memory is opening has a first connection return that does not finish by its pool attempt's deadline
- **THEN** the pool still opens, that connection is closed, and identity verification authenticates its own connection under its own deadline

### Requirement: Diagnosed memory pool acquisition timeout

Once memory is open, a memory pool acquisition SHALL be bounded by the remaining budget of the statement or operation it serves, never by a separate shorter window. Each pool's lifetime acquisition ceiling MUST equal the memory statement budget, and no acquisition bound MUST be set above the budget it serves. A pool's first acquisition while memory is opening keeps the bound derived from the startup budget. A pool created after memory is open SHALL run its first acquisition, first connection return and identity verification under one creation budget. A receipt-bearing write SHALL take one write budget before its pool acquisition, and that budget SHALL cover the acquisition, its identity statement, any validation before its pending record, its apply and its session return, so these end within one statement budget from before the acquisition. A store mutation, session catalog write, candidate creation or usage ledger change SHALL take that budget once it holds the store's write lock, so it also covers every read before its pending record. A candidate promotion merge, status transition, deletion or session exclusion, and a usage validation record, SHALL take its budget at its acquisition; their earlier reads keep their own statement budgets. The wait for the write lock, those earlier reads, a multi-write candidate operation's other writes and session retirement, and reconciliation after a write that ends without its receipt are outside the write budget; reconciliation stays on the uncertain-write fence's own path. A memory pool acquisition that reaches its bound SHALL fail with a bounded, secret-free diagnostic that names the pool's branch; which bound ended the wait (the statement budget or the pool ceiling), that budget and the share of it left when the acquisition began; whether the wait was for connections held by Kuru work, for a new connection in Kuru's identity callback, or with no new connection reaching that callback (an idle-connection check, a release in flight, or a TCP or MySQL handshake that did not finish, which Kuru cannot tell apart); the pool's maximum, size, idle and checked-out counts; the elapsed wait; the connections that entered the identity callback since the pool opened and during the wait; and, only when a connection entered the callback during the wait, its latest phase. The diagnostic MUST reach the caller's error chain and MUST NOT include SQL text, credentials, endpoints or paths. A budget that expires after the acquisition completed MUST NOT be reported as an acquisition timeout. An acquisition that fails before its first statement MUST NOT make a write uncertain. An authored identity rejection MUST end the acquisition at once with its cause, never as a timeout, and MUST fail every later acquisition on that pool for the pool's life. An acquisition still pending past the slow-acquire threshold MUST leave a diagnostics record, and that threshold MUST NOT decide any acquisition's outcome.

#### Scenario: Contended pool
- **WHEN** every connection of a pool is held and another acquisition reaches its bound
- **THEN** it fails naming a wait for held connections with the pool's size, checked-out count, the bound that ended the wait and its budget, and the chain contains SQLx's pool timeout only when the pool ceiling, not the statement budget, ended the wait

#### Scenario: Stalled identity callback
- **WHEN** a pool with spare capacity must open a connection whose identity callback does not finish within the bound
- **THEN** the acquisition fails naming a new-connection wait, the callback phase reached and one connection entering the callback during the wait

#### Scenario: Wait with no identity callback reached
- **WHEN** a pool with spare capacity times out while no new connection reached the identity callback during the wait, including when a TCP or MySQL handshake has not finished
- **THEN** the acquisition names a wait with no new connection reaching the identity callback and reports no connection phase

#### Scenario: Cancelled release
- **WHEN** a session's release is cancelled after the pool has taken its connection, and a later acquisition reaches its bound
- **THEN** the cancelled session is not counted as checked out, and the wait class reflects only the sessions Kuru work still holds

#### Scenario: Slow but successful contended acquisition
- **WHEN** every connection of an open pool is held past the slow-acquire threshold and one is released inside the waiting statement's budget
- **THEN** the waiting acquisition succeeds, and a diagnostics record names it as still waiting

#### Scenario: Budget expired in execution
- **WHEN** a statement's budget expires after its acquisition has completed
- **THEN** the error names the elapsed statement budget and carries no acquisition timeout diagnostic

#### Scenario: Acquisition failure before a write
- **WHEN** a receipt-bearing write's acquisition fails before its first statement
- **THEN** the write fails with that cause and no uncertain outcome is recorded

#### Scenario: Contended receipt-bearing write
- **WHEN** a receipt-bearing write waits for a pool connection
- **THEN** that wait is charged to the one write budget taken before its acquisition, and the acquisition, identity statement, any validation before its pending record and apply end within that budget; for a store mutation, session catalog write, candidate creation or usage ledger change, a wait of a read before its pending record is charged to the same budget

#### Scenario: Identity rejection on a retained pool
- **WHEN** a new connection of an open pool is rejected by the identity callback with an authored mismatch
- **THEN** the acquisition fails at once naming the mismatch with no acquisition timeout, and every later acquisition on that pool fails the same way

#### Scenario: Identity rejection during post-open pool creation
- **WHEN** a pool created after memory is open has its first connection rejected by the identity callback with an authored mismatch
- **THEN** the creation fails at once naming the mismatch with no acquisition timeout, and no pool is retained for the branch

#### Scenario: Post-open pool creation outlasting the slow-acquire threshold
- **WHEN** a pool created after memory is open has a first connection whose authentication outlasts the slow-acquire threshold but finishes inside its creation budget
- **THEN** the creation succeeds and the pool's lifetime acquisition ceiling equals the memory statement budget
