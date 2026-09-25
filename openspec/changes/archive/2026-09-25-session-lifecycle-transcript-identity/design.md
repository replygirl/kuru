## Context

The runtime currently serializes `Session` objects into one project state value and stores public messages in the general message table under a transcript namespace. A user prompt is written when a turn starts, its journal is a separate state record, and the assistant row is written at completion. The durable rows therefore preserve conversation bytes but do not bind a historical message to one turn settlement or stable speaking actor. The list blob also cannot express reversible removal, fork provenance or an atomic lifecycle mutation.

P29 adds explicit session attribution to private history and one managed store owner with typed receipts, but intentionally does not make private history a conversation transcript. P11 must preserve that distinction, keep reads and wire responses bounded, migrate legacy rows without inference, and retain the single conversation-driver lease until P30.

## Goals / Non-Goals

**Goals:**

- Give every new public turn one durable pending-to-settled lifecycle with exact typed entries and stable speaker provenance.
- Make rename, reversible removal, restoration and settled-prefix fork atomic, recoverable and independent of parent metadata changes.
- Page recent transcript and complete chronological export with bounded memory and managed frames, including fork-of-fork histories.
- Preserve readable proven legacy sessions and all ambiguous legacy bytes without inventing turn or speaker identity.
- Supply the catalog and identities P30 can later combine with live driver registrations.

**Non-Goals:**

- No private actor-history, context-summary, reasoning-sidecar or topology lineage is forked.
- No concurrent conversation admission, second surface on one session, daemon control API, provider work or inherited session-only grant is added.
- No transcript editing, secure erasure, automatic expiry or general persistent graph abstraction is introduced.

## Decisions

### Store immutable settled turns behind a versioned session catalog

Schema v7 adds a normalized session catalog and public turn records after v6 introduced operation-attributed context summaries and atomic private sidecars. The v7 migration does not reinterpret or rewrite those v6 rows, and the permanent usage registry remains independently pinned at v4. A catalog row owns the stable session ID, mode, label, durable creation/update order, lifecycle generation and state, current settled head, optional active pending node, optional validated legacy-prefix descriptor, and optional fork provenance. A primary turn row owns its session-scoped external turn ID, typed user entry, predecessor settled-turn ID, settlement kind, typed public terminal entries and stable speaker identity. A safe pre-dispatch interruption may attach the one durable marker while that row remains pending and retryable. Pending rows may make the single transition to completed or terminally interrupted; settled rows and predecessor links are immutable.

The catalog mode is current session metadata. A mode switch uses the existing receipt-bearing session checkpoint to compare the active catalog's generation and prior mode, then writes its new mode and matching runtime session/preferences/topology values in one transaction. It advances catalog update order without creating a public turn or changing lifecycle identity. Runtime publishes the new live profile only after persistence or exact reconciliation confirms both saved values and catalog mode. A project-bound memory handle must match the runtime's canonical project scope before any catalog-only resume; intentionally unbound temporary fixtures remain usable across test scopes.

Turn admission atomically inserts the pending row and user entry. A pre-dispatch interruption atomically retains its fixed marker with the journal while leaving the public row pending. Same-ID completion atomically settles that row with the retained marker and answer, without another user entry; an interruption after possible dispatch settles it terminally. When a distinct new turn supersedes a marked primary pending row, one transaction settles the old row as interrupted, advances the head and admits the new row/user entry. This preserves the current exact retry boundary rather than adding an eventually consistent transcript writer.

An abruptly dropped caller can leave that admitted pending row even though its actor work is still exiting. Only the same Harness's turn-scoped drop marker authorizes recovery: it cancels the retained invocation token, waits for a FIFO cleanup acknowledgement from every actor after its prior Work and usage settlement, then reconciles memory and checks the exact journal, pending node, session and generation before using the existing interruption checkpoint. A bounded cleanup failure retains the pending row and refuses new work. Durable pending state alone never proves an abandoned driver, and an old possible-dispatch ID remains nonreplayable after this recovery.

The raw transcript table has a global sequence but the bounded actor history exposes only a session namespace row count and suffix. Fresh runtime admission captures that count, stores its checked one-based successor in the journal, and sends the count to the existing public-turn checkpoint. The writer transaction rechecks the namespace count before any public, raw or journal write; an accepted operation receipt is reconciled before this stale-count check. The anchor survives safe retry and is optional when decoding old journals, so a later current-input rewrite can identify its original raw row without inferring from message content or a trailing interruption marker.

