# Spec Delta

## MODIFIED Requirements

### Requirement: Idle shutdown and safe recovery

After the final attachment has released and every accepted operation has settled,
the service SHALL retain its checked owner, listener and engine for
`memory.service_idle_timeout_secs` seconds, default 30, bounded from 0 through
300. Zero SHALL select immediate retirement. The idle deadline MUST use one
monotonic absolute interval starting after the final attachment task joins and
MUST NOT restart on lock rechecks. A newly accepted attachment SHALL prevent idle
retirement, and its later final detach SHALL begin a fresh interval. A newly
started service first waits, within the startup budget, for its starter.
Explicit maintenance MAY ask an owner with no other attachment to retire while
retaining the starter election gate, without waiting for idle expiry. Shutdown
SHALL stop accepting and retire its endpoint, close pools, await its owned
supervisor and Dolt reap, and release leases in that order. A client arriving
during shutdown SHALL either attach to the still-live generation or wait for
complete owned shutdown and start or attach to a successor within its startup
budget; meeting a retiring owner SHALL NOT by itself produce an error. Following
crash, a successor SHALL use the existing lifecycle locks and uncertain-operation
reconciliation before further mutation and SHALL never infer authority to kill a
process from a stale PID or occupied port.

#### Scenario: Sequential completed commands
- **WHEN** separate standalone commands complete and detach, then the next command attaches before idle expiry
- **THEN** they reuse the same checked service generation and owned Dolt engine; after the final interval expires endpoint retirement and engine reap precede lease release.

#### Scenario: Active client crosses idle interval
- **WHEN** a checked client remains attached beyond the configured idle duration
- **THEN** the owner stays available and no empty-interval deadline terminates its accepted work.

#### Scenario: Abandoned client
- **WHEN** a client crashes without explicit detach while another remains attached
- **THEN** the service reclaims only that client's attachment and remains usable for the other client.

#### Scenario: Candidate survives a lost attachment
- **WHEN** an attachment holding a candidate disconnects after an accepted candidate write but before explicit promotion or abandonment
- **THEN** the service releases the connection-owned handle without abandoning or deleting the candidate ref, leaving its branch and rows for exact-ref inspection and later explicit resolution.

#### Scenario: Dream publication follows the exact candidate outcome
- **WHEN** a dream candidate write or promotion loses its reply, including after the owner is replaced or a sibling advances live memory
- **THEN** the runtime retains the candidate and staged report, recovers the exact unit receipt and checked candidate handle before further candidate work, and publishes staged topology only from the exact promoted revision; an open or conflicted ref remains available for explicit resolution without automatic abandonment or replay.
