# Design

## Context

The approved W7 topology design section 6 and its independent design review section 1 select a private candidate merge followed by exact fast-forward. B already branches before snapshot/inference (`runtime/dream.rs:97,251`) and writes only membership/undo/last-dream/log/notes on that candidate. Raw source readers and compaction validation use exact session filters (`memory/store.rs:2557,3299,7393,7467`); `runtime/actor.rs:953` separately admits policy-selected current summaries without granting foreign raw history.

The present store `Candidate` holds `base: String` (`store.rs:842`) and promotion refuses `current != base` (`:1090,1117`). Reattachment derives the common ancestor (`:4072`), but facade transition recovery requires that returned base equal the remembered pending base (`facade.rs:842`). Those invariants must adopt a proven reconciliation base coherently; simply adding a merge call would fail promotion or invalidate recovery. Store uncertainty receipts are process-local (`store.rs:356`), while accepted remote outcome queries require original request/generation proof. Existing session teardown, checked ref transitions and worker lifetime remain owning boundaries.

## Goals / Non-Goals

The proposal/spec deltas define behavior. Design goals are to retain exact historical proof while updating the effective promotion base, use the existing concrete store/worker/receipt machinery, and keep policy reasoning outside SQL mechanics. There is no admission change, generic object/field merge, schema migration, new prompt authority or alternate provider runtime. Existing notes/summaries policy and unattributed dream append provenance remain unchanged.

## Decisions

### Merge only into the private candidate

The client reads live `L` through the existing revision getter and invokes `Candidate::reconcile_with_live(expected_head, expected_live)`. Hold the owner's ordinary mutation mutex only for the short reconcile worker after earlier uncertainty settles. Validate exact open branch/head, clean working set and current-schema parity, then require committed main still equals supplied `L`. A moved live head returns definite `LiveMoved` before effects; the existing bounded publication attempts may sample again. Compare exact immutable row snapshots. If `L` is already an ancestor of candidate, return unchanged without writing. Otherwise merge the exact supplied revision `L` into the candidate using the owned branch connection and existing operation/query budgets, default conflict sysvars and owned session-completion discipline. Live never becomes a conflict working set.

Rejected: merge candidate directly into main or reapply dream notes/tool writes to live. Those approaches lose the private inspection unit, change sequences and invalidate exact promotion receipts. Folding reconciliation into promotion is also rejected: keep an exact target known before fast-forward, with a bounded three-attempt reconcile/promote race policy as approved W7.

### Preserve membership ownership beyond cell-level merge

At exact common base `B`, candidate target `T` and captured live `L`, read only membership keys that the candidate changed, identified by canonical existing state-key shape. Compare presence/version tokens and validate nonnegative versions at each exact revision. If candidate changed a membership token and live also changed its token from `B`, return explicit membership conflict before native merge, including equal-value overwrite and absence-to-present cases. Do not apply predicates to reports, sessions or journal rows. Native row/schema/constraint conflicts remain fail-closed and are not auto-resolved.

Rejected: rely solely on Dolt identical-cell merge. Equal membership values and version increments can merge cleanly while candidate undo still names an older membership; that would violate the independently accepted live writer's token. Root confirmed this narrow overlap policy on 2026-10-05. A generic report/session compare guard or field merge is unnecessary and out of scope.

### Exact committed reconciliation identity

The mutation and read-only outcome are request-bound to `{id, generation, branch, from, live}`. Both facade and owner know the supplied exact heads before dispatch. Bind the receipt-progress key to that full tuple, following existing selected-abandon progress identity (`service/rpc.rs:2081`); same-generation unknown or differently bound IDs remain uncertain. No table/schema/outbox or new commit-message receipt format is added.

The store's pending receipt retains its pool/session and exact coordinates. After SQL completion/rollback, prove clean state and one exact outcome: head still `from` means no committed merge; head equals supplied `L` and `from` is an ancestor of `L` proves private fast-forward; otherwise the exact candidate head must have exactly two ordered parents `[from,L]`. The remote outcome first proves original worker completion in the same generation, or original owner reap through successor attachment. A clean unchanged head after terminal worker/reap proves no committed reconciliation, but cannot manufacture the original no-op/conflict detail after restart: return honest typed `NotCommitted` plus current checked open status. Missing, dirty, changed or mismatched evidence stays uncertain. Never scan arbitrary descendants and declare success from ancestry alone.

Actual pinned-engine fixtures exposed a query-shape failure on `dolt_commit_ancestors`: selecting both parents by commit hash alone returns SQL 1105 `max1Row`. The optimizer's root cause is not established. Bound the reader to exact parent indexes 0 and 1 plus explicit absence of indexes outside that range at the same immutable target. This proves complete cardinality/order without scanning history; the corrected two-parent fixtures pass. No engine or pin change is required.

Rejected: an owner-only captured live revision, which the pending client would not know after a lost reply. Requiring the caller's checked input solves that race with a definite no-effect result and reuses existing exact outcome semantics. A new merge-message marker or second post-merge receipt commit is unnecessary, adds format/fast-forward complications, and could change the exact target. Native merge behavior and ordinary commit metadata remain unchanged.

### Adopt effective base through a fresh checked candidate

