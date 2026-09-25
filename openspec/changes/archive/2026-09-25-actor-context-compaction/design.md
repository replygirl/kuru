## Context

P7 already sizes the exact effective provider request and omits only optional whole history rows. P29 plus its archived atomic checkpoint prerequisite add explicit session attribution, bounded sequenced source projections, and one revision-proven summary, cursor and producer-private reasoning mutation. P02 supplies the shared command registry. Compaction must join those contracts without treating summaries as deletion, notes, public transcript content or producer-private reasoning. Parallel peers write distinct actor namespaces on the same project branch; its Dolt HEAD changes even when one actor's source has not changed.

The remaining storage projection belongs to P29: a bounded local/remote live/candidate window over records currently selected by `context_summary_cursors`, optionally filtered to one exact session and source namespace. Historical summary records remain export/audit data and do not all re-enter context.

## Goals / Non-Goals

**Goals:**
- Keep each actor/session/source context useful within the effective model window while preserving every original row.
- Publish a rolling summary and its raw-source cursor atomically against the exact captured view, historical revision and unchanged consumed source.
- Account for compaction as real provider work and expose truthful automatic/manual notices.
- Preserve policy, candidate, cancellation, retry and mandatory-context authority.

**Non-Goals:**
- Deleting or expiring history, rewriting beliefs or notes, or creating a shared pool summary.
- Summarizing tool calls whose receipts are still active, opaque provider continuations, P08 reasoning records or unattributed legacy rows.
- Remote token counting, background workers, general memory search or Phase 3 developmental memory.

## Decisions

### Use a percentage trigger and a separate reserve

Add `context_compaction_threshold_percent` with a documented default of 75 and accepted range 50–95, plus `context_compaction_output_reserve_tokens` with a default of 1024 and accepted range 1–2,000,000. The effective model window must be at least the reserve. Percentage is an integer to keep TOML/schema equality and deterministic comparisons. Reusing the ordinary output reserve was rejected because summary length and user-answer length are independent decisions.

Automatic compaction is considered from the exact full policy-admitted ordinary request candidate before P7 removes optional whole history rows; otherwise P7's existing fitting step could silently hide the threshold forever. A hard overflow also attempts compaction when eligible source exists. After the one attempt, the rebuilt ordinary request still uses P7's existing omission and mandatory-material rules. Threshold selection alone grants no provider, storage or visibility authority.

### Build one bounded oldest-prefix summary request

For one actor/session/source, read the prior cursor, the prior cursor-selected summary, and the next P29 sequenced source page. Starting from the oldest row after the cursor, use the connector's pure exact-request estimator to select the largest complete prefix that fits with the fixed compaction instruction, prior summary and reserve. A row is never split. If even one row cannot fit, refuse without inference.

The new summary is rolling: the summarizer replaces the prior usable summary using that summary plus the newly selected raw prefix. The P29 cursor chain and immutable historical records preserve how it evolved. Loading every historical summary was rejected because superseded incremental summaries would grow context indefinitely and could repeat facts.

### Allow one compaction attempt per ordinary actor request

Compaction runs under a typed `ActorPhase::Compact` and `UsagePhase::Compact`, with no tools and no automatic-compaction hook inside its own request. After one settled attempt the runtime rebuilds the original request once. If mandatory content still cannot fit, it refuses. A loop that repeatedly summarizes during one dispatch was rejected because it can spend without bound and makes cancellation and exact retry harder to reason about.

### Bind provider work to the source proof before dispatch

The compaction invocation identity is derived from the real foreground operation, session, producer actor, pinned view, captured revision and exact `(after, through]` source range. Admission stores that identity before provider dispatch. An exact retry of the same operation and proof therefore retains one identity, while a later operation may retry a previously failed or cancelled unchanged source without conflicting with the earlier terminal usage outcome. The resulting operation-attributed `ContextSummaryRecord` carries no fabricated conversation turn and binds the producer actor, operation and invocation. Publication uses P29's managed idempotent `ContextSummaryCheckpoint`, which writes the summary, cursor and any returned producer-private reasoning sidecars in one receipted transaction. A changed consumed row or prior cursor/summary is a typed no-effect stale result; ambiguous transport/storage settlement stays fenced until the exact receipt reconciles without provider replay. Provider usage is admitted, observed and settled durably before this checkpoint, so a paid stale result retains its truthful charge without replay.

The checkpoint transaction proves that the captured revision is an ancestor of the pinned view's current HEAD. It reads the exact bounded consumed source rows and selected prior cursor/summary both at that historical Dolt commit and in the current transaction, comparing sequence, typed content, count and prior-summary identity/content. The historical query makes `source_revision` verifiable provenance rather than a caller-supplied label. Dolt rejects a prepared placeholder in `AS OF`, so only an exact 32-character lowercase Dolt base32 hash is admitted into that clause; namespace, session and range remain bound parameters. Relevant changes reject publication even when a row count or sequence remains unchanged. An unrelated peer's commit may advance HEAD without invalidating this proof. The existing RPC record and receipt shape stay unchanged; the short transaction performs bounded reads before the existing atomic mutation. Requiring unchanged project-wide HEAD was rejected because it serializes otherwise independent peer compactions through false stale errors.

