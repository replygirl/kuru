## Context

`MemoryStore::open` owns provisioning, migration, validation, and server lifecycle, while the CLI currently waits silently before entering its normal terminal session. Legacy import protects SQLite source layout but its parent privacy rejection is generic.

## Goals / Non-Goals

**Goals:** expose truthful fixed startup stages to the application; retain silent library opens; keep JSON on stdout; give a safe, actionable legacy privacy refusal; and prove real cold/warm/import behavior.

**Non-Goals:** a spinner, percentages, timing estimates, a new daemon, automatic permission repair, changing digests/probes, or a warm-path optimization.

## Decisions

- Use a bounded receiver returned with an observed-open future. An arbitrary callback was rejected because it could be invoked under ownership-sensitive operations and makes receiver loss/lifetime harder to bound.
- Keep stage definitions and emissions in memory, but map them to fixed stderr text in the app. A memory-owned display string was rejected because terminal behavior belongs to the application.
- Emit `Ready` only immediately before a successful return after final validation. Treating server connection acceptance as ready was rejected because migration/activation can still fail.
- Refuse an unsafe Unix legacy directory with its exact `chmod 700` remedy; automatic chmod was rejected because the application must not adopt or change an untrusted legacy path. Windows retains native privacy guidance.

## Risks / Trade-offs

- [Progress receiver is dropped] → nonblocking reporting disables itself and never owns the store, locks, server, or worker.
- [A stage remains visible when startup fails] → it describes started work; the ordinary open error remains the outcome authority and ready is not emitted.
- [Large warm verification is costly] → retain current validation and record measured cold/warm cost separately; no optimization follows from this change.

## Operational surface

The application opens only its existing local embedded Dolt runtime; this adds no
bind address, container, required secret, connection limit, binary version, or
architecture selection. Fixed progress is bounded stderr presentation before the
existing terminal session, while library opens stay silent.
