## Context

`Server::open_inner` gives the owner readiness frame the configured startup timeout plus its existing two-second transport margin. After Ready, the required client probe uses `connect_pool_attempt`'s unrelated two-second SQLx acquisition deadline, then `verify_identity` uses another two-second deadline. A Windows native run failed before the probe's `after_connect` callback; it does not establish whether TCP connection, handshake, authentication, or runner scheduling consumed that interval. Owner readiness alone cannot make an unverified store usable.

## Goals / Non-Goals

**Goals:** Bound owner readiness, initial authenticated connection, and identity verification by one existing startup allowance; retain exact identity checks and owned cleanup on failure. Prove the corrected acquisition budget with real Dolt and a controlled delay inside the acquisition future.

**Non-Goals:** Change normal branch-pool acquisition, add connection retries, enlarge the configured startup budget, or classify the unobserved Windows lower-level delay as a provider or Dolt fault.

## Decisions

- Capture one monotonic deadline immediately before owner launch using `options.timeout + 2 seconds`, matching the current readiness allowance. Both Unix and Windows readiness waits consume that same deadline; no fresh allowance begins at Ready.
- Pass only the remaining duration to the initial post-Ready SQLx acquisition. Bound the following identity query by the same deadline. Ordinary `Server::pool`, attached endpoint probes, and their two-second acquisition policy stay unchanged.
- On an expired step or failed verification, close an acquired probe under the existing cleanup grace, then use `startup_failure` to reap the owned supervisor while retaining its guard. Cleanup can finish after the startup deadline; an unverified `Server` is never returned.
- Use an instance-scoped, test-only delay inside the initial SQLx acquisition callback to distinguish the old fixed two-second failure from the new remaining-budget success with real Dolt. A second bounded case exhausts the same startup deadline and verifies no usable owner or held lease remains before a fresh open. The callback delay exercises the acquisition budget but does not claim to recreate the exact Windows pre-callback stall.

## Risks / Trade-offs

- A very late Ready leaves little time for client authentication. This is truthful to the existing bounded startup promise; the caller gets a startup failure and owner cleanup instead of a separate hidden extension.
- The deterministic regression enters SQLx's callback after handshake, while the CI failure was before that callback. The test proves the budget correction and cleanup, not the lower-level cause of the native stall. Exact-head Windows CI remains required.

## Operational surface

This is the existing private loopback Dolt sidecar on supported native runners and packaged targets. The fix adds no bind address, secret, environment variable, tool version, or launch topology.

## Integration contract

SQLx pool acquisition and Dolt identity queries remain internal to `kuru-memory`. The initial connection uses the authenticated project and exact data-directory/instance checks already required by the native server; no RPC, database schema, or external route changes.