The ordinal is an append-only namespace coordinate. Ordinary session turns never clear raw transcript rows; if maintenance or external mutation deletes them, a consumer of the anchor must reject a projection it cannot verify against the original durable entry rather than guessing a replacement row.

Existing explicit IDs may safely resume even after intervening turns, so v7 reserves one domain-separated continuation node per original primary turn. It contains no user entry, carries a typed same-session `continuation_of` reference, and is admitted or reactivated at the current settled head under the exact checkpoint receipt before provider dispatch. The journal retains the external turn ID as retry identity; the primary and continuation have distinct stable node IDs, and every fork selector names the exact node ID so a repeated logical ID is never ambiguous. A safe interruption leaves that continuation pending without duplicating the original marker. A distinct later turn may park it while admitting its own primary node; later exact resume reactivates the same identity and updates its still-mutable predecessor to the latest head. Completion or terminal interruption settles the continuation in the ordinary chain, after which its reference, predecessor and entries are immutable. This is a bounded special case for the existing retry contract, not a reusable attempt graph.

The rejected alternative is adding nullable turn and speaker columns to unrelated raw/private message rows and continuing to derive session state from one JSON blob. That would couple public lifecycle to private memory, leave fork provenance implicit, and make rename/delete races difficult to type.

### Represent a fork as a new catalog head on the immutable turn chain

A fork validates one selected node as settled and reachable from the source session's captured head, then publishes a new catalog row whose initial head is that node. In the same checked mutation it revalidates the source lifecycle generation and selected node, and copies the source catalog's immutable legacy-prefix descriptor into the child. The child also stores source session ID, selected turn ID, source label at fork and the current-memory-sharing disclosure. Parent rename, removal or restoration changes only its catalog metadata; later parent appends create successors beyond the selected node. A fork of a fork points to the selected immutable node and inherits the same validated legacy prefix in the same way.

Reachability validation walks the plain predecessor chain on a pinned read view with bounded client memory and an overall operation deadline. Publication is one small receipt-bearing transaction that rechecks source lifecycle and the immutable selected record before inserting the child. Append-only settled heads mean a concurrent later parent turn cannot invalidate an already-proved ancestor; a concurrent source removal produces a definite refusal. Cancellation before mutation publishes nothing, and accepted lost replies use the normal logical receipt outcome.

The rejected alternatives are physically copying an unbounded prefix in one transaction, which violates the short-transaction rule, and a persistent logarithmic ancestor index, which optimizes an unmeasured rare path and adds O(log turns) rows to every completion. A plain chain is sufficient: recent views traverse only their bounded suffix, fork validation is linear but constant-memory and rare, and export has its own one-pass strategy. If measured histories later make fork validation too slow, an internal skip index can be added without changing the public contract.

### Page newest-first internally and spool export for chronological output

The store returns bounded public transcript pages from the captured head toward predecessors, with an opaque cursor, exact captured live revision, and the shared 1,024-row/32 MiB response limits. Each page retains complete turns; a single individually valid turn that exceeds the page budget is rejected at write time rather than split into an ambiguous settlement. TUI restart needs only a recent bounded suffix and reverses that page in memory.

Full export walks the same pinned chain once from newest to oldest. The CLI writes each canonical record to a private length-suffixed staging stream, then reverse-seeks frames and renders Markdown or JSONL in chronological order into the checked final publication. Memory remains bounded by one page and one record, remote frames remain bounded, and the source is never reread quadratically. A failed destination check or publication leaves no completed output and does not mutate the session.

The rejected alternatives are returning a whole transcript over one managed frame and repeatedly rescanning the chain from its head for each chronological page.

### Keep legacy bytes in place and overlay only proven projection metadata

Migration validates existing saved sessions and transcript namespace structure. Where those durable keys prove session membership and row order, the catalog records one immutable legacy-prefix descriptor containing the exact original session identity, namespace, source revision and sequence range; replay/export validates legacy rows against that original identity even after an exact descriptor is inherited by a fork. Missing speaker and turn identities remain explicitly unknown; content equality, prompt text and current topology are never evidence. New P11 turns append after that prefix. A legacy-only row with no settled turn identity is not a selectable fork boundary, while a later P11 settled node may include the complete legacy prefix behind it.

