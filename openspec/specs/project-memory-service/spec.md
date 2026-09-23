# project-memory-service Specification

## Purpose

Define the per-project private memory service that owns writable Dolt lifetime, validates compatible local attachments, retains idle service authority safely, and exposes exact candidate recovery without granting concurrent conversation-driving authority.

## Requirements

### Requirement: One on-demand project storage owner

For a canonical project, Kuru SHALL start or attach to at most one private memory service which owns the writable store and Dolt supervisor independently of any conversation process. Separate clients SHALL be able to attach to that service, and ending or crashing one client SHALL NOT close another client's storage attachment. The service SHALL retain its owned process and lifecycle authority through child reap. A service attachment SHALL NOT grant permission to drive a second concurrent conversation until session-private history isolation is enabled.

#### Scenario: Simultaneous cold starts
- **WHEN** two processes start from a cold project at the same time
- **THEN** exactly one service and one Dolt engine own that project, both processes attach to the same validated service generation, and no duplicate initialization or import occurs

#### Scenario: Starter exits
- **WHEN** the process that started the service exits while another client remains attached
- **THEN** the remaining client can read and write through the service and the engine stays owned

### Requirement: Authenticated compatible local attachment

The service SHALL expose only private local IPC and SHALL require a bounded, versioned handshake before admitting a client. It MUST verify canonical project identity, store instance, live service generation, compatible protocol and schema, and owner-private endpoint authority. It MUST reject mismatches without disclosing storage credentials or killing/replacing an active service. Endpoint metadata, PIDs and ports alone SHALL NOT authorize attachment or takeover.

#### Scenario: Wrong project or generation
- **WHEN** a client supplies another project's identity, a stale generation or an invalid runtime connection secret
- **THEN** attachment fails before a storage operation and leaves the active owner unchanged

#### Scenario: Active older service
- **WHEN** an updated client encounters an active service with incompatible required protocol or schema
- **THEN** it receives an actionable incompatibility result and does not terminate or replace the service

### Requirement: Idle shutdown and safe recovery

After the final attachment and accepted operation settle, the service SHALL wait a documented idle interval before shutdown; a valid reconnect during that interval SHALL cancel shutdown. Explicit maintenance MAY ask an otherwise idle owner to retire early while retaining the starter election gate. It SHALL close pools, await its owned supervisor and Dolt reap, retire its endpoint, and release leases in that order. Following crash, a successor SHALL use the existing lifecycle locks and uncertain-operation reconciliation before further mutation, and SHALL never infer authority to kill a process from a stale PID or occupied port.

#### Scenario: Abandoned client
- **WHEN** a client crashes without explicit detach while another remains attached
- **THEN** the service reclaims only that client's attachment and remains usable for the other client

#### Scenario: Candidate survives a lost attachment
- **WHEN** an attachment holding a candidate disconnects after an accepted candidate write but before explicit promotion or abandonment
- **THEN** the service releases the connection-owned handle without abandoning or deleting the candidate ref, leaving its branch and rows for exact-ref inspection and later explicit resolution

#### Scenario: Dream publication follows the exact candidate outcome
- **WHEN** a dream candidate write or promotion loses its reply, including after the owner is replaced or a sibling advances live memory
- **THEN** the runtime retains the candidate and staged report, recovers the exact unit receipt and checked candidate handle before further candidate work, and publishes staged topology only from the exact promoted revision; an open or conflicted ref remains available for explicit resolution without automatic abandonment or replay

### Requirement: Reachable exact-ref candidate resolution

The managed owner SHALL expose a bounded read-only inventory and status for validated exact Kuru candidate refs through the existing project memory command boundary, including after the initiating client exits. Status MUST distinguish a proven open ref with unchanged live base, a proven open ref whose live base moved, an unsettled or otherwise uncertain transition, and a resolved or missing ref without inferring that an earlier accepted write never committed. CLI and TUI command discovery and dispatch MUST expose the same checked inspection and explicit abandonment route through their existing command systems. Explicit abandonment SHALL bind a new logical request identity to the selected exact ref, inspected base and head, exclude new attachments during its owner-authorized recheck and transition, settle accepted candidate writes and use typed transition proof before any checked ref transition. Changed, active, historical-stale, ambiguous or unverified refs MUST remain intact with an actionable refusal. Inspection MUST NOT dispatch a mutation. This route SHALL NOT automatically retry candidate creation or promotion, abandon on attachment loss, or offer a public stale-base merge or forced promotion.

#### Scenario: Candidate remains inspectable after the client exits
- **WHEN** an initiating dream client exits with a retained open or conflicted candidate
- **THEN** a later memory inspection lists its exact validated ref and checked status without changing its head, private rows or live revision

#### Scenario: Explicit selected abandonment
- **WHEN** a user selects a proven open candidate and confirms its exact inspected ref and head for abandonment
- **THEN** the owner settles any accepted work, rechecks the same ref and head, durably records only that candidate's explicit abandonment, and preserves unrelated candidates and live history

#### Scenario: Selected abandonment loses its reply
- **WHEN** the owner accepts exact selected abandonment but the client loses its reply, including across owner restart
- **THEN** the client retains its new request identity and exact ref/base/head, queries typed transition outcome without resending abandonment or fabricating a creation identity, and reports success only for proved Abandoned; unsettled or conflicting outcomes preserve the ref and mutation fence

#### Scenario: Selected abandonment is cancelled before dispatch
- **WHEN** the caller is cancelled after retaining the selected abandonment intent but before a transition request is sent
- **THEN** read-only inspection may restore explicit selection only for the checked still-open exact ref; missing or changed evidence stays uncertain, and no abandonment is replayed automatically

#### Scenario: Empty candidate outcome remains unproved after reclamation
- **WHEN** an empty candidate has the same target and base, its abandonment reply is lost, and its exact refs have been reclaimed
- **THEN** absence is reported separately from an extant conflict, but neither inspection nor closing and reopening proves the abandonment request succeeded; its pending caller remains fenced until typed proof is available

#### Scenario: Unknown same-generation request cannot claim reclaimed abandonment
- **WHEN** a caller queries a reclaimed candidate using an unregistered or differently bound selected-abandon request ID while the original owner generation is still live
- **THEN** the absence remains uncertain; only a completed handler for the exact ID and branch/base/head tuple or verified prior-owner reap can settle that inference

#### Scenario: Uncertain or changed candidate is preserved
- **WHEN** an earlier transition remains uncertain, a ref or head changes after inspection, or an active candidate view prevents safe resolution
- **THEN** explicit abandonment refuses without replaying promotion or deleting any candidate ref, and reports the unresolved state for later exact inspection

#### Scenario: Reconnect races idle shutdown
- **WHEN** a client reconnects as the idle interval expires
- **THEN** it either attaches to the still-live generation or safely starts/attaches to a successor after complete owned shutdown, with no simultaneous owners

#### Scenario: Service or engine crash
- **WHEN** the service or its Dolt engine exits during an uncertain operation
- **THEN** the next owner waits for retained cleanup, reconciles the durable receipt, and does not duplicate a committed effect

#### Scenario: Maintenance after the final client exits
- **WHEN** an authorized purge starts during the warm idle interval with no other attachment
- **THEN** the owner retires and reaps early, purge retains the starter and owner gates through its directory work, and no replacement can race it

#### Scenario: Maintenance while a client remains active
- **WHEN** an authorized purge encounters a service with another live attachment
- **THEN** it refuses within a bounded deadline without retiring the owner or writing purge intent, and that client remains usable
