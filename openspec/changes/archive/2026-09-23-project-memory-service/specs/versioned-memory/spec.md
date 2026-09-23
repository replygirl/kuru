## MODIFIED Requirements

### Requirement: Owned private server lifecycle

Kuru MUST authenticate loopback SQL from first readiness, isolate runtime config, verify store identity, bound waits and retain ownership until its child is reaped. The per-project memory service SHALL own the writable server lifetime independently of any conversation client. Service exit or crash SHALL release its server through the existing lifetime supervisor; one client's exit or crash SHALL NOT release the server while another client or accepted operation remains. Unowned ports, PIDs and held locks MUST NOT authorize destructive takeover. Writable client opens SHALL attach to the validated service and MUST NOT independently start a second writer server. Inspection MAY attach without ownership. Directory activation and recovery MUST retain the lifecycle lease through any move and revalidate directory and lock identity.

#### Scenario: Writer crash
- **WHEN** a conversation client is killed without cleanup
- **THEN** the service removes that client's attachment and continues to own its Dolt child for other clients

#### Scenario: Service crash
- **WHEN** the service itself is killed without cleanup
- **THEN** its supervisor observes lifetime EOF and reaps the owned Dolt child before another service opens the store

#### Scenario: Wrong endpoint
- **WHEN** readiness encounters wrong credentials, datadir or project identity
- **THEN** opening fails without adopting or terminating a foreign process

#### Scenario: Inspection owns the server
- **WHEN** a writable client opens while a cold inspection owns the server
- **THEN** it waits for ownership or fails within its startup deadline; inspection cleanup cannot stop a server borrowed by the writer

#### Scenario: Interrupted initializer is still running
- **WHEN** recovery finds an otherwise valid stage with an active lifecycle owner
- **THEN** it waits or fails without moving the directory until the owner has reaped its child