A pre-v7 safe journal can survive migration even though no original public node can be constructed without inventing a turn boundary. Resume therefore carries a narrow legacy proof: the exact immutable prefix descriptor, session-scoped journal key and expected pre-resume journal value already validated by runtime. Under the catalog/generation lock the store rechecks that descriptor and journal value, admits one domain-separated `legacy_continuation` node with no user entry or fabricated primary reference, and updates the journal in the same checkpoint. Changed, unrelated or unattributed state refuses before effect. The continuation follows the ordinary pending/parking/settlement rules and gives later terminal output a stable public boundary after the honest unknown-attribution prefix.

Rows that cannot be assigned remain unchanged and available through full-memory inspection/export and revision recovery. They are excluded from ordinary session lists and continuity, matching P29's treatment of null-attributed private rows.

The rejected alternative is rewriting every legacy message into guessed user/assistant pairs, which could assign the wrong speaker, session or settlement and could leak ambiguous rows into new conversations.

### Use typed managed operations and one central session service

The facade exposes bounded catalog/transcript reads and receipt-bearing create, rename, remove, restore and fork mutations. Local and remote backends share validators, DTOs, result types and logical receipt fingerprints; definite validation/conflict results clear only their request, while transport/storage uncertainty retains the ordinary mutation fence. Existing turn checkpoints gain the catalog/turn writes in their current transaction rather than becoming separate RPCs.

Runtime owns resume, continue and fork policy. Continue selects the highest durable update order among nonremoved sessions with session ID as a stable tie-break. The CLI and TUI call the same runtime service; slash definitions live only in the shared registry. Explicit session export is separate from full-memory export and reads no private namespaces.

The rejected alternative is implementing lifecycle separately in CLI state and TUI state or parsing string errors, which would diverge on removal, lost replies and P30 integration.

### Leave liveness and concurrent driver claims to P30

The P11 catalog contains durable history state only. It does not persist an `active` bit or infer liveness from a PID. Current writer/driver admission remains unchanged. P30 will join validated service registrations/leases to these stable session IDs, atomically claim one driver per session, expose live versus inactive status, and make a second process default to a new session.

The rejected alternative is a provisional persisted liveness flag, which would become stale after crash and create a second authority system beside the managed service.

## Operational surface

P11 runs inside the existing ordinary `kuru` binary and its authenticated same-user per-project memory service. It adds no TCP bind address, daemon installation, container, credential, provider request or externally reachable protocol. Local and managed operations keep the current 100 MiB frame ceiling; transcript pages additionally retain the shared 1,024-row/32 MiB record budget, and full export crosses that boundary only through bounded page requests and a private local staging file.

The memory service and CLI/TUI must come from one protocol-compatible Kuru build; the managed protocol minor and Dolt schema version advance together on this feature lineage. Supported OS/architecture and bundled-Dolt requirements remain the existing native macOS, Linux and Windows targets. P30 may later add more simultaneous authenticated attachments, but P11 retains the current conversation-driver lease and therefore adds no new connection-count or live-session capacity claim.

## Risks / Trade-offs

- **[Linear predecessor validation on a very long fork source]** → Keep it read-only, pinned, constant-memory and under the existing operation deadline; record elapsed/row diagnostics and add an internal skip index only from measured need.
- **[Turn completion now spans catalog, turn, journal and transcript state]** → Extend the existing one-transaction checkpoint and its lost-reply tests; publish runtime state only after typed durable settlement.
- **[A crash or safe pre-dispatch interruption leaves a pending row]** → Preserve it as pending; a safe interruption may carry its one visible marker, while only completion, a genuinely terminal interruption or atomic supersession advances the settled head. A superseded older ID reuses one receipt-bound assistant-only continuation at the latest head; never make active pending work forkable or discard its same-ID retry.
- **[Export staging can consume disk proportional to one transcript]** → Use a private checked temporary file, stream one bounded page at a time, report required space/publication failures, and clean the stage through the existing checked-output lifecycle.
- **[Legacy projection cannot recover every historical turn]** → Keep proven rows readable with unknown metadata, retain ambiguous rows in full-memory inspection/export, make unavailable legacy fork boundaries explicit, and use exact prefix+journal proof rather than inferred content when a safe pre-v7 retry needs one assistant-only continuation.
- **[P30 changes admission after P11 ships]** → Keep catalog identity and lifecycle independent of liveness so P30 adds a lease join rather than migrating transcript records again.
