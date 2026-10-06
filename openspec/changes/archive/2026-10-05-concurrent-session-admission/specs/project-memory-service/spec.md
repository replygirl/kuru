# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
