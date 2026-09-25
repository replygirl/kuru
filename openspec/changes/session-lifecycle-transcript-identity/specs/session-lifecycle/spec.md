## ADDED Requirements

### Requirement: Durable session catalog and reversible lifecycle

The memory store SHALL maintain a versioned catalog for every canonical-project session with stable identity, mode, label, creation and update order, lifecycle state and exact fork provenance. Rename, removal and restoration MUST be receipt-bearing checked mutations. Removal MUST exclude a session from ordinary listing, continue, picker and resume without deleting or rewriting its public transcript, private actor memory, summaries, notes, usage or version history; restoration MUST make the same retained session resumable again.

#### Scenario: Remove and restore a session
- **WHEN** a caller removes a stored session and later restores its exact retained identity
- **THEN** ordinary listings and resume refuse it while removed, restoration returns the same label, transcript and provenance, and no project memory or historical row is erased.

#### Scenario: Accepted lifecycle reply is lost
- **WHEN** the owner accepts a rename, removal or restoration and its authenticated reply is lost
- **THEN** the retained request identity recovers the one durable outcome without replay, while an unproved outcome fences later mutation rather than being reported as success.

### Requirement: Settled-turn public transcript identity

Each new public turn SHALL carry a bounded session-scoped turn identity, an ordered typed user entry, one pending/completed/interrupted settlement state and, when settled, the exact stable speaker identity and typed public terminal entries. The pending admission and user entry MUST become durable together. Completion or a terminal interruption after possible dispatch MUST settle the turn and advance the session's public head atomically. A safe pre-dispatch interruption MUST retain its one visible durable marker on the pending turn so the same logical turn can complete without another user entry; it MUST remain distinct from a settled answer or fork boundary. A settled turn and its predecessor MUST thereafter be immutable.

Fresh admission SHALL bind its journal to the exact one-based raw transcript user-row position: the store MUST check the caller's captured namespace row count under the same transaction that writes the pending public turn, raw user entry and journal, refusing a changed count without effect. Safe retries SHALL retain that anchor through interruption markers and intervening turns. Historical journals without a recoverable anchor remain readable, but a consumer MUST NOT guess a raw user row for a changed-input projection.

If a distinct later turn supersedes that retryable pending turn, the store MUST settle the marked turn as interrupted before admitting the later user entry. A later exact retry of the older ID MUST use one typed assistant-only continuation node that references the original same-session interrupted node, appends no user/raw row, and attaches to the then-current settled head. The continuation identity and every pending/settled transition MUST remain receipt-bound so reply loss or exact retry cannot duplicate dispatch or transcript content. The original journal/external turn ID MUST remain the retry identity, while the continuation admission uses its own domain-separated stable node identity. Fork selection MUST use the exact stable node ID rather than an external turn ID that can name both the original and continuation. A pending continuation MAY update its predecessor when reactivated after another superseding turn; once settled, its reference, predecessor and content MUST be immutable. This specialized continuation remains part of the same plain predecessor chain and MUST NOT introduce private-history lineage or a general attempt graph.

#### Scenario: Completed and terminally interrupted turns settle once
- **WHEN** a turn durably completes or an interruption after possible dispatch becomes terminal
- **THEN** its exact public entries, settlement kind, speaker identity and predecessor are published once, survive restart and cannot later change from completed to interrupted or vice versa.

#### Scenario: Safe pre-dispatch interruption remains retryable
- **WHEN** interruption wins before possible dispatch and commits the fixed marker
- **THEN** the marker remains visible and idempotent on the pending public turn, fork through it refuses, and the same turn ID may later settle completed with that marker and the answer but no second user entry.

#### Scenario: Older safe ID completes after later turns
- **WHEN** a distinct turn settles after a safe interrupted ID and that older exact ID is later resumed
- **THEN** the original interruption prefix remains immutable, one assistant-only continuation attaches after the intervening settled head, completion appends no user/raw row, and exact retry or lost-reply recovery creates no second continuation or dispatch.

