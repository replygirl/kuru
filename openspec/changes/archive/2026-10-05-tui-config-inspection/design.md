# Design

## Context

`ConfigSnapshot` already retains the merged invocation layers and final-leaf origins. Its current public `snapshot_toml` method omits saved preferences and does not expose provenance. Saved `ProjectPreferences` are loaded before the TUI starts; the live view also tracks the current mode, model, and effort. The existing command registry currently rejects `/config` as unavailable.

## Goals / Non-Goals

**Goals:**

- Project safe values and final-leaf sources from the same immutable snapshot used to start the session.
- Preserve existing configuration precedence and trust behavior while showing saved and live selections truthfully.
- Keep display work bounded and make every omission visible.

**Non-Goals:**

- Editing configuration files, changing preferences, or adding a generic config editor.
- Rereading configuration or reopening workspace authority during an interactive command.
- Reading environment values or credential stores.

## Decisions

1. **Make `ConfigSnapshot` own the projection.** Add a small public display projection in `kuru-core` that uses the captured value and origin maps, with already-loaded `ProjectPreferences` as input. The alternative—reconstructing layers in `kuru-tui`—would duplicate precedence, expose private config internals, and risk displaying stale or incorrectly attributed values.

2. **Record saved-preference provenance only when it wins.** When `ProjectPreferences::overlay` changes mode, model, or effort, tag only those effective leaves as `saved project preferences`; subsequent local, explicit, typed, and dedicated invocation overrides continue to replace provenance under existing precedence. The alternative—tagging every saved preference regardless of whether it was applied—would misreport values that an explicit config or invocation selection superseded.

3. **Use field-aware redaction in the core projection.** Redact MCP environment values, static values in configured MCP header maps, URL user information and sensitive query values in the actual endpoint URL fields. Keep named environment references visible and never resolve them. Do not use a broad substring match that could hide ordinary capability names or unrelated settings. The alternative—reusing `snapshot_toml` unchanged—would leave configured secrets and sensitive URL values exposed.

4. **Keep current runtime choices in a separate typed view section.** Initialize the live mode/model/effort values from the finalized harness state, then update them only when the corresponding selection operation succeeds. The alternative—rewriting captured provenance after each choice—would conflate startup configuration with later session state.

5. **Bound rows and rendered text explicitly.** Sort projection rows deterministically, enforce fixed row/value/total-text limits, and return counts for omitted or truncated material so the UI can show a visible notice. The alternative—dumping arbitrary-size TOML into the transcript—would make a configuration inspection an unbounded render operation.

6. **Handle `/config` as a local TUI command.** The idle command path renders the stored projection directly, without dispatching through the harness or memory command machinery. Invalid arguments produce a local usage response. The alternative—adding a normal harness command—would needlessly reconcile storage and risk turning inspection into a runtime request.

## Risks / Trade-offs

- [Risk] Redaction rules can lag behind future secret-bearing config fields → Keep projection tests keyed to the current typed configuration fields and require new fields to declare whether their values are safe to display.
- [Risk] A useful full configuration can exceed the TUI's practical display bound → Show an explicit omitted/truncated count and keep source ordering deterministic.
- [Risk] Current selections change after startup → Present them separately as live session state and update only after successful operations.

## Operational surface

`/config` runs inside the existing local Kuru TUI process. It adds no listener,
container, hosted runner, secret requirement, or connection pool. It reads only
the immutable configuration projection captured at startup and the already
available current-session selections; it does not resolve configured
environment references or contact providers or configured endpoints.
