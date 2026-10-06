# session-scoped-memory-provenance Specification

## Purpose
Define durable physical session attribution, bounded view- and revision-pinned
history and summary projections, atomic summary-cursor publication, coherent
per-turn topology refresh, and project-wide dream serialization for private
versioned memory.

## Requirements

### Requirement: Session-attributed private history

The memory store SHALL persist an explicit bounded session identity on each newly attributed raw private-history row while retaining one global server-assigned message sequence. Existing or imported rows without durable session evidence MUST remain present with no session attribution and MUST NOT enter a new session's continuity query.

#### Scenario: Ambiguous legacy rows survive migration
- **WHEN** a current store is upgraded with raw private rows that have no durable session evidence
- **THEN** every row, sequence, role, format and content remains available for inspection, export and recovery with a null session identity, and a session-filtered source query excludes it.

#### Scenario: Distinct sessions share an allocator but not a stream
- **WHEN** two session-bearing append operations target the same private namespace
- **THEN** the owner assigns unique globally ordered sequences and each session-filtered query returns only its own typed rows in sequence order.

### Requirement: Revision-bound sequenced source snapshot

The typed memory interface SHALL return a row- and byte-bounded source snapshot identified by actor namespace, source namespace, session identity, pinned live or candidate view, captured revision, after-exclusive sequence and derived through-inclusive sequence. It MUST stream typed rows with their durable sequences, MUST reject invalid requests, MUST stop before an aggregate response exceeds the managed-operation frame budget, and MUST NOT substitute a later revision or namespace-only history.

#### Scenario: Bounded live snapshot
- **WHEN** an actor requests the next source page after its session cursor on a live view
- **THEN** one short store operation captures the view revision and returns at most the configured row and serialized-byte bounds of matching session rows, with `through` equal to the last retained sequence.

#### Scenario: Candidate and live snapshots remain distinct
- **WHEN** the same actor and session are read through live and candidate views whose revisions differ
- **THEN** each snapshot names its actual pinned view and revision and returns only rows reachable from that captured view.

### Requirement: Atomic conditional summary checkpoint

The typed memory interface SHALL publish one strict `context_summary.v1` record and advance its cursor in one transaction keyed by actor namespace, session identity and source namespace. The mutation MUST bind the exact source view, captured revision, after-exclusive and through-inclusive range, summary namespace, settled turn and invocation identity; it MUST revalidate the complete source range under the same row and byte bounds as snapshot capture and reject stale, over-bound or conflicting input before any summary or cursor becomes visible.

#### Scenario: Exact snapshot settles summary and cursor
- **WHEN** a caller submits a valid summary for an unchanged captured source snapshot whose prior cursor equals `after`
- **THEN** the summary record and cursor become durable in the same revision and a later typed read observes both or neither.

#### Scenario: Revision or cursor moved
- **WHEN** the pinned view revision changed, another checkpoint advanced the actor/session/source cursor, or the supplied range does not match the captured source
- **THEN** a typed definite stale result is returned with no summary row, cursor movement or partial receipt effect.

#### Scenario: Accepted reply is lost
- **WHEN** the owner accepts the conditional checkpoint and the authenticated reply is lost
- **THEN** the original request identity and exact view/range fingerprint recover the committed unit outcome without replay, while an unproved outcome fences later mutation.

### Requirement: Bounded current-summary projection

The typed memory interface SHALL read context summaries only through their current cursor-selected identity for each actor/session/source tuple. The caller MUST name an actor namespace, a policy-admitted summary namespace, optional exact session and source selectors, and a bounded row limit. The result MUST retain those selectors, identify the pinned live or candidate view and revision, include the durable summary identity and strict record provenance, stream no more than 1,024 current records or 32 MiB of serialized records, and MUST NOT project superseded historical summaries except through full-memory export or audit. Rolling compaction MUST select its exact session and source so a bounded cross-source page cannot turn omission into false absence.

#### Scenario: Rolling summary replaces prompt-visible predecessor
- **WHEN** a later checkpoint advances one actor/session/source cursor while retaining the earlier summary for history
- **THEN** the bounded summary projection returns only the record named by the current cursor, while full-memory export retains both records.

#### Scenario: Policy selects current-session or shared summaries
- **WHEN** a caller requests an exact session or an existing memory policy admits the actor's shared summary namespace
- **THEN** the local or managed live/candidate view returns only matching current summaries with its exact view/revision, and no namespace-only raw history authority is implied.

### Requirement: Session-scoped runtime continuity

The runtime SHALL construct raw private actor and relationship continuity only from rows attributed to the current session and SHALL exclude null-attributed legacy rows and other sessions. It MAY include cross-session notes and context summaries only when the active mode's existing visibility and memory policy selects them. Local, remote, live and candidate reads MUST preserve the same namespace, session and pinned-view boundary.

#### Scenario: Raw private history does not cross sessions
- **WHEN** two sessions append raw or opaque provider-continuation rows to the same private namespace and one session begins a later turn
- **THEN** that turn's raw context contains only its own session rows, while full-memory inspection and export still retain both sessions and unattributed legacy rows.

#### Scenario: Policy-admitted shared memory remains visible
- **WHEN** a mode's existing memory policy selects a cross-session note or settled context summary for the current actor
- **THEN** the selected shared record may enter context without granting access to raw private history from the record's originating session.

### Requirement: Coherent per-turn topology

