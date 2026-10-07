# project-memory-service Specification

## Purpose

Define the per-project private memory service that owns writable Dolt lifetime, validates compatible local attachments, retains service authority only while attached clients or pending work exist, and exposes exact candidate recovery without granting concurrent conversation-driving authority.

## Requirements

### Requirement: One on-demand project storage owner

For a canonical project, Kuru SHALL start or attach to at most one private memory service which owns the writable store and Dolt supervisor independently of any conversation process. Separate clients SHALL be able to attach to that service, and ending or crashing one client SHALL NOT close another client's storage attachment. The service SHALL retain its owned process and lifecycle authority through child reap. Ordinary drivers MAY concurrently drive distinct sessions only through validated session claims; one session SHALL have at most one live driver. A storage or inspection attachment alone SHALL NOT grant driving authority.

#### Scenario: Simultaneous cold starts
- **WHEN** two ordinary processes start from a cold project at the same time
- **THEN** exactly one service and one Dolt engine own that project, both processes attach to the same validated service generation and drive distinct fresh sessions, and no duplicate initialization or import occurs.

#### Scenario: Starter exits
- **WHEN** the process that started the service exits while another client remains attached
- **THEN** only the exiting client's claim and attachments end, the remaining client can read and write through its checked claim, and the engine stays owned.

### Requirement: Authenticated compatible local attachment

The service SHALL expose only private local IPC and SHALL require a bounded, versioned handshake before admitting a client. It MUST verify canonical project identity, store instance, live service generation, compatible protocol and schema, and owner-private endpoint authority. Writable clients MUST carry one authenticated client identity consistently across their driver-presence and ordinary exchange attachments. The compatible protocol floor MUST prevent older writable clients from bypassing driver checks and MUST return actionable incompatibility refusal. The service MUST reject mismatches without disclosing storage credentials or killing/replacing an active service. Endpoint metadata, PIDs, ports or a caller-chosen session string alone SHALL NOT authorize attachment, driving or takeover.

#### Scenario: Wrong project or generation
- **WHEN** a client supplies another project's identity, a stale generation or an invalid runtime connection secret
- **THEN** attachment fails before a storage operation and leaves the active owner unchanged.

#### Scenario: Active older service
- **WHEN** an updated client encounters an active service with incompatible required protocol or schema
- **THEN** it receives an actionable incompatibility result and does not terminate or replace the service.

#### Scenario: Older writable caller
- **WHEN** a client omits the driver-compatible identity or protocol while a newer owner is live
- **THEN** the owner refuses it before any private session mutation instead of granting unclaimed write authority.

### Requirement: Idle shutdown and safe recovery

After the final attachment has released and every accepted operation has settled, the service SHALL shut down immediately, without an idle interval; a newly started service first waits, within the startup budget, for the client that started it. Explicit maintenance MAY ask an owner with no other attachment to retire while retaining the starter election gate. It SHALL stop accepting and retire its endpoint, close pools, await its owned supervisor and Dolt reap, and release leases in that order. A client that arrives during shutdown SHALL either attach to the still-live generation or wait for complete owned shutdown and start or attach to a successor within its startup budget; meeting a retiring owner SHALL NOT by itself produce an error. Following crash, a successor SHALL use the existing lifecycle locks and uncertain-operation reconciliation before further mutation, and SHALL never infer authority to kill a process from a stale PID or occupied port.

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
- **WHEN** a client attaches as the last attachment releases
- **THEN** it either attaches to the still-live generation or, within its startup budget, starts or attaches to a successor after complete owned shutdown, with no simultaneous owners and no error caused only by the retiring owner

#### Scenario: Service or engine crash
- **WHEN** the service or its Dolt engine exits during an uncertain operation
- **THEN** the next owner waits for retained cleanup, reconciles the durable receipt, and does not duplicate a committed effect

