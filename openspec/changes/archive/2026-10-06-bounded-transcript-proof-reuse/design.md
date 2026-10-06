# Design

## Context

Public paging captures actual selected HEAD under a short writer guard, then validates a historical immutable pool and walks every predecessor for structure, reachability and exact totals. It decodes only page bodies, but restarts the full metadata walk for each continuation. At a fixed 32-record UI page size, a complete scan repeats quadratic metadata work.

## Decisions

One Arc-owned proof slot belongs to each pinned store view. Ordinary and logical-receipt clones share it; fresh/candidate views start empty. Each authenticated service attachment takes a main-view clone with a fresh slot, so independent session attachments do not evict one another. It retains no pool, connection, transaction, body or native resource and disappears with those view clones/attachment EOF.

The slot contains exact branch/session/revision/head identity, validated chain totals and the requested/next positions of the last two successfully returned pages. It holds at most four positions and 2 KiB of encoded proof. Bounds constrain the optimization only: a proof that cannot fit is not retained, and valid requests keep the existing full-validation path.

Every request independently captures the selected revision, rejects stale cursors, verifies the immutable pool and loads/validates that cut's catalog. Only an exact matching proof with HEAD or a retained actually issued position can skip the global walk. It then validates metadata/bodies from the proven position to the page boundary, retaining the existing overflow/byte accounting and legacy-prefix checks. Forgotten or forged positions require the complete reachability walk. No caller-provided revision selects a pool.

Cache state is read/copied briefly, never locked through SQL. Publish the new state only after transaction commit, final encoded response trimming and typed page validation. Errors and cancelled partial reads cannot replace the retained proof. A concurrent view change makes the next request's exact key check fail safely.

Initial/cache-miss projection remains O(N). Sequential older navigation/search performs one initial full chain proof plus page-sized metadata/body work. Existing UI newer-page reconstruction still repeats prefix pages and is explicitly outside this correction.

## Operational surface

The existing native TUI and authenticated private per-project service run unchanged on supported hosts. This is read-only in-memory optimization, with no bind address, secrets, new process, deployment input or connection-limit changes. Proofs retain no pool/connection/native ownership and do not delay final-client retirement.

## Risks / Trade-offs

- A tiny continuation window can evict a valid older cursor: fall back to full proof, preserving API semantics.
- Mutable HEAD changes invalidate proof reuse: retain current stale-source behavior and require a fresh proof.
- Cancellation before publication must leave no partial proof: exercise an actual completed SQL read held before cache publication, abort its caller, then observe a fresh full walk.
- Initial proof still has the existing finite traversal budget: do not claim constant-time projection or arbitrary-history latency.
- Warm bodies retain complete typed privacy validation; cached structure never substitutes for body validation.
