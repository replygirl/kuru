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

The owner SHALL retain the engine while any attachment remains. After the last attachment and accepted work settle, it SHALL wait 30 seconds before stopping; it SHALL close the store and reap Dolt before retiring its endpoint and owner lock. A new attachment during the idle interval SHALL cancel retirement.

#### Scenario: Last client disconnects
- **WHEN** the last attached client disconnects and no accepted operation remains
- **THEN** the endpoint and owner lock retire only after the idle interval and owned Dolt cleanup