#### Scenario: Maintenance after the final client exits
- **WHEN** an authorized purge starts while the previous owner is shutting down or after it has retired
- **THEN** it waits for the owner lock, retains the starter and owner gates through its directory work, and no replacement can race it

#### Scenario: Maintenance while a client remains active
- **WHEN** an authorized purge encounters a service with another live attachment
- **THEN** it refuses within a bounded deadline without retiring the owner or writing purge intent, and that client remains usable

### Requirement: Typed conditional state service operations

The private storage service SHALL expose scalar versioned, value-only batch and coherent versioned batch reads and
conditional state publication through the existing generation-bound facade.
Conditional writes MUST be classified explicitly as logical receipt-bearing
mutations. A stale response SHALL retain bounded key/expected/actual metadata as
a definite refusal distinct from uncertain transport or storage failure. The
protocol minor and exact pin SHALL change together; no session driver admission
or arbitrary SQL authority SHALL follow from these operations.

#### Scenario: Stale managed write
- **WHEN** an attached client publishes with a stale expectation
- **THEN** it receives the typed stale refusal and may issue a fresh mutation
  without an unresolved accepted-write fence

#### Scenario: Uncertain managed write
- **WHEN** a reply is lost after a conditional request was accepted
- **THEN** the facade fences subsequent writes until the exact logical unit
  outcome is reconciled, including after checked successor attachment

### Requirement: Attachment-owned immutable state cut handles

The private service SHALL bind each state cut to its attachment, owner generation
and captured live/candidate identity. Its read and close operations MUST use that
exact handle and attachment, carry no mutation receipt and reject foreign,
closed or restarted handles and crossed cursors. Disconnect SHALL release owned
cuts, and retained cuts SHALL prevent idle retirement. Protocol minor and exact
golden pin MUST advance together. Conditional publication SHALL keep its existing
typed stale refusal and exact receipt/recovery classification while validating
the full encoded request against the unchanged service envelope.

#### Scenario: Foreign or stale cut
- **WHEN** another attachment presents a handle, a cursor crosses cuts, or the owner restarts
- **THEN** the request refuses without silently reopening a new revision or granting candidate access

#### Scenario: Explicit close or disconnect
- **WHEN** the owning attachment closes a cut or disconnects
- **THEN** its retained cut resources release and later handle reads refuse while unrelated attachments remain usable

#### Scenario: Exact conditional request envelope
- **WHEN** an escaped conditional payload crosses the complete service request limit
- **THEN** local and remote preflight refuse before mutation or receipt, while the last fitting payload retains ordinary atomic publication

### Requirement: Exact context summary confirmation omits private bodies

The existing memory boundary SHALL provide a validated read-only exact context-summary identity confirmation on the caller's selected live or candidate view. It SHALL inspect retained accepted summary records independently of the current cursor-selected window and return only bounded identity/provenance metadata or absence. It MUST NOT return summary text or reasoning/source bodies, activate a writable open for inspection, change revisions/receipts/cursors, or transfer candidate mutation authority. Existing uncertain-write fences and candidate recovery/inspection ownership SHALL remain authoritative before this confirmation is used for a runtime notice.

#### Scenario: Superseded accepted summary remains confirmable
- **WHEN** an exact accepted summary identity is requested after its cursor advanced to a later summary
- **THEN** the selected view confirms the original identity/provenance without exposing its summary body or mutating storage.

#### Scenario: Candidate isolation and invalid identity
- **WHEN** a candidate-only summary is queried from live main, or malformed identity/provenance is supplied
- **THEN** main does not observe the unpromoted candidate record and invalid input is refused before effects, with unchanged revision and no leaked private body.

### Requirement: Owner-internal exact candidate reconciliation

