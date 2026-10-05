# Design

## Context

The proposal names the ownership problem. Schema 9 and wire 1.8 provide the
conditional primitive; the runtime still persists `Topology` as one blob.
Existing export readers validate a commit-qualified pool. Server pools are
shared by revision, and private candidate handles belong to their attachment.

## Goals / Non-Goals

Implement the proposal with concrete MemoryStore methods and one runtime codec
helper. Preserve existing typed public Topology/Part/Relationship/StateReport,
single-driver checkpoints and candidate exact-target recovery. No generic
storage hierarchy, raw-field merge framework, admission, candidate merge,
new timer, dependency update or retained-graph limit.

## Decisions

- Add schema 10 as a staged transform after then-latest schema 9. In the existing
  build-attempt transaction before version advancement, split all matching
  canonical legacy topology rows into membership and reports. Copy raw parts,
  relationships and report Values after compatible typed shape checks; retain
  original legacy bytes. Reject malformed or conflicting destinations atomically.
  Materializing during PR A was rejected because legacy writers remained active.
- Freeze keys at `{scope}/{mode}/membership`, `{scope}/{mode}/state/`, retained
  topology and dream-undo keys. Membership JSON is `membership.v1` with parts and
  relationships. Runtime uses ordinary typed members, retaining its established
  unknown-field serialization behavior. Legacy undo accepts an empty format;
  unknown nonempty formats refuse.
- Report rows use lowercase SHA-256 of exact identity UTF-8 as their suffix and
  `state_report.v1` records containing exact identity and raw report. Validate
  suffix/identity agreement; preserve all extra and archived report identities.
  Direct identity suffixes were rejected: legacy report maps permitted slash,
  control and long extra keys that remain publicly inspectable. Hash collisions
  and conflicting existing destinations refuse; identities are never normalized.
- Capture a selected live/candidate committed revision in a memory-owned state
  read cut. Validate its checked commit-qualified pool and retain provenance.
  Point reads and binary-prefix pages use only that pool, without holding a SQL
  transaction or writer lock between calls. Independent live pages were rejected
  because their reports could mix revisions.
- Bound page row count to 256 and target ordinary page bytes at 16 MiB; a single
  larger row remains readable when the exact existing 100 MiB service reply fits.
  Account for encoded envelope bytes and cursor metadata. No new scalar or graph
  cap follows from pagination; existing scalar APIs remain unchanged.
- Cut handles bind snapshot/view, attachment and generation. Every remote cut
  owns a dedicated read-only attachment, including candidate cuts. Capture the
  exact candidate ref/head through its original capability, then validate that
  selected canonical open ref and unchanged head when the dedicated attachment
  begins the cut; never transfer mutable candidate authority or silently select
  a newer head. Explicit close marks terminal closure only after checked cleanup
  or owned transport teardown; interrupted close remains retryable. Last-drop
  cancellation disconnects only this dedicated attachment. Closing one cut
  must not close another cut/export's shared revision pool. Restarted or closed
  handles and foreign cursors refuse; idle retirement sees retained cuts.
- Preserve the small coherent getter as a fast path only when the complete report
  inventory is known and bounded. Recheck full membership value/version in that
  same batch with the session and reports. Use one cut for the complete prefix
  scan otherwise; loading only active member reports was rejected because the
  public Topology retains extra and archived reports.
- Expand the one generic conditional publication seam to exact complete-request
  validation against the existing 100 MiB service envelope, retaining its current
  transaction, receipt, key/count and expected-key-written constraints. The
  16 MiB bound remains on small batch reads. A separate domain-specific memory
  operation or caller-selected budget was rejected: the new foundation's write
  bound must not make previously legal membership records unwritable.
- Runtime writes only owned deltas: reports retain last-report semantics;
  session/preferences/focus/turn checkpoints do not rewrite membership; relate,
  seed, dream and undo use membership expectations. Dream reads its candidate
  after branching and writes no session or report rows. Pending publication
  applies only after exact persistence; superseded shared values reload.

## Risks / Trade-offs

- Large graphs require extra pages → every page shares one checked cut; release
  handles promptly and test live updates between pages.
- Migration can discover malformed/conflicting split records → fail the existing
  attempt before schema advancement, retain the original live data and exact
  failure evidence, never overwrite a conflicting destination.
- Session focus can reference newly archived membership → clear it in the live
  session projection only after persisted membership makes its target inactive;
  never assign legacy project focus or write another session from a candidate.
- PR A merged normally as e6fa5d864f9c884c394cfb481516c4528e156013 after its
  PR-head CI succeeded; its merged-main CI later failed the unrelated cold CLI
  readiness case before schema work. The separate reviewed cold-probe correction
  supplies genuine work progress and gated evidence without claiming its native
  stall cause. Restack onto accepted main before coordinated delivery; N3/N4
  retain their later candidate/admission ownership.

## Integration contract

Memory owns schema 10, compatibility decoding and all private RPC additions.
Core owns canonical state keys; runtime owns typed membership/report assembly and
publication policy. The `membership.v1` and `state_report.v1` formats and exact
UTF-8 identity hash are frozen between those owners. Existing equal destination
versions survive migration, and legacy bytes/historical refs remain unchanged.
Wire 1.9 uses checked attachment/generation handles and opaque cursors; it adds
no provider route or third-party SDK dependency. Real released-schema Dolt,
candidate, managed-attachment and exact-envelope fixtures verify reconciliation.
The exact 100 MiB complete request/reply limit remains distinct from the 16 MiB
small read target.
