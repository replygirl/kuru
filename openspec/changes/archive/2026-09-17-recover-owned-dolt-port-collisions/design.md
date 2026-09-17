## Context

The memory supervisor binds an ephemeral loopback listener to select a port, then drops that listener before writing Dolt's YAML and spawning the owned database process. In P9 Ubuntu CI, Dolt's own staged log reported `Port 44995 already in use.` and exited before readiness. No runtime request or accounting assertion had begun.

## Goals / Non-Goals

**Goals:** Recover an exact selected-port takeover without losing the owned lifecycle lease, extending the startup deadline, or publishing an unverified endpoint.

**Non-Goals:** Do not retry SQL/bootstrap failures, parent cancellation, termination, uncertain child cleanup, or arbitrary startup errors. Do not alter unrelated port owners.

## Decisions

- Keep the original startup `Instant` and lifecycle lease across at most three owned Dolt attempts. Every attempt selects a new concrete loopback port and writes its own private YAML.
- Retry only a typed premature Dolt exit and a fully drained fresh trusted child log containing the exact chosen-port collision line, after the child is reaped and before Ready or endpoint publication. An error or timeout in cleanup returns the failure.
- Retain the explicit numeric port: the current endpoint, authenticated readiness and client connection contracts need it. Dolt's documented listener configuration does not offer port zero as a supported value.
- Use an internal test hook at the instant the selected listener is released to place a real unrelated listener on that exact port. The test hook is not a public setting or an external control surface.

## Risks / Trade-offs

The selected port can be taken again. The finite attempt bound and original deadline prevent unbounded startup, and the final error retains its private log. Exact matching may miss a changed Dolt diagnostic; that safely fails closed rather than retrying unrelated errors.

## Operational surface

Dolt continues to bind only `127.0.0.1` inside the owned memory supervisor. The correction applies equally to local runners and packaged binaries, including native Windows, macOS and Ubuntu; it adds no container route or externally reachable listener. Existing Kuru-generated root credentials stay in the private process environment and records, never in the collision diagnostic. The verified bundled Dolt 2.3.3 for each target architecture and current 32-connection limit remain unchanged.
