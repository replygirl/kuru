## Context

`Server::open_inner` treats readiness, the first authenticated probe and its identity check as one startup operation bounded by `startup_deadline` (`startup_timeout_secs` plus the two-second supervisor-transport allowance). The probe's pool is closed immediately, and the store then asks the same just-started server for its working pools through `Server::pool`, which always used `PoolAttemptOptions::ordinary()` (a flat two-second SQLx acquire window) and `verify_identity` (a flat two-second identity query window). Every pool requested during a store open therefore received a fresh, much shorter budget than the startup it immediately follows. A fresh store open runs four owned starts and requests many such pools (staged main pools, migration-attempt branches, historical classification branches, candidate recovery and the usage-ledger branch). On a loaded native Windows runner, Dolt can accept TCP before the first authenticated connection completes within two seconds, which produced both #91 Windows failures: one wrapped by the store's `open migrated staged main pool` context and one bare `authenticate memory branch pool` from an unwrapped open-sequence branch pool.

## Goals / Non-Goals

**Goals:**
- Pools the open sequence requests from a server it just started share that start's remaining startup deadline, never receiving less than the ordinary window.
- Post-open pool acquisition and identity verification remain exactly the ordinary two-second windows.
- Identity rejections are not waited out by the longer opening window.

**Non-Goals:**
- Changing `startup_timeout_secs`, the transport allowance, query timeouts or any test deadline.
- Changing borrowed read-only endpoint inspection (`live_endpoint`), which starts nothing.
- Restructuring the store open sequence or pool caching.

## Decisions

- **The opening phase is a property of the server, ended by the store.** The owned start records `Some(startup_deadline)`; the borrowed read-only path records `None`. `MemoryStore::open_inner` clears it immediately before reporting `Ready`, after candidate recovery and the usage-ledger branch. Rejected alternative: a dedicated `opening_pool` call at the two named store call sites — it cannot cover the unwrapped migration, classification, recovery and usage-ledger pools that produced the second CI failure.
- **Window = max(ordinary, remaining startup deadline).** Rejected alternative: the bare remaining deadline. A slow but healthy start could leave that near zero and make the first open-sequence pool strictly worse than before. The floor is the existing ordinary window, so no attempt is ever shorter than today. Computing a `Duration` (not a new instant) makes the not-opening case exactly two seconds. The existing two-second literals are named `ORDINARY_POOL_WINDOW`; no new budget is introduced.
- **Opening-phase identity rejections are terminal.** SQLx 0.9 returns credential/handshake failures immediately (only connection-refused and connect-phase-transient database errors retry) but retries any `after_connect` error until the acquire deadline. The two authored identity rejections (data-directory mismatch, SQL project/instance mismatch) cannot improve on retry against the same endpoint, so opening-phase attempts (including the post-readiness probe, which already used the remaining deadline) stop at the first such rejection and return it as the typed `sqlx::Error::Protocol`. Ordinary post-open attempts keep their existing retry-until-two-seconds failure mode, preserving "unchanged after open".
- **Windows supervisor pipe accept uses the same remaining deadline.** The accept was already awaited inside `timeout_at(startup_deadline, …)`; its separate flat five seconds could fail a healthy slow start earlier than the startup budget. Rejected alternative: leave it; it is on this exact open path and the change is one expression.

## Risks / Trade-offs

- [A genuinely unreachable server during open now takes up to the remaining startup deadline, not two seconds, to fail a pool] → That is the same bound the probe and readiness already accept for that start; the configured `startup_timeout_secs` stays the user's control.
- [`Server::pool` holds the per-server pool map lock across authentication, so a longer opening acquire serializes other branch requests longer] → Open-sequence pool requests are already sequential; post-open behavior is unchanged.
- [The Windows accept change cannot be compiled or exercised on the macOS development host] → Native Windows CI is its check; recorded as unrun locally.
