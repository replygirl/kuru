## Context

The current runtime emits string triplets and journals them with completed turn
state. P4 must replace that internal representation without changing the public
three-key event adapter, and must make journal compatibility depend on the
enclosing journal version rather than an event deserializer guessing its source.

## Goals / Non-Goals

**Goals:**

- Keep one projected semantic event representation from construction through
  replay and typed UI consumption.
- Make each admitted tool call observable once with receipt metadata that can
  be independently recomputed from public projected values.
- Preserve completed output/checkpoint and exact-retry behavior.

**Non-Goals:**

- Provider streaming, durable preview/reasoning text, permission decisions,
  cost accounting, event sourcing, and a second durable event copy.

## Decisions

### Keep semantic variants until the wire boundary

`event.rs` owns the enum, projection, and `{kind, actor, detail}` adapter;
`engine.rs` constructs projected events and dispatches journal formats; the TUI
matches variants. Converting immediately back to generic fields was rejected
because it would leave the runtime and TUI stringly and invite reparsing of
redacted presentation data.

### Use an unambiguous settled observation kind

`ToolStarted` retains the existing `tool` compatibility event and
`ToolSettled` uses `tool-observation`. Reusing `tool` for both was rejected
because legacy start events cannot be distinguished reliably from a receipt.

### Measure projected JSON serialization

The receipt builder first projects and bounds arguments and the actor-visible
result, then measures `serde_json::to_vec` bytes and hashes the result bytes.
Hashing raw tool output or measuring a formatted display string was rejected
because it could leak secret-bearing material or make public metadata
unverifiable. A textual result therefore includes JSON string quoting in its
measured bytes.

### Select event decoding from journal format

Journal replay first decodes a bounded raw wire record and selects the v1 or v2
normalizer from `TurnJournal.format`. Letting a nested `Event` deserializer
infer legacy behavior was rejected because serde does not pass the enclosing
format to elements. Incomplete resumable v1 journals are upgraded before a new
completion write; completed v1 journals remain historical records.

## Operational surface

This change has no listener, container, secret, or binary-version requirement.
The interactive surface is the existing local TUI: it receives projected typed
events from the runtime process and preserves its bounded redraw behavior.

## Integration contract

CLI and A2A-facing serialized events remain a three-key JSON object with
`kind`, `actor`, and `detail`. `tool-observation` is the only new kind; its
detail is bounded JSON with stable snake_case receipt fields. Runtime fixtures
exercise this adapter and the versioned journal wire representation locally;
there is no new SDK, mounted route, or network endpoint.

## Risks / Trade-offs

- **[Risk]** Projection and tool receipt paths can diverge. **Mitigation:** use
  one receipt builder for provider-visible errors and observations, with byte
  and digest recomputation tests.
- **[Risk]** New event fields could silently alter JSON consumers. **Mitigation:**
  serialize only the existing three keys and assert exact wire fixtures.
- **[Risk]** A corrupted historical record could cause unsafe replay.
  **Mitigation:** retain v1's established reprojector and replace malformed v2
  known payloads with fixed withheld events.
