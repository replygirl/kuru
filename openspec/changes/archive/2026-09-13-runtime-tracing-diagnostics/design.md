## Context

The runtime owns asynchronous tasks while the app alone owns CLI process setup and checked filesystem authority. A global mutable project sink would race concurrent library calls and violate that separation.

## Goals / Non-Goals

**Goals:** one app-installed fixed subscriber for the real CLI process; one project ring retained under its writer lease; safe fields at producer boundaries.

**Non-Goals:** a logging platform service, remote exporter, metrics, `RUST_LOG`, payload capture, or a new session-log data domain.

## Decisions

- Install a fixed app subscriber once for the real binary process, then open one sink before runtime memory startup; library entrypoints only emit tracing. A mutable active-project router is rejected because it permits cross-project races.
- Use an app-owned byte/count ring over checked platform directory/file primitives. `tracing-appender` is rejected because it rotates on time and introduces a separate async-drop policy.
- Reuse the journal digest calculation for turn correlation. Rehashing raw caller IDs separately is rejected because it creates an independent identity contract.
- Emit events/spans with explicit safe fields and async instrumentation. Formatting errors to redact later is rejected because error chains may already contain private payloads.

## Risks / Trade-offs

- [Process-global subscriber] → install only in the binary entrypoint and exercise logging through child CLI fixtures, not parallel in-process global setup.
- [Diagnostic I/O failure] → return a bounded application error for setup; retain observed write failure for shutdown without replacing committed turn semantics.

## Operational surface

The native `kuru` process installs the fixed subscriber for its own runtime-owning
commands only. It opens no listener, container route, remote exporter, secret,
or environment-selected filter; the only added CLI control is `--debug`. Exact
workspace-pinned `tracing` and `tracing-subscriber` versions apply on every
supported native target.

## Integration contract

The only SDK integration is the pinned `tracing` event/layer API. Connector
fixtures retain their existing fake HTTP shape and emit typed retry fields;
runtime fixture calls retain their existing provider and ToolHost contracts. The
diagnostic record is private app JSONL rather than a public schema or provider
protocol, so no route, mount, payload type, or persistent domain migration is
introduced.
