## Context

A `MemoryStore` view shares its Dolt server and pool with clones, but the old
borrowed `close` signature did not express that it closes every view. Reads also
bypassed the existing closed-pool check, and reconciliation discarded the
already-computed durable outcome. The revision query relied on unqualified
`dolt_log` order although the pinned engine exposes graph metadata.

## Goals / Non-Goals

**Goals:**

- Make shared-server shutdown explicit at the consuming API boundary.
- Give all closed views one stable failure message.
- Expose the existing durable reconciliation result without changing its
  pending-publication authority.
- Make graph ordering explicit and test it against the bundled Dolt runtime.

**Non-Goals:**

- New owner-token or reference-counted close protocol.
- Schema, migration, journaling, GC, retention, erasure, or read-only ownership
  changes.

## Decisions

- Keep `Server::close` as the sole shared cleanup authority and make only the
  store wrapper consuming. Existing command cleanup retains its explicit store
  handle until the writer lease can be released.
- Reuse `pool.is_closed()` for all public read entrypoints instead of adding a
  second closed-state flag.
- Return the existing `Option<bool>` from reconciliation and retain all current
  durable receipt checks.
- Query the pinned Dolt `dolt_log` system table with explicit graph order and a
  hash tie-break. A real fixture establishes the supported column and tied-date
  behavior before treating that order as contract.

## Risks / Trade-offs

- [A consuming close requires mechanical caller changes] → update callers that
  intentionally close the retained command owner and let ordinary views drop.
- [Graph metadata is engine-specific] → pin the exact SQL to the bundled Dolt
  fixture and retain native Windows verification as a separate required gate.

## Integration contract

The only external contract is the bundled pinned Dolt system-table schema. The
fixture uses an isolated local Dolt server and no provider or credential route.

## Integration handoff

This branch predates selected-note forgetting and human notes reads. When this
change is transplanted into that source, the public notes-read entrypoint must
use the same closed-pool guard before it queries storage, while retaining that
feature's signed note IDs and all stored roles.
