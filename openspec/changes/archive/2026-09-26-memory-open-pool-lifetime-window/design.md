## Context

SQLx 0.9 `PoolOptions::connect_with` uses `acquire_timeout` as the deadline of its initial connection, and every later `Pool::acquire` uses the same stored value (`pool/options.rs` `connect_with`, `pool/inner.rs` `acquire`). The archived fix passed the opening window as that value, so the window outlived the opening phase for every pool created during it. The retained active main pool, `Shared.usage_pool`, and any other pool cached during opening kept up to the remaining startup deadline, about 30 seconds by default, for their whole lives.

## Goals / Non-Goals

**Goals:**
- Pool lifetime `acquire_timeout` is always the ordinary window for `Server::pool`.
- Only the first acquisition of an opening-phase pool may use the remaining startup deadline, and never less than the ordinary window.
- Keep identity fail-fast during open, the Windows accept bound and `finish_opening` placement.

**Non-Goals:**
- Changing the post-readiness probe, which builds a transient pool and closes it after verification.
- Rebuilding or swapping retained pools at `finish_opening`.

## Decisions

- **Lazy pool plus a bounded first-acquire loop.** `connect_lazy_with` creates the pool without connecting. The first `acquire` runs connection plus `after_connect` under the ordinary lifetime timeout and is retried on `PoolTimedOut` until the first-acquire deadline. Each retry is also bounded by that deadline. An ordinary attempt has a first-acquire window equal to its lifetime timeout, so it makes exactly one acquire and releases it, as `connect_with` did with `min_connections(0)`. Rejected alternative: keep `connect_with` with a longer `acquire_timeout`, which is the lifetime defect itself. SQLx offers no per-call acquire timeout, and connections cannot be moved between pools.
- **Identity fail-fast races each acquire, not construction.** Only the acquire future is dropped when the recorded identity rejection wins. The pool itself is dropped by the caller on error, as `connect_with` did.
- **Test seam stalls callbacks until one instant.** The delay starts at the first callback of a pool attempt, and every callback in that attempt waits until the same instant. This models a server that becomes responsive at a point in time, which is what a retrying first acquire can recover from. A delay applied again to each retry would never succeed.

## Risks / Trade-offs

- [Each first-acquire retry restarts the connection handshake, so a server that consistently needs more than the ordinary window for every handshake is not rescued during open] → The failure mode observed on Windows was a just-started server becoming responsive late. The single-attempt probe already proved the server answers within the startup budget. A persistently slow handshake also fails every ordinary post-open acquire.
- [A retried acquire can overlap the ordinary window's final backoff] → Every retry is bounded by the first-acquire deadline, so the opening window is never exceeded.
