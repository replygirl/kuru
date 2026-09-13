## Why

An admitted turn currently has no durable identity or terminal status. Losing a caller or process can therefore leave a user prompt, private actor history, or external effect behind with no safe way to tell whether retrying would repeat work.

Cancellation is also expressed by aborting the caller task. That stops the returned future but cannot reliably record interruption, distinguish it from a completed answer, or bound shutdown dreaming while existing connector owners finish their own cleanup.

## What Changes

- Give controlled turns a caller-supplied bounded ID and store durable started, ended, and interrupted state using existing application state rows.
- Commit admission with exactly one early user transcript row, and commit completion with exactly one assistant row, session/topology/index state, and the authoritative `TurnOutput`.
- Return a stored completed result without provider or tool dispatch, reject mismatched ID reuse, and refuse automatic replay after possibly external dispatch.
- Propagate one explicit cancellation token through actor admission, provider waits, tools, cognitive operations, A2A, dreaming, and the TUI without abandoning accepted memory writes or owned process cleanup.
- Bound shutdown dreaming and always attempt normal actor and tool-host cleanup after dream cancellation, timeout, or failure.
- Keep the existing `TurnOutput` JSON shape. Freeze its event trace at the answer boundary; later dream events remain live maintenance activity and cannot suppress a completed answer.

## Capabilities

### New Capabilities

### Modified Capabilities

- `chat-harness`: Define controlled turn IDs, authoritative completed retry, explicit interactive cancellation, the answer boundary, and bounded shutdown dreaming.
- `versioned-memory`: Define the two atomic transcript-and-state journal checkpoints and recovery of started, interrupted, possibly dispatched, and completed turns using existing schema.
- `provider-tools`: Define cancellation before provider/tool/A2A admission, conservative no-replay after possible dispatch, and independent cleanup ownership.

## Impact

This changes runtime turn and dream orchestration, actor work messages, TUI dispatch cancellation, the A2A turn entrypoint, and one narrow `kuru-memory` atomic message-and-state method. It adds no table, schema migration, general transaction API, provider retry, external exactly-once claim, dependency, configuration, or JSON field.

## Surfaces

- [x] interactive — cancellation and completion races affect terminal conversation state.
- [ ] deploy — no deployment, workflow, credential, or runtime-topology change.
- [x] integration — provider, MCP, shell, and A2A dispatch cancellation and retry boundaries change.
- [x] agent-behavior — cancelled work stops further actor and tool admission while completed retries preserve the authoritative answer.
