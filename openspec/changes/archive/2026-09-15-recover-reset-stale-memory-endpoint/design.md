## Context

PR26 run `34935309747`, Windows memory/runtime job `104272547990`, observed `windows_lifecycle::enclosing_job_loss_contains_the_tree_and_reopens_committed_state` fail after the prior Job was reaped. The endpoint's raw TCP probe connected, but the following SQLx connection failed with Windows error 10054 before `after_connect` was entered. The listener identity and reason for the reset are not established.

`live_endpoint` currently treats a successful raw probe as availability, then propagates every authenticated-pool error. This prevents a writer from reaching the existing lifecycle lease and owned supervisor startup even when the typed evidence says authentication never began because the connection was reset.

## Goals / Non-Goals

**Goals:**

- Recognize only the observed typed pre-callback reset as endpoint unavailability.
- Reuse the current lifecycle lease and startup path to recover the same committed store.
- Prove the boundary with an owned loopback listener and preserve the original errors for all ineligible cases.

**Non-Goals:**

- Identify the transient listener or add endpoint retries, deadline changes or process termination.
- Treat authentication, identity, data-directory, protocol, SQL or later-callback I/O failures as stale endpoints.
- Change server publication, platform process ownership or memory storage behavior.

## Decisions

### Classify the SQLx result before erasing it

`connect_pool` keeps its existing `anyhow::Result<MySqlPool>` contract for every caller. A private inner connection attempt will retain the typed `sqlx::Error` and the existing `ConnectionObservation` long enough for `live_endpoint` to distinguish `sqlx::Error::Io` with `ErrorKind::ConnectionReset` while the observation still says `after_connect not entered`. A new observation predicate fails closed if the observation lock is unavailable. Only that exact state becomes endpoint unavailable; all other errors continue through the existing contextual diagnostic.

This uses both the transport type and callback boundary. Matching error text or raw OS code alone would be broader and could incorrectly recover from authentication or verification failures.

### Continue through existing ownership

On the narrow unavailable result, `live_endpoint` returns no live endpoint. `Server::open_inner` then follows its existing supervisor path, whose supervisor acquires the authoritative lifecycle lease before starting Dolt. The change introduces no loop, sleep, retry or detached work.

### Exercise the real connection boundary

A direct portable `server.rs` regression will use a deterministic owned Tokio listener on `127.0.0.1:0` that accepts exactly two connections: the raw TCP probe, then the SQL connection. The second connection uses Tokio's safe zero-linger setting before close so the peer receives an abortive reset before SQLx can enter `after_connect`; the test proves `live_endpoint` returns no endpoint only under the narrow predicate. The native Windows end-to-end fixture reuses that connection shape after proving the prior Job has zero active processes and releasing the lifecycle lock, rewrites only the stale endpoint record's port while preserving its instance, then proves the corrected open starts its owned server and reads the same committed row. Every listener task must prove both connections were observed and be joined on every success or error path. Existing wrong-credential, checked-directory, SQL-identity and occupied-lifecycle controls remain authoritative for refusal behavior.

## Risks / Trade-offs

- **A reset can have more than one external cause.** The fix makes no listener-identity claim and only reuses the lifecycle lease that already arbitrates ownership.
- **An overly broad predicate could hide a genuine server error.** The predicate requires the typed I/O kind and the untouched pre-callback phase; focused refusal controls cover the remaining classes.
- **Socket reset behavior varies by platform.** The decisive owned listener regression uses Tokio's pinned safe zero-linger API on native Windows, while host checks cover compilation and unchanged server lifecycle where portable.