#### Scenario: Retryable continuation is superseded and resumed
- **WHEN** an assistant-only continuation is safely interrupted, a distinct later turn supersedes it, and the older ID is resumed again
- **THEN** the same pending continuation identity is reactivated at the latest settled head without another user row or interruption marker, and only its eventual terminal settlement becomes immutable.

#### Scenario: Pending work remains pending
- **WHEN** a process stops after the user entry is durable but before a completion or interruption settlement is proved
- **THEN** inspection reports the pending turn honestly, export does not invent a speaker or answer, and fork through that turn refuses without changing either session.

### Requirement: Honest legacy transcript migration

The v7 migration SHALL retain every legacy transcript and session byte and MUST leave v6 context-summary, cursor and private-reasoning provenance unchanged. It MAY associate a row with a session and ordering only from validated durable namespace, catalog or record evidence, and the immutable prefix descriptor MUST retain the exact original session identity with its namespace, source revision and sequence range. A fork MUST copy that descriptor exactly and validate projected rows against the retained original identity rather than the child identity. Missing historical speaker or turn identity MUST be labeled `unknown` rather than inferred from content or current topology. Proven legacy session rows MUST remain visible on resume and session export; unassignable rows MUST remain reachable through existing full-memory inspection and export and MUST NOT enter every new session.

A validated pre-v7 safe journal whose proven session transcript is represented only by an immutable legacy-prefix descriptor MUST remain retryable. Its resume checkpoint MUST bind the exact catalog generation, unchanged legacy-prefix descriptor, session-scoped journal key and expected pre-resume journal value in the same transaction. The store SHALL admit one domain-separated assistant-only legacy continuation with no fabricated primary-node reference or user row. It MUST refuse unrelated, changed or unattributed journal state without effect, and its eventual terminal entry SHALL follow the proven legacy prefix while the original bytes and unknown historical turn boundaries remain unchanged.

#### Scenario: Legacy speaker is unknown
- **WHEN** a saved session has ordered transcript rows but no durable historical speaker evidence
- **THEN** the upgraded session remains readable in the same order with unknown speaker provenance, and current actor names are not substituted.

#### Scenario: Ambiguous legacy data remains accessible
- **WHEN** a retained row cannot be assigned to a session from durable evidence
- **THEN** migration preserves it for full-memory inspection, export and recovery while excluding it from ordinary resume, continue and fork projections.

#### Scenario: Proven legacy safe journal resumes without inferred history
- **WHEN** a migrated session has an exact safe pre-dispatch journal but no v7 primary node and the same journal is resumed
- **THEN** one legacy continuation atomically binds the proven prefix and expected journal state, appends no user row, and later completion is visible after the unknown-attribution prefix without changing any legacy byte.

### Requirement: Stable public-prefix fork

Fork SHALL create a new session whose initial public transcript is the immutable prefix ending at one exact completed or terminally interrupted source turn. Publication MUST validate that the selected turn is settled and reachable from the source session, recheck the source lifecycle generation, copy the source catalog's exact immutable legacy-prefix descriptor when present, record parent session and turn provenance, preserve the parent, and atomically publish the child catalog entry. The child MUST share current project notes, policy-admitted summaries and topology while copying no raw private actor or relationship history; every fork result and later resume MUST disclose that project memory was not rewound.

#### Scenario: Parent and fork diverge after a settled answer
- **WHEN** a session is forked through a completed turn and both parent and child later receive turns
- **THEN** both show the same stable selected prefix followed by independent public suffixes, later parent writes never enter the child, and neither session's raw private history enters the other.

#### Scenario: Interrupted boundary and pending refusal
- **WHEN** a caller selects a terminally interrupted turn or a pending turn carrying a safe pre-dispatch marker
- **THEN** the terminal interruption marker is a valid final prefix entry, while the retryable pending selection returns a definite no-effect refusal and publishes no child.

#### Scenario: Fork of fork survives parent lifecycle changes
- **WHEN** a fork is itself forked and an ancestor is later renamed, removed and restored
- **THEN** both descendant prefixes and recorded source identities remain unchanged and readable, with no recursive dependency on the ancestor's current listing state.

