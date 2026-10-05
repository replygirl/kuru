# Spec Delta

## ADDED Requirements

### Requirement: Split topology publication ownership

The runtime SHALL store shared membership separately from per-identity reports
and session focus. Membership changes SHALL use the observed version or absence
expectation; reports SHALL preserve last-report-per-identity semantics. Session,
preference and turn checkpoints MUST retain their existing atomic publication
without rewriting membership or legacy topology. Public Topology MUST retain
all parts, relationships and reports, including archived and extra identities.
Session focus SHALL default absent for old sessions and SHALL NOT receive
ambiguous project focus. In-memory changes MUST publish only after exact durable
evidence; superseded shared state SHALL reload rather than reapply an old graph.

#### Scenario: Independent report and session writes
- **WHEN** one identity reports while another session or membership changes
- **THEN** only that identity's report is replaced and unrelated membership, reports and focus remain durable

#### Scenario: Stale membership writer
- **WHEN** seed, relate or undo publishes against an intervening membership change
- **THEN** it preserves the winner and revalidates or reports the existing explicit refusal without an unchanged membership write

#### Scenario: Candidate dream owns only its delta
- **WHEN** a dream acquires its lease, creates a candidate and reads its snapshot
- **THEN** its membership/undo/log/notes remain on that candidate, it writes no session or report rows and exact promotion/conflict recovery precedes publication

## MODIFIED Requirements

### Requirement: Coherent per-turn topology

The runtime SHALL reload and validate one versioned topology snapshot after
memory reconciliation and before each turn's actor admission. It SHALL retain
that snapshot for the complete turn and SHALL NOT mix a topology committed
during the turn into already admitted actor work. A small complete-inventory
load MAY use one coherent versioned batch, accepting only the same full
membership value and version captured by its header read. Otherwise membership,
the current session and the complete report prefix SHALL use one immutable
revision cut. Independent live pages or active-only reconstruction MUST NOT
silently omit retained reports. The existing conversation-driver lease SHALL
remain until later session admission work.

#### Scenario: Topology change appears on the next turn
- **WHEN** another accepted memory operation changes stored membership or reports after one turn captures its graph
- **THEN** the active turn completes against its retained graph and the next turn reloads the changed complete graph

#### Scenario: Large or unknown report inventory
- **WHEN** the complete retained report inventory cannot be proven to fit one small coherent batch
- **THEN** one immutable cut assembles membership, session and every report with archived and extra identities intact