The runtime SHALL reload and validate one versioned topology snapshot after memory reconciliation and before each turn's actor admission. It SHALL retain that snapshot for the complete turn and SHALL NOT mix a topology committed during the turn into already admitted actor work. A small complete-inventory load MAY use one coherent versioned batch, accepting only the same full membership value and version captured by its header read. Otherwise membership, the current session and the complete report prefix SHALL use one immutable revision cut. Independent live pages or active-only reconstruction MUST NOT silently omit retained reports. Each admitted conversation turn MUST retain its checked session driver authority and existing session-private raw continuity while shared membership/reports and dreaming keep their established ownership and merge policies.

#### Scenario: Topology change appears on the next turn
- **WHEN** another accepted memory operation changes stored membership or reports after one turn captures its graph
- **THEN** the active turn completes against its retained graph and the next turn reloads the changed complete graph.

#### Scenario: Large or unknown report inventory
- **WHEN** the complete retained report inventory cannot be proven to fit one small coherent batch
- **THEN** one immutable cut assembles membership, session and every report with archived and extra identities intact.

### Requirement: Serialized cancellable dream ownership

The managed memory owner SHALL admit at most one active dream lease per project. The lease MUST span candidate inference and its explicit promotion or resolution boundary, MUST NOT hold a SQL transaction, pool connection or ordinary mutation guard across provider work, and MUST release after cancellation, client disconnect or terminal resolution so another dream can proceed. Ordinary conversation memory operations MUST remain available while the lease is held.

#### Scenario: Concurrent dreams serialize without blocking chat
- **WHEN** one dream holds the project dream lease and a second dream and an ordinary conversation write are attempted
- **THEN** the second dream waits within a bounded cancellable acquisition while the conversation write can settle, and the second dream begins only after the first lease releases.

#### Scenario: Cancellation releases dream ownership
- **WHEN** a local dream future is cancelled or a remote dream attachment disconnects while holding the lease
- **THEN** the owner releases that exact lease without replaying candidate work and a later dream can acquire it.

### Requirement: Moved-base dream conflict remains explicit

Dream publication SHALL bind the candidate's exact staged target and a proven effective live base. If live memory moves before promotion, the owner SHALL reconcile the clean open candidate with then-latest live by merging live into that private candidate, then advance live only by checked fast-forward to the exact reconciled target. Unrelated session, report, journal and history progress MUST survive. If candidate membership changed and the exact live membership token advanced from the common base, the owner MUST report an explicit membership conflict even when identical cells could merge; other overlapping row or constraint conflicts MUST also remain explicit. Conflict MUST preserve the candidate, staged report and live history, clean the private working set without advancing its head, and publish no candidate topology. Reconciliation MUST NOT force, resolve conflicts automatically, abandon, rerun inference or accept an arbitrary descendant target. Uncertain candidate writes, reconciliation and promotion MUST recover exact outcomes and fresh checked handles before further mutation or publication.

#### Scenario: Live memory moves during dream inference
- **WHEN** ordinary conversation commits after a dream captures its candidate and before exact promotion
- **THEN** checked reconciliation preserves both histories and live reports/session/journal, and successful fast-forward publishes only the exact reconciled candidate target

#### Scenario: Live membership overlaps candidate membership
- **WHEN** a candidate changes membership and live also advances that membership token from their exact base, including an equal-value overwrite
- **THEN** explicit membership conflict preserves both heads and clean candidate inspection without publishing topology or reverting the independently accepted live change through undo

#### Scenario: Dream rows remain outside another raw session
- **WHEN** reconciliation brings unattributed dream notes/tool rows below another session's captured compaction cursor
- **THEN** exact-session raw context and summary source exclude those rows, the unchanged selected range may checkpoint, and full export retains dream rows without assigning them to a session

#### Scenario: Accepted reconciliation reply is lost
- **WHEN** a reconciliation worker settles but its caller loses the reply or the owner restarts
- **THEN** exact request-bound proof recovers the committed target/effective base or honest noncommit status and a checked open handle, while ambiguous evidence preserves the fence and never authorizes replay or abandonment

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

### Requirement: Driver-bound private mutation and ownership-loss cancellation

Session-bound raw appends, checkpoints, journals/session records and compaction summaries MUST validate their current driver proof under the owner's mutation guard before an effect or receipt. A stale, absent, wrong-session or old-generation claim MUST produce a definite no-effect refusal. Driver loss MUST cancel the existing shared operation token before further provider/tool dispatch, preserve accepted durable receipt/journal recovery, and drain locally owned actors/hooks/tools before releasing the session's native exclusion. Kuru MUST NOT infer rollback, nonexecution or remote completion for an external request already accepted before loss. Such effects retain existing pending/uncertain semantics and MUST NOT be automatically replayed after another driver is admitted.

#### Scenario: Owner loss during provider or shell work
- **WHEN** actual presence or owner-generation loss occurs while provider or owned shell work is admitted
- **THEN** no later tool/provider dispatch is admitted under the old claim, stale private mutation refuses, existing cancellation and checked local cleanup run while native exclusion remains held, and already accepted external effects remain honest journal outcomes.

#### Scenario: Concurrent private sentinels and shared policy memory
- **WHEN** two actual processes concurrently drive different sessions containing distinct raw/opaque actor and relationship sentinels plus policy-approved notes and summaries
- **THEN** each provider context contains only its own raw sentinels and its mode's approved shared records, including after shared dreaming and undo, while full inspection retains both histories.

#### Scenario: Accepted checkpoint precedes claim loss
- **WHEN** a checkpoint is accepted and its reply is lost before the claim or owner ends
- **THEN** exact receipt recovery establishes that checkpoint once, stale replay is refused, and subsequent publication or interruption never guesses nonexecution from claim loss.