The managed owner SHALL offer typed candidate reconciliation only through an authenticated held candidate capability, binding a new request identity to the exact branch, expected candidate head and caller-captured live head. It SHALL settle earlier uncertain work and serialize the short operation under existing owner mutation ownership. It MUST require a clean open current-schema candidate and exact current live equality to the supplied head before effects; live movement SHALL return a definite no-effect result. An accepted operation SHALL return unchanged, committed reconciliation or explicit conflict. Every accepted mutating path MUST retain exact request-bound outcome evidence; a lost reply MUST be queried without resending reconciliation. Same-generation unknown or differently bound request identities MUST remain unproved, and restart recovery MUST first prove original owner reap. Recovery SHALL validate the exact committed candidate head, parents and effective live base, issue a fresh checked handle for an open ref and preserve ambiguity. A proven noncommit MUST NOT fabricate the lost original conflict/no-op result. Existing candidate creation, promotion, abandonment and selected-ref outcomes MUST retain their exact identities. This is an internal step of checked dream publication, not a public stale-base merge command or forced promotion.

#### Scenario: Checked capability reconciles unrelated progress
- **WHEN** an authenticated candidate holder reconciles an exact clean head while ordinary live rows advanced
- **THEN** only that candidate receives the checked merge, main stays unchanged, and the response identifies the exact target and effective promotion base

#### Scenario: Foreign or changed request is refused
- **WHEN** a caller supplies a foreign handle, stale generation, changed head, non-open ref or historical schema
- **THEN** reconciliation refuses before mutation without adopting another candidate or changing live memory

#### Scenario: Exact outcome survives transport loss
- **WHEN** an accepted reconciliation reply is lost in the same generation or after owner restart
- **THEN** the original request is inspected without replay, only exact evidence clears uncertainty, and missing or unproved evidence cannot be reported as a committed merge

#### Scenario: Existing selected resolution uses updated inspection
- **WHEN** a reconciled candidate remains open for explicit resolution
- **THEN** its inspected effective base/head and fresh authenticated handle agree, and only a new exact selected abandonment request may dispose of it

### Requirement: Connection-bound session driver claims

The memory owner SHALL retain volatile session claims under its existing mutation guard, with exact client, session, claim and owner-generation identity. A dedicated retained presence connection SHALL own each claim independently of ordinary SQL exchange, candidate, dream and read-cut connections. Actual presence EOF MUST release only that connection's exact claim, and the existing final-attachment/accepted-work drain MUST trigger immediate service retirement without an idle timer. Native checked session leases MUST retain local driver exclusion across owner restart until admitted locally owned work has drained; no PID, elapsed time or lockfile deletion may reclaim them.

#### Scenario: Query replacement preserves presence
- **WHEN** an ordinary SQL exchange is cancelled or its primary stream is replaced while driver presence remains live
- **THEN** the session claim remains on its dedicated connection, the same driver continues only after its existing mutation fence is resolved, and replacement grants no new driving authority.

#### Scenario: Driver exits while a peer remains
- **WHEN** a driver closes normally or its process dies while another session remains attached
- **THEN** its actual connection loss releases only its own claim, the other session remains usable, and final-client exit later retires the endpoint and reaps Dolt before lifecycle authority is released.

#### Scenario: Owner restarts while a driver drains
- **WHEN** the memory owner exits while the original conversation process still owns admitted local work
- **THEN** ownership loss cancels later dispatch and stale private writes, the original process retains its native session barrier through checked local cleanup, and a same-session competing process cannot acquire that barrier before the drain completes.

#### Scenario: Maintenance meets a surviving driver
- **WHEN** purge or restore is requested after the memory owner exits but an original driver still retains native ownership through cleanup
- **THEN** maintenance refuses through the existing checked native/lifecycle exclusion, and a stale client's recovery cannot initialize a removed store before rejecting its old store instance.

#### Scenario: Two harnesses share an input store
- **WHEN** two Harness constructors receive clones of one local or remote memory view
- **THEN** each gets a distinct driver binding after resolving the input fence, may select a different session, and cannot share or bypass one global selected claim.

### Requirement: Exact volatile driver transition outcomes