Generating a fresh ID on retry was rejected because an accepted provider result or committed checkpoint could be duplicated.

### Revalidate policy and view at each effect boundary

Visibility and memory policy choose the actor's raw source and summary namespace before snapshot. Before dispatch, revalidate the active actor, session, namespace, view and policy decision; compare the selected prior cursor/summary and exact bounded source rows again, allowing unrelated HEAD advances. The compaction request contains only that prior summary and those selected rows. Ordinary answer context may include a policy-admitted cross-session summary projection and current-session suffix observed through separate bounded reads; its current cursor and source are rechecked before dispatch, without treating another peer's revision advance as a privacy failure. Candidate compaction uses the candidate view for snapshot, summary reads and checkpoint, and remains private until ordinary exact promotion.

Holding a SQL transaction, pool connection or runtime-wide mutation lock across inference was rejected. Inferring authority from matching namespace strings after policy change was also rejected.

### Project only cursor-selected summaries into later context

Current-session requests read raw rows strictly after the durable cursor and the one current cursor-selected rolling summary for that actor/session/source. Cross-session continuity queries P29's cursor-selected summary window only after the active policy admits that summary namespace. It never reads the historical summary table wholesale. Speaking and Compact reasoning sidecars retain their producer-private record type and are not eligible continuity rows.

### Make manual compaction explicit and sequential

`/compact [ID]` is a registry-owned local control. A named live identity compacts alone; with no argument, active parts and relationships are visited in stable topology order. Each identity gets its own source proof, provider invocation, checkpoint and notice. Cancellation drains the current invocation and prevents later identities from starting. Parallel manual summaries were rejected because stable notices, bounded provider spend and cancellation ownership matter more than latency.

## Operational surface

This slice adds no listener, bind address, container mount, service daemon, credential store or downloaded binary. Automatic compaction runs inside the existing foreground Harness immediately before an eligible actor request. Manual compaction is the registered local `/compact [ID]` TUI control and uses the current checked project, session, provider route, model and memory view. It introduces no CLI placeholder and no background job.

Existing provider authentication remains authoritative; no secret value enters a summary record, notice, status or configuration projection. Compaction uses the existing per-request provider timeout and cancellation path. Manual all-active operation is sequential, so it consumes at most one provider request at a time; automatic operation permits at most one attempt for the actor's ordinary request. P29 retains its existing memory service attachment/frame limits and pinned Dolt version/architectures. The two new numeric configuration fields are ordinary reviewed configuration and grant no file, tool, provider or memory authority.

## Integration contract

`kuru-core` owns the two configuration fields, `ActorPhase::Compact`, `UsagePhase::Compact` and budget-facing types. `kuru-connectors` owns pure estimates over the exact full pre-omission ordinary `CompletionRequest` candidate and compaction request shape; it receives already selected messages and returns sizing/prefix facts without reading memory, omitting rows or dispatching. `kuru-runtime` owns policy selection, stable invocation identity, one-attempt orchestration, accounting, cancellation and notices. `apps/kuru-tui` owns only the shared-registry command projection and rendering.

P29 owns the physical and RPC contract. P09 consumes `SessionSourceSnapshot`, `SessionHistoryWindowAfter`, `ContextSummaryCursor`, `ContextSummaryCheckpoint`, `ContextSummaryStale`, `checkpoint_context_summary`, and a bounded `ContextSummaryWindow` of records joined from current cursor identities. The window's optional exact session/source selectors and policy-authorized broader mode are not routing authority: runtime must validate actor, namespace and policy before calling. Rolling compaction always selects its exact current session and source so a bounded broader page cannot mistake omission for no prior summary; policy-authorized continuity may enumerate bounded current cursor rows only after admitting the actor and summary namespace. Summary identity is the P29 content-derived 64-byte lowercase hex ID; session/operation/invocation IDs retain their existing bounded strings, and source sequences retain the global signed 64-bit domain. No duplicate SQL schema, export scan or generic writable handle is introduced.

Deterministic provider fixtures accept the exact compaction phase/request and return bounded text plus usage. Real-Dolt fixtures run the same local/managed DTOs, candidate view and receipt reconciliation. P08 summary integration may share actor/engine call sites; Compact sidecars preserve the same producer item/output/summary coordinates but use operation provenance and enter only the atomic checkpoint, never compaction or replay context. P06 tool waves do not run inside a compaction request.

## Risks / Trade-offs

- [A rolling summary can lose detail] → Preserve all raw rows, retain exact range/revision provenance and show that originals remain stored; users can run with a higher threshold or larger reserve.
- [A relevant source or prior-summary change during inference wastes one summarization call] → Keep inference outside storage locks, reject publication with a typed no-effect stale result and charge/report the real invocation honestly.
- [Cross-session continuity leaks private raw history] → Query only cursor-selected summary records through an explicitly admitted summary namespace; never fall back to namespace-wide raw history.
- [One attempt may not rescue a very large request] → Bound cost and recursion; rebuild once and refuse hard overflow with an actionable diagnostic.
- [Candidate and live summaries diverge] → Bind every read and checkpoint to the pinned view/revision and rely on existing candidate promotion conflict handling.
