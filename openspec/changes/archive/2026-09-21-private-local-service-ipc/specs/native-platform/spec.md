## ADDED Requirements

### Requirement: Private successive-client service IPC

The native platform package SHALL provide a local listener that accepts successive independent clients on Unix and Windows without granting service, database or process authority to the transport. Unix socket creation and connection MUST require a checked owner-private directory and an owner-private socket of the expected type; a listener MUST NOT adopt or remove an occupied name. Windows named-pipe instances MUST retain a private local-only DACL, and cancelling an accept MUST preserve the pending native operation so a later accept can complete safely. Releasing a listener SHALL remove only its own checked Unix socket name. The consuming service MUST separately authenticate every connection; native permissions do not isolate processes running as the same OS user.

#### Scenario: Successive independent clients
- **WHEN** two clients connect to one native listener in succession
- **THEN** both obtain usable byte channels and the first client's lifetime does not block the second

#### Scenario: Cancelled Windows accept
- **WHEN** a Windows service listener's bounded accept times out before a client connects
- **THEN** a later accept can complete the original pending connection without discarding kernel-owned buffers or stranding the listener

#### Scenario: Unsafe Unix endpoint
- **WHEN** a Unix caller supplies a public parent, a non-socket endpoint or a socket with group/other access
- **THEN** binding or connecting rejects the endpoint without adopting it as private service IPC

#### Scenario: Occupied or replaced Unix name
- **WHEN** the chosen Unix socket name already exists or the bound name is replaced before listener drop
- **THEN** binding rejects the occupied name and cleanup does not remove the replacement