Initial claim and session switch SHALL bind the exact authenticated client, expected old claim, target catalog expectation and request identity. The owner MUST check these inputs and target availability under the same write guard before replacing the old claim; refusal MUST leave it unchanged. Cancellation or lost reply MUST retain the full tuple and fence driver work/local publication until existing handler-completion evidence and exact selected claim establish the outcome. Unknown, mismatched, evicted, disconnected or successor-generation state MUST remain uncertain rather than infer a prior accepted or refused switch. Checked reattachment MAY transfer only a proven still-owned claim after the original handler completes, atomically changing its connection ownership so late old-connection teardown cannot remove it. Volatile transition state MUST NOT become a durable registration schema or a generic receipt framework.

#### Scenario: Refused target preserves old claim
- **WHEN** a switch selects a driven, removed or changed target or supplies a stale old claim
- **THEN** the typed no-effect refusal preserves the old claim and no target becomes selected locally.

#### Scenario: Accepted switch reply is lost
- **WHEN** the owner switches to the exact target but its reply is lost or the caller is cancelled
- **THEN** the facade retains the request tuple and required native barriers, fences further driver work, and recovers or reattaches only the exact completed selected claim without replaying a guessed switch.

#### Scenario: Late teardown after checked reattachment
- **WHEN** an abandoned presence connection ends after a checked exact claim was transferred to a fresh channel
- **THEN** the old guard cannot remove the fresh connection's claim and unrelated claims remain unchanged.

### Requirement: Honest bounded live presence

An attached compatible memory view SHALL expose a bounded complete live-claim projection containing only safe session/claim/generation observation metadata. A standalone view without live-owner evidence MUST distinguish unknown presence from a known empty set. Listing MUST NOT expose private history, provider data, credentials, PID authority or another session's permission context.

#### Scenario: Attached and cold inspection
- **WHEN** an attached inspector lists concurrent sessions and a standalone cold inspector lists their retained catalog later
- **THEN** the attached result reports known live presence, the standalone result reports unknown presence, and neither makes a provider request or grants driving authority.

### Requirement: Compatible Linux self-launch after replacement

After its installed pathname is replaced, a running Linux Kuru SHALL launch its own compatible native memory service and lifetime supervisor from its actual running executable. It MUST NOT trim a deleted pathname or silently execute the new occupant. Explicit prepared supervisor selection and checked attachment compatibility MUST remain intact.

#### Scenario: Old mapped executable starts storage after update
- **WHEN** a retained running Kuru has its installed executable atomically replaced and subsequently needs a new project memory owner
- **THEN** its checked self-launch starts its own build's service and supervisor, or refuses unproved self-image authority, and never substitutes the replacement's protocol or engine

#### Scenario: Compatible active owner remains attached
- **WHEN** the project already has a checked compatible owner
- **THEN** replacement does not terminate that owner or change the existing attachment, driver claims, private state or uncertain-operation recovery

### Requirement: Owner-served native backup

The authenticated project memory service SHALL serve one typed backup request through a dedicated attached exchange, checking canonical project/store/generation authority and request bounds before starting native capture. Accepted work MUST retain owner lifetime while ordinary claimed writers remain admitted. Backup MUST NOT expose arbitrary SQL, acquire a project maintenance permit, transfer candidate/driver capabilities or hold the ordinary mutation guard through copying or validation. Client EOF SHALL withdraw unpublished destination publication authority and settle accepted native work through its retained cleanup boundary; a reply lost after checked publication remains an uncertain caller outcome rather than authorizing replay. Protocol version, operation classification and exact wire pin MUST advance together.

#### Scenario: Concurrent attached writers

- **WHEN** a backup client captures while two ordinary clients drive distinct sessions
- **THEN** their authenticated writes continue through the same owner, whose accepted backup keeps it alive until native capture/validation cleanup settles

#### Scenario: Backup attachment disappears

- **WHEN** the dedicated backup connection reaches EOF during accepted work
- **THEN** the owner settles or retains its exact native operation and stage without deleting history, retiring a peer or publishing from cancelled authority
