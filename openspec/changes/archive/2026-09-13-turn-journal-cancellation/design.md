## Context

`Harness::run_for` currently appends the public user message before work, writes actor histories around each provider call, appends the assistant message, persists session/topology, emits the response, awaits periodic dream, and only then returns `TurnOutput`. Memory mutations already run in independent receipt-bearing workers and reconcile uncertain acknowledgements. The TUI cancels by aborting its dispatch task, while shell and MCP now retain process ownership independently.

The change spans runtime, memory, connectors, A2A, and the terminal, so the proposal's two durable boundaries and one cancellation signal need an explicit shared implementation cut.

## Goals / Non-Goals

**Goals:** Keep interrupted prompts in normal conversation history; make completed retries exact and side-effect-free; surface uncertainty before any actor work; preserve the current public output shape; and stop cancelled work promptly without weakening memory or subprocess ownership.

**Non-Goals:** A workflow engine, generic transaction/event/token framework, external exactly-once delivery, automatic replay of ambiguous calls, a new memory table or migration, journal GC, or reconstruction of pre-journal turns.

## Decisions

### Existing state rows hold a bounded per-turn journal

Store one versioned value beneath the canonical project and session scope, addressed by a SHA-256 path component derived from the caller ID. The identity tuple is `(project, session, turn ID)`; no project-global ID index is added, and another session may use the same ID independently. Retain the original bounded ID in the value and compare it plus the exact prompt and raw target within that session, avoiding raw external IDs in storage keys. The value holds the request, ordered transitions, possible-dispatch flag, and optional `TurnOutput`. Existing state rows were chosen over a new table because they already preserve opaque JSON and Dolt history and require no migration or export inventory. Overwriting only a terminal status was rejected because current export must retain the lifecycle history.

### Admission and completion are separate atomic message/state mutations

Add one narrow memory method that appends supplied messages to one validated namespace and upserts supplied state values in the existing receipt-bearing transaction. Admission uses it for the started value and one user row. Completion uses it for the ended value, one assistant row, session/topology/index, and exact output. This preserves current prompt timing and makes the ended journal value proof that the full completion batch committed. A general SQL transaction API and delayed user transcript were rejected because they widen storage authority or lose interrupted conversational history.

Write the possible-dispatch transition before sending `Work` to an actor mailbox. All turns call a provider, so this one conservative marker also covers later tool and A2A work. It may report uncertainty when cancellation won just before native dispatch; that false positive is safer and smaller than claiming remote exactly-once behavior.

### Cancellation and turn identity remain separate

Expose one small cloneable cancellation token plus a controlled run method accepting a separate bounded turn ID. Existing run methods create both internally. A2A uses its inbound message ID; the TUI retains the token beside the active job. Runtime-private operation context carries journal state and the token. Provider and ToolHost public traits remain unchanged: runtime selects cancellation around their existing futures, while actor `Work` observes it during queue/semaphore/provider stages.

Cancellation is checked before memory mutation admission, but an accepted write is awaited and reconciled before interruption. Normal TUI cancellation signals and consumes the typed job result rather than blindly aborting it. If completion won, it renders the answer; otherwise the durable interruption controls the display. Caller/process loss can still drop a future, leaving journal recovery to report started or possible state.

### Completion freezes output before maintenance dreaming

Prepare the response event and `TurnOutput` without broadcasting, commit that exact output in the ended checkpoint, publish reconciled runtime state, then broadcast the response. Preserve all existing fields and serialization. The stored event vector ends at the response event; periodic dream activity is broadcast after the answer boundary and cannot alter a retry. Dream remains sequenced after the answer but observes cancellation, and its failure cannot replace the completed result.

Shutdown supplies a separate cancellation token and aggregate dream deadline, then always aborts/awaits actors through their existing boundary and shuts down ToolHost. Timeout is reported alongside cleanup failure when both occur; it never proves cleanup.

## Risks / Trade-offs

- [A durable possible marker can precede an operation that never reaches its peer] → report uncertainty and require a new turn instead of risking duplicate external effects.
- [Journal values retain full prompts and outputs] → keep current prompt/output bounds, scope them to the session, treat them as exported user content, and add no expiry in this change.
- [Cancellation can race the ended checkpoint] → decide by the reconciled ended value and always prefer its stored output over an interruption display.
- [Post-answer dream events no longer appear in returned `events`] → retain their live broadcast visibility and document that `TurnOutput.events` freezes at answer completion.
- [Dropping connector futures does not prove native cleanup] → preserve the independent registered shell/MCP owners and their explicit shutdown evidence.

## Operational surface

This remains one local foreground Kuru process with its managed loopback Dolt sidecar and optional existing loopback A2A listener. It adds no bind address, container, daemon, credential, connection pool, binary, target architecture, or native runtime. Journal state stays in the current project database outside the tool root. Tests use isolated stores, fake providers, loopback protocol peers, and real native process/PTTY fixtures without live credentials.

## Integration contract

The controlled library entrypoint accepts a 1–256 byte turn ID separately from the cancellation token. Storage addresses it beneath the current session through a domain-separated SHA-256 component and retains the exact original string, prompt, and raw target for comparison. A2A maps the validated inbound `messageId` to that session-scoped ID; the TUI and default library wrappers generate UUIDs. The runtime adds no global ID registry and no field to Provider, ToolHost, MCP, A2A wire, or `TurnOutput`.

Cancellation drops only the containing provider/tool/A2A future after the durable possible marker. Existing connector workers continue shell/MCP cleanup. Deterministic fake providers identify sends, HTTP peers count complete requests, stdio peers count framed writes, and memory fault/process fixtures inspect committed state and transcript rows. Native fixture assertions remain specific to Unix process groups or Windows Jobs rather than inferring parity.
