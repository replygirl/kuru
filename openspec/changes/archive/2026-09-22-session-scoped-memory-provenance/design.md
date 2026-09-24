## Context

The current schema has one globally auto-incremented `messages.sequence`, a namespace, role, content format and payload. Version 4 adds indexed durable operation receipts but has no physical session column. Every managed local, remote, live and candidate view already carries a pinned branch and reconciles accepted unit mutations by request fingerprint.

P09 needs a real source boundary before it can compact actor context: a session-filtered sequence page captured at one view revision, followed by a conditional summary-and-cursor publication against that exact proof. Existing namespace-only history and generic state upserts cannot provide either guarantee.

## Goals / Non-Goals

**Goals:**

- Add session provenance without rewriting or guessing legacy history.
- Give runtime policy code a bounded typed snapshot and one atomic conditional checkpoint through local and remote views.
- Reuse exact-view service receipts so cancellation and lost replies retain the existing definite-versus-uncertain boundary.
- Build actor context from current-session raw history plus only policy-admitted shared notes and summaries.
- Reload one coherent topology before each turn and retain that snapshot through the turn.
- Serialize dream work across a managed owner without holding database resources or blocking ordinary chat, and preserve explicit promotion conflicts.
- Keep store locks and SQL transactions shorter than provider work.

**Non-Goals:**

- Enabling a second conversation driver or changing project/session admission.
- Persisting P08 provider reasoning rows as compaction continuity.
- Session resume/fork UI or a general historical-query API.

## Decisions

### Add a nullable session column without changing the allocator

Schema 5 adds nullable `messages.session_id` and an index over namespace, session and sequence. Explicit session-bearing append APIs require a bounded nonempty identity; existing rows and compatibility append/import paths retain null. The primary global auto-increment sequence remains the only allocator.

Rewriting legacy rows or assigning the current session was rejected because namespace and process timing are not durable attribution evidence. A per-session allocator was rejected because source ranges, export ordering and concurrent append receipts already rely on one stable sequence domain.

### Store summaries and cursors in strict current-schema tables

Schema 5 adds typed context-summary and cursor tables. A summary retains its actor/session/source key, summary namespace, source view and revision, exclusive/inclusive sequence range, settled turn/invocation identity, fixed `context_summary.v1` format and bounded text. The cursor table has one row per actor/session/source and points to the last settled range and summary identity.

Encoding these records as generic state values was rejected because it would hide identity, format and referential validation behind untyped JSON. Encoding them as ordinary raw history was rejected because summary continuity has different visibility and session rules and must never be mistaken for source dialogue.

### Capture one bounded next page, then conditionally settle that page

The snapshot request names actor, source and session plus an exclusive cursor and a 1-through-1024 row limit. Under the store's short mutation/read serialization boundary it captures the pinned branch revision and streams the next matching rows in global sequence order. Collection stops before the exact serialized rows exceed 32 MiB, leaving fixed metadata well below the 100 MiB managed-operation frame. The returned last retained row defines `through`; an empty page has no publishable proof.

The checkpoint accepts that immutable proof and a strict summary record. While holding the ordinary store mutation guard, it verifies the view and current revision, verifies the durable cursor still equals `after`, and streams the complete same-session/source range from `after` through `through` with the identical 1,024-row and 32 MiB ceilings. Only then does it insert the summary, advance the cursor, record the logical receipt and commit once. Provider inference happens between snapshot and checkpoint with no held transaction, connection or mutation guard.

A long-lived SQL snapshot and a generic caller-selected revision query were rejected because they would retain database resources through inference or expose historical read authority beyond this source contract.

### Keep stale rejection typed and pre-effect

Revision, cursor, view and source-range checks complete before inserts. Their one domain error is a definite no-effect response and is reconstructed across RPC. Errors after mutation dispatch or incomplete transport remain ordinary storage uncertainty, retain the request fence and require receipt outcome reconciliation.

String parsing and treating reconnect as success were rejected because both collapse the service's authenticated definite-versus-uncertain boundary.

### Preserve current admission and separate later P29 work

The current conversation driver lease remains required. P30 owns default concurrent process/session admission; P29 does not weaken that gate while adding the storage and runtime boundaries needed underneath it.

### Read raw history by session and keep shared continuity policy-controlled

Runtime actor context reads raw private history through a bounded typed session-history operation. The request names the exact namespace and current invocation session, and local, remote, live and candidate views return the same newest suffix and total count. Namespace-only history remains an inspection/export compatibility surface and is not used for new-session continuity.

Notes and context summaries retain their distinct visibility contracts. Cross-session rows enter a prompt only through the existing mode and memory-policy selection for that row type; session filtering raw history does not grant broader shared-memory authority. P09 owns summary production and selection details, while this change supplies the durable source/checkpoint contract and consumes only summaries that policy has admitted.

The summary read joins `context_summary_cursors` to `context_summaries` by the complete actor/session/source coordinate and durable summary identity. It returns only the current rolling summary per source, never superseded incremental records. The caller names an exact actor and admitted summary namespace, may restrict one session and source for rolling compaction or request a policy-authorized bounded projection across either coordinate, and receives those selectors, the pinned view/revision and full record provenance under the same row and byte response bounds as source snapshots.

### Reload one topology snapshot per turn

After reconciling pending memory operations and before admitting actor work, the runtime reloads the stored topology and validates its namespaces/profile. That value replaces the previous in-memory topology once and is retained throughout the turn, so every actor and relationship in one turn observes one coherent graph. A topology committed during the turn becomes visible on the next turn rather than partially changing the active pool.

Holding a long-lived database snapshot was rejected because the topology is already one versioned stored value and provider work must not retain storage resources.

### Serialize dreams with a separate owner lease

Dream inference acquires a dedicated managed lease before creating or using its candidate and retains it until the candidate is promoted, explicitly abandoned, left open for resolution, or the operation is cancelled. The lease is independent of the store mutation lock, SQL pools and the conversation-driver lease, so ordinary chat can continue while one dream performs provider work. Remote ownership binds the lease to one authenticated attachment; disconnect or cancelled acquisition releases it without killing or electing an owner from a stale endpoint.

Promotion still uses the candidate's captured base and exact target. If live memory moved, the owner returns the existing typed conflict, retains the candidate/report and current history, publishes no stale topology, and requires the explicit candidate-resolution flow. Kuru never reruns provider inference or force-merges the candidate automatically.

## Risks / Trade-offs

- [Schema indexes over long binary namespaces approach engine key limits] → Keep every indexed tuple below the pinned Dolt/MySQL byte limit, validate the exact schema through real migration fixtures and use fixed-size summary identities where a composite key would be too wide.
- [A caller summarizes a stale page after provider work] → Make revision and prior cursor authoritative and return a definite no-effect stale result; the caller may select again but never silently publishes against a different source.
- [Null session rows become invisible to ordinary new-session context] → Preserve them in full inspection, export and recovery, and document that continuity exclusion is intentional rather than deletion.
- [P29 is mistaken for complete concurrent-session admission] → Keep the driver lease unchanged and leave only P30 admission explicitly deferred.
- [A session filter accidentally hides intentionally shared continuity] → Keep notes and summaries on their existing policy-selected paths and filter only raw private histories by session.
- [Topology changes split one turn across graph versions] → Reload once before admission and retain that validated topology through the turn.
- [Dream serialization blocks chat or leaks on cancellation] → Use a separate attachment-bound owner lease with no SQL transaction or mutation guard spanning inference, and prove ordinary writes and disconnect release while it is held.
