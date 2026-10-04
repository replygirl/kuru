# Spec Delta

## MODIFIED Requirements

### Requirement: Owned process tree lifecycle

Windows owned process execution MUST establish ownership before executing child
code, supplying Job and inherited-handle attributes atomically at creation
without a spawn-then-assign interval. Job ownership MUST enforce cleanup when
its owner dies.

The platform MAY provide a narrow Unix fresh-process-group owner for the
built-in shell, but it MUST own spawn, the standard child, root identity,
signal-before-reap transition, reaping, and absence observation together. It
MUST NOT expose a child or numeric identity that a safe caller can independently
reap and later signal. It MUST observe ownership immediately before signalling,
retry a bounded `EINTR` without consuming authority, permanently disarm on
`ECHILD` or another ownership-invalidating error, and signal the original group
and root only while the exact root is unreaped. The immediate initial transition
MUST remain one-shot. A nonblocking pre-reap step MAY repeat an ordered sweep
only after a fresh exact-root ownership observation and before the caller's
existing absolute deadline. It MUST retain at most one read-only membership
worker, launch no listing for a running root, exclude only the retained root
from listed group membership, and keep descendant zombies or unavailable
listings pending. Snapshot admission MUST use bounded spawn-lock acquisition
and the absolute inspection deadline. Worker cancellation/expiry MUST retain
helper cleanup ownership; late results MUST NOT cause signals. After reap or
disarm, only read-only group observation is permitted.
Completion waiting MUST retain its receiver across caller cancellation and use
the caller's existing poll/deadline; it MUST NOT join an unfinished worker or
replace the pre-reap step's result classification and fresh signal authority.

The Unix boundary MUST NOT claim that a process group provides atomic
owner-death containment or controls processes that deliberately escape it.
Windows cancellation and close MUST observe root reaping and actual owned-tree
quiescence before reporting completion. A Unix ordinary shell result MUST
observe root reap and group absence; a bounded cleanup failure MAY return an
unconfirmed result only while an independent owner retains the child and
associated capability until later confirmation. Stale PIDs MUST NOT authorize
termination on any platform. Pre-reap readiness MUST NOT replace the final
post-reap absence and output gates or allocate another cleanup allowance.

#### Scenario: Owner disappears during startup
- **WHEN** a fixture owner using the owner-loss contract exits at a creation or startup handshake boundary
- **THEN** its child tree cannot persist beyond the bounded owned cleanup and unrelated processes remain untouched.

#### Scenario: Root exits before its descendant
- **WHEN** a root exits while an owned grandchild is active or retains output
- **THEN** root status alone does not complete the tree wait; descendants and owned output are quiescent before completion is reported.

#### Scenario: Concurrent child inheritance
- **WHEN** concurrent children have distinct explicit inherited-handle lists
- **THEN** neither receives the other's private handles and independent lifetime closure remains observable.

#### Scenario: Unix signal authority is consumed before reap
- **WHEN** a fresh Unix shell group reaches natural or forced cleanup
- **THEN** platform signals the original group then root while retaining the unreaped standard child, freshly authorizes any bounded repeated sweep, reaps the exact root status, and performs no destructive numeric signal afterward.

#### Scenario: Unix child ownership cannot be established
- **WHEN** non-reaping observation reports `ECHILD` or another ownership-invalidating error before any destructive cleanup attempt
- **THEN** platform permanently disarms signal authority, reports the distinct ownership state, and never blindly signals the saved numeric group or root.

#### Scenario: Unix observation remains interrupted beyond caller return
- **WHEN** bounded `EINTR` exhausts an observation attempt while the exact root remains owned and unreaped
- **THEN** platform sends nothing and retains authority so a later valid observation within the existing deadline may perform its permitted transition or repeated sweep.

#### Scenario: First Unix group sweep misses a live descendant
- **WHEN** a ready descendant survives the initial sweep in the original group while the root exits
- **THEN** pre-reap settlement observes the member and performs a real freshly authorized second sweep before exact root reap; a descendant zombie remains pending.

#### Scenario: Unix inspection is queued past cleanup expiry
- **WHEN** worker admission or the shared spawn lock remains unavailable until the caller's deadline
- **THEN** no late snapshot or repeated tree signal is admitted, caller polling stays nonblocking, and helper cleanup remains owned without reporting ready.

#### Scenario: Unix cleanup is cancelled and resumed
- **WHEN** an async cleanup poll is cancelled while a membership worker is pending
- **THEN** the owner retains and consumes that single job on resume and never uses a late result after expiry, reap or disarm to signal the tree.

#### Scenario: Unix membership completes before the next caller poll
- **WHEN** the retained read-only worker completes helper cleanup within the caller's deadline
- **THEN** its retained completion wake advances the existing loop without waiting for the full poll timer, and only a finished worker is joined before readiness is classified.

#### Scenario: Coverage disposes an expired pending Unix owner
- **WHEN** final coverage supervision returns unconfirmed with an exited root and a pending membership helper
- **THEN** final disposal reaps only the exact exited root, keeps the unconfirmed result, and leaves helper cleanup under its read-only continuation without signalling.
