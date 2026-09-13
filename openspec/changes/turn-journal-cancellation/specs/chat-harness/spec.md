## ADDED Requirements

### Requirement: Durable idempotent turn lifecycle

The harness SHALL offer a controlled turn entrypoint with a bounded caller-supplied turn ID and an explicit cancellation token while retaining convenience entrypoints that create fresh IDs and tokens. Turn identity SHALL be scoped to the canonical project and session. Within that scope the harness MUST compare the exact prompt and raw target before reusing an ID. A completed matching ID MUST return its stored authoritative `TurnOutput` without another provider, tool, cognitive, or A2A dispatch; mismatched reuse and a possibly dispatched incomplete turn MUST fail explicitly rather than replay. The same ID MAY identify an independent turn in another session.

#### Scenario: Completed retry
- **WHEN** a caller repeats a completed turn ID with the same session, prompt, and target
- **THEN** the harness returns the same authoritative output without another provider or tool invocation or another transcript row.

#### Scenario: Mismatched or uncertain retry
- **WHEN** a caller reuses an ID for a different request or for an incomplete turn that may have dispatched external work
- **THEN** the harness reports the mismatch or uncertain status and performs no provider, tool, cognitive, or A2A dispatch.

#### Scenario: Session-scoped reuse
- **WHEN** two sessions in the same project use the same turn ID for independent requests
- **THEN** each session journals and completes its own turn without a global ID conflict.

### Requirement: Explicit cancellation and answer boundary

The runtime SHALL propagate one cancellation signal through turn admission, actor queue and semaphore waits, provider asks, built-in and MCP tools, cognitive calls, A2A, and dreaming. Once a memory mutation is accepted, it MUST finish or reconcile before another mutation; owned subprocess cleanup MUST remain independent of the cancelled caller. A turn SHALL become complete at its durable ended checkpoint before response publication and periodic dreaming. Cancellation after that boundary MUST preserve and return the completed answer. `TurnOutput` SHALL retain its existing JSON shape, with its event trace frozen at the answer boundary; later dream events are maintenance activity.

#### Scenario: Cancellation loses or wins the completion race
- **WHEN** interactive cancellation settles before the ended checkpoint
- **THEN** the TUI reports an interrupted turn whose user prompt remains in conversation history and permits the next operation.
- **WHEN** the ended checkpoint settles first
- **THEN** the TUI displays the authoritative answer even if cancellation or periodic dream shutdown follows.

#### Scenario: Post-answer dream stops
- **WHEN** periodic dreaming fails or is cancelled after the ended checkpoint
- **THEN** the completed `TurnOutput` remains retrievable and dream diagnostics appear only as later maintenance activity.

### Requirement: Bounded shutdown dreaming

Shutdown SHALL give optional dreaming one finite aggregate deadline and SHALL attempt normal actor and tool-host cleanup after dream success, failure, cancellation, or timeout. A deadline or channel closure MUST NOT be treated as proof that an owned subprocess was cleaned.

#### Scenario: Shutdown dream exceeds its deadline
- **WHEN** a provider stalls during shutdown dreaming
- **THEN** Kuru cancels that dream, attempts actor and tool-host cleanup, and returns an honest bounded failure if cleanup or dreaming remains unconfirmed.
