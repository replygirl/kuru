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
`ECHILD` or another ownership-invalidating error, signal the original group and
root only while the root is unreaped, consume signal authority when destructive
attempts begin, and permit only read-only group observation after reap.

The Unix boundary MUST NOT claim that a process group provides atomic
owner-death containment or controls processes that deliberately escape it.
Windows cancellation and close MUST observe root reaping and actual owned-tree
quiescence before reporting completion. A Unix ordinary shell result MUST
observe root reap and group absence; a bounded cleanup failure MAY return an
unconfirmed result only while an independent owner retains the child and
associated capability until later confirmation. Stale PIDs MUST NOT authorize
termination on any platform.

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
- **THEN** platform signals the original group then root while retaining the unreaped standard child, reaps the exact root status, and performs no destructive numeric signal afterward.

#### Scenario: Unix child ownership cannot be established
- **WHEN** non-reaping observation reports `ECHILD` or another ownership-invalidating error before destructive cleanup begins
- **THEN** platform permanently disarms signal authority, reports the distinct ownership state, and never blindly signals the saved numeric group or root.

#### Scenario: Unix observation remains interrupted beyond caller return
- **WHEN** bounded `EINTR` exhausts an observation attempt while the exact root remains owned and unreaped
- **THEN** platform sends nothing and retains anchored authority so a later valid observation can perform the one destructive transition; once that transition begins, no later poll can signal again.