Keep the original creation and each reconciliation proof immutable. `base()` continues to identify the checked common-ancestor/effective base carried by the current authenticated handle; creating a fresh handle after a proved merge adopts `L` without rewriting old request receipts. Bind new promotion and selected-abandon intents to that effective base and exact head. Every pending runtime promotion proof and facade transition record must be updated together from typed reconciliation, including reattachment and same-generation/restart recovery. Candidate inspection describes creation/common ancestor or latest reconciled base, rather than claiming all common ancestors are creation bases.

Existing `Receipt::Promotion {base,target}` remains an exact fast-forward pair. No `descendant of target` rule is introduced. A promotion lost reply still publishes only after its original exact target is proven promoted; later sibling writes trigger normal shared-state reload, not restaging an old graph.

### Runtime settlement and dream lease

Retain existing owner dream lease around candidate creation, candidate snapshot, controlled inference and each explicit publication/resolution attempt. No SQL transaction, connection or ordinary mutation guard crosses inference. Cancelled accepted work continues in its existing memory-owned worker; lease/attachment cleanup cannot authorize candidate abandonment or replay. A subsequent controlled operation reacquires the dream lease before resuming pending reconciliation/promotion work, using exact recovery first. Ordinary writes remain available.

The dream already uses a candidate topology for prompts and prepared candidate-local actors. Keep those policies. Stage the desired membership once, then update only checked target/base and outcome status through reconciliation. At most three attempts sample live, reconcile and fast-forward, handling both definite `LiveMoved` before merge and movement between reconcile/promotion. Exhaustion or overlapping membership reports explicit recoverable conflict without rerunning providers. A recovered `NotCommitted` is not fabricated into an earlier membership conflict: retain its checked open status for deliberate continuation or existing explicit resolution. Preserve staged report and open candidate. Free undo must also use the existing dream lease and version-conditional membership-only compensation.

### Privacy and C1 coordination

Do not attribute dream note/tool rows to the dreaming session. They remain unavailable to raw session windows and captured compaction ranges, but existing policy-selected notes/summaries can be shared. Test merged rows below another session's cursor, source checkpoint validity and export retention. Keep usage on its separate existing usage branch, preserving exact natural-key ledger accounting; reconciliation does not invent candidate usage writes.

N3 memory ownership: candidate/store reconciliation and cleanup, facade typed recovery, service/RPC contracts/golden and focused tests; runtime ownership: `dream.rs`, candidate-related `engine.rs` pending proof/recovery, and fixtures; docs: owning memory/session pages. C1 separately owns actor compaction/event metadata, exact summary-ID read and settlement notices. Shared engine/facade/RPC edits were coordinated before source, and accepted dependencies are normally integrated with one protocol minor 1.11. C1 is not a business blocker.

## Integration contract

Use pinned full Dolt only. Its documented merge procedure accepts commit revisions and implicitly commits its transaction; commit ancestors expose indexed parents. These are design inputs, not native acceptance: actual pinned-engine fixtures must prove merge/fast-forward/unchanged behavior, parent ordering, clean conflict rollback/error behavior, cancellation/session return and restart outcomes. See [Dolt merge procedures](https://www.dolthub.com/docs/sql-reference/version-control/dolt-sql-procedures/#dolt_merge) and [Dolt commit ancestors](https://www.dolthub.com/docs/sql-reference/version-control/dolt-system-tables/#dolt_commit_ancestors).

Use one typed reconciliation mutation and one read-only outcome query, with exhaustive mutation/receipt/reply classification. Reuse exact project/generation/attachment checks, bounded metadata and existing service frame envelope. Conflicts expose bounded validated table/key coordinates, never row values, private history or SQL diagnostics; incomplete cleanup stays uncertain rather than a definite refusal. Older services reject new required protocol calls before acceptance. No public stale-base merge command or dependency update is introduced.

Pinned default native conflict handling rolls back before returning its fixed error. Only a positively classified native conflict plus exact unchanged heads and clean working-set proof may become a typed conflict. Membership preflight has actual known key metadata. Native row/schema conflicts may obtain bounded table names/state primary keys through existing read-only preview functions at exact `from`/`L`, after refusal only, within the existing operation budget. Unsupported or failed metadata reads return a conflict explicitly marked as having unavailable coordinates; they never imply absence of conflict or authorize publication. No private body columns, force flag, conflict sysvar or mutable dry run is used. Failure to prove cleanup preserves uncertainty.

## Risks / Trade-offs

- [Dolt commits/rolls back conflict behavior differs from the old design's assumptions] → Real pinned-engine clean/conflict/constraint, exact-parent, fast-forward and unchanged fixtures before runtime integration; preserve uncertainty until exact clean state is proved.
- [Candidate or live moves during recovery] → Request-bound proof plus fresh checked handle, exact effective base/target and no arbitrary descendant success. Serialize owner mutation and reject changed refs before effects.
- [Membership versions coincide despite independent writes] → Compare each side against exact common-base tokens before native merge, only for candidate-changed membership.
- [Message allocator changes across restart] → Highest candidate sequence/restart fixture and deliberate primary-key collision fixture; collision must preserve both histories explicitly.
- [Shared engine/RPC ownership collides with C1] → Separate worktrees, assigned files/seams and accepted commit integration before source; no duplicate full suite or protocol pin writers.
- [Hosted dependency acceptance is pending] → Retain normal accepted local dependency integration and root queue coordination before publication; no N4 admission, and local versus hosted native acceptance remains explicit.
