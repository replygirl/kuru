## Context

The active project directory is only one part of the managed deletion set:
interrupted staging/recovery trees can be retained beside it. `Directory` can
remove a checked regular file but has no root-identity-bound tree removal. The
ordinary open path currently imports legacy SQLite when an active project store
is absent, so a plain directory delete would resurrect content.

## Goals / Non-Goals

**Goals:** remove exactly one quiescent canonical project's managed Dolt trees
and history; persist retry-safe suppression and intent; retain authority through
quarantine and deletion; clean the matching app diagnostics ring under the same
CLI writer lease.

**Non-Goals:** secure erasure, automatic expiry, selected-history rewrite,
restore UI, legacy SQLite deletion, export deletion, cache cleanup, provider
startup, or a generic filesystem cleanup framework.

## Decisions

1. **One control record after lease acquisition.** Purge first obtains and
   verifies all lifecycle leases, then atomically publishes suppression plus an
   optional pending inventory before the first move. Publishing before leases
   could make a live-owner refusal mutate durable state; separate suppression
   and manifest protocols would create inconsistent recovery states.
2. **Quarantine by held identity, then consume it.** Memory enumerates exact
   recognised project trees, moves them by `LifecycleLease::move_to` into a
   private same-volume quarantine, and consumes the held checked root through
   the platform primitive. Re-selecting paths after an interruption is rejected
   because an attacker or later open could occupy the original name.
3. **Associated memory entrypoint.** `MemoryStore::purge` takes options and
   never opens a live pool, provisions Dolt, or imports legacy input. Reusing a
   pre-opened store would make live-owner refusal and engine-free deletion
   impossible.
4. **App cleanup remains app-owned.** The CLI calls a small canonical
   diagnostics-ring removal helper after managed memory deletion while retaining
   its project writer lease. Moving diagnostics into memory would invert package
   ownership; pretending database success removed an app ring would be false.

## Risks / Trade-offs

- **A native remove may be uncertain.** → retain the original/quarantine
  identity in the control record and require explicit reconciliation on retry.
- **A shared legacy snapshot contains selected-project rows.** → preserve it
  and suppress only automatic re-import for this project.
- **App-ring cleanup can fail after database cleanup.** → return an explicit
  incomplete-purge result with completed DB state; retry only the verified ring
  inventory.

## Operational surface

The only user surface is local `kuru memory purge --yes`; it opens neither a
network listener nor a provider connection and requires no secret. It uses the
selected executable's existing embedded Dolt compatibility only for ordinary
future opens, never for purge itself. The checked removal primitive runs on the
native Unix and Windows filesystems; native Windows held-handle and uncertainty
behavior remains a required CI proof.
