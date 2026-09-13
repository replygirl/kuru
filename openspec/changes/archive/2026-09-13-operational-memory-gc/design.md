## Context

The active `operations` table grows one current-state row per mutation, ordinary
`candidate_<uuid>` refs survive every outcome, and generated server YAML disables
automatic GC. Existing writes serialize through `Shared.write`, reconcile one
accepted SQL session before another mutation, and retain the Dolt process through
supervisor reap. Dolt is pinned to 2.3.3 at commit `79caf258...`.

## Goals / Non-Goals

**Goals:** Bound active operational receipts, make destructive candidate cleanup
explicit and recoverable, and use the pinned engine's owned GC cadence.

**Non-Goals:** Expiring or rewriting user history, deleting unrecorded candidates
or migration/import refs, adding a retention policy, maintenance service, schema
migration, export record, GC status API or secure-erasure claim.

## Decisions

### Replace receipts inside the existing transaction

`apply` deletes prior `operations` rows immediately before inserting the current
UUID and committing the mutation. `resolve_uncertain` already runs under the
writer lock before `apply`, so the receipt needed for a lost reply cannot be
removed early. A scheduler or receipt-age policy adds no safety.

### Encode explicit candidate state in exact refs

An ordinary candidate remains untouched until promotion or `Candidate::abandon`
chooses a random-UUID-paired reserved status name. After candidate writes settle,
Kuru drains its registered pool and requests a non-force rename so pinned Dolt's
all-session check remains authoritative. A table and schema version would repeat
state Dolt refs already persist.

Pinned Dolt implements rename as ref copy, working-set copy and old-ref deletion,
not one atomic update. After the command socket ends, Kuru therefore observes
both exact names and hashes: old-only means no transition; status-only means
complete; equal old/status heads mean a partial transition whose duplicate may
be removed while the status ref remains. A live operation fails honestly on
missing, mismatched or dirty state. Startup preserves and skips a known
ineligible status ref so it cannot block ordinary main use when inspection
proves no mutation or SQL session remains uncertain. Candidate working state
must be clean before pool retirement, so head equality never hides unknown
dirty content.

Startup never performs a requested merge. It handles a small fixed batch and
reclaims a promoting ref only when `DOLT_MERGE_BASE` proves its head is already
reachable from live. Explicit abandoned force deletion is allowed only after the
current owner establishes no live view; ordinary merged cleanup uses non-force
deletion. Every uncertain delete waits for its command session to end and checks
exact ref absence.

### Separate promoted authority from cleanup

Promotion still runs in an accepted worker under the writer lock. Once the live
fast-forward is reconciled, that target is cached on the `Candidate`, returned on
repeated promotion after view closure, and published by the runtime. Ref cleanup
may be retried and cannot turn that authoritative outcome into a failed dream.

### Use the pinned engine's automatic collector

Generated YAML enables `auto_gc_behavior`. Dolt 2.3.3 uses one background worker,
a 128 MiB journal/growth trigger and at most thirty one-minute load deferrals.
Its default session-aware safepoint visits registered roots and waits for active
commands; Kuru's cleared child environment cannot select legacy connection
killing. This fits existing process ownership. An application `DOLT_GC` worker
would duplicate scheduling and introduce a harder unconfirmed-query lifetime.

## Integration contract

`kuru-memory` remains the sole owner of Dolt configuration, branch SQL and real
engine fixtures. The integration uses only pinned Dolt 2.3.3 contracts already
available over the private SQL server: `DOLT_BRANCH` for non-force status rename
and exact deletion, `dolt_branches` for binary name/head observation,
`DOLT_MERGE_BASE` for ancestry, `DOLT_MERGE(..., '--ff-only')` for publication,
`DOLT_GC('--full')` in acceptance fixtures, and
`@@GLOBAL.dolt_auto_gc_enabled` for effective configuration. Status names contain
one canonical simple UUID and are never accepted by prefix alone. Runtime sees
only the existing `Candidate` plus its new `abandon` operation; export schemas,
turn IDs and user storage records do not change.

## Risks / Trade-offs

- [Dolt rename can expose two refs] → Reconcile the exact UUID pair and hashes;
  never apply a broad prefix cleanup.
- [Force deletion bypasses Dolt's session check] → Use it only for explicitly
  abandoned refs after the current owner drains and rechecks the exact view;
  preserve on any doubt.
- [Automatic GC reports failures in private server logs] → Preserve bounded
  diagnostics, test the effective setting and actual pinned collector, and call
  the cadence best effort rather than promising immediate success.
- [Reachable history still grows] → Document that receipts disappear only from
  active state and all retained revisions remain part of storage.