#### Scenario: Cancelled or lost fork publication
- **WHEN** cancellation occurs before fork mutation acceptance or the accepted reply is lost
- **THEN** pre-acceptance cancellation publishes no session, while accepted recovery proves exactly one complete child or retains an uncertainty fence; no partial child is listable or resumable.

### Requirement: Bounded pinned transcript projection

The typed local and managed memory interface SHALL expose row- and byte-bounded session catalog and public transcript pages with exact session, view, captured revision, ordering and opaque continuation identity. A page MUST preserve typed blocks and speaker/turn settlement metadata and MUST NOT include raw private histories, private reasoning summaries, context summaries, notes, candidates or another session. Page continuation MUST remain on the captured revision or fail explicitly rather than mix revisions.

#### Scenario: Long transcript pages on one revision
- **WHEN** a session contains more rows or bytes than one managed response permits
- **THEN** bounded pages cover its exact public records once in stable order at one revision, and a concurrent later append is absent from that captured traversal.

#### Scenario: Session projections remain isolated
- **WHEN** two sessions contain distinct typed sentinels and their actors also retain private rows and summaries
- **THEN** each public transcript page contains only its own public entries and no private or sibling sentinel.

### Requirement: One-session Markdown and JSONL export

Session export SHALL stream one selected session's complete captured public transcript in chronological order as Markdown or strict JSONL, with manifest provenance, session and fork identity, turn settlement, stable speaker identity and canonical typed content. Assistant-only continuation records MUST retain their original-turn reference without fabricating or repeating a user entry. Export MUST remain separate from full-memory export, use bounded memory and checked output publication, and MUST NOT silently omit an active pending or legacy entry; such entries SHALL be represented with their honest status and unknown fields.

#### Scenario: Export a long forked session
- **WHEN** a caller exports a fork whose transcript exceeds one page and contains text, tool and interruption content
- **THEN** both formats contain the stable shared prefix and child suffix exactly once in chronological order, identify the source fork and current-memory sharing, and decode without private-history rows.

#### Scenario: Output publication fails
- **WHEN** a selected output path changes identity or publication fails after export traversal begins
- **THEN** no partial final export is presented as complete, the source session is unchanged and bounded staging is cleaned or reported for recovery.

### Requirement: Deterministic resume and continue

Exact resume SHALL accept only a stored nonremoved session in the canonical project. `--continue` SHALL select the most recently updated nonremoved session by durable catalog order, with a stable identity tie-break, and SHALL fail clearly when none exists. Every new process invocation, including resume, continue and a forked session, MUST start a new permission-grant session and MUST NOT inherit session-only grants. P11 MUST retain the current single conversation-driver admission until P30 installs validated live-session claims.

#### Scenario: Continue chooses the durable latest session
- **WHEN** multiple stored sessions exist, including a newer removed session
- **THEN** continue deterministically resumes the most recently updated ordinary session and does not restore or select the removed session.

#### Scenario: Resume does not inherit authority
- **WHEN** a fresh process resumes or continues a session whose previous process held session-only tool permission
- **THEN** the transcript and session mode resume but the permission requires fresh admission under the new process.

### Requirement: Consistent CLI and TUI session management

CLI session commands and the TUI picker SHALL use the same typed catalog and mutation results for list, rename, remove, restore, fork, resume, continue and export. The picker MUST distinguish ordinary and removed sessions, expose settled fork boundaries, preserve the current draft until a lifecycle action succeeds, and provide actionable pending, missing, removed and uncertain diagnostics without making a provider request.

#### Scenario: Picker and CLI observe the same lifecycle
- **WHEN** a session is renamed, removed, restored or forked through either surface and the other surface reopens the project
- **THEN** both show the same identity, label, status, provenance and transcript and neither lifecycle action invokes a provider.

#### Scenario: P30 admission remains absent
- **WHEN** another ordinary conversation driver already owns the project under the current P11 runtime
- **THEN** session inspection remains available where already permitted, but P11 does not claim a live-session lease, admit a second driver or attach a second surface to an existing session.
