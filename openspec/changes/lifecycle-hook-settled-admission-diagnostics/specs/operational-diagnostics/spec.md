## MODIFIED Requirements

### Requirement: Redacted asynchronous turn observations

Runtime and connector code SHALL emit subscriber-neutral asynchronous spans/events for turns, actor work, external tool execution, tool calls settled by pre-dispatch admission, and typed retry decisions. A turn's tool call that the runtime settles before dispatch — because its name was not offered for that request and phase, or because a pre-tool hook denied it, failed or returned an invalid rewrite — MUST leave one tool record with the `admission` operation, its tool category and its status, and MUST NOT record the tool name or a hook's reason. Records MUST use the existing domain-separated journal digest for turn correlation and MAY contain only stable identifiers, literal operation/status categories, attempt, elapsed time, and already-available byte or token counts. They MUST NOT contain prompts, histories, tool arguments/results, configuration, endpoints, headers, raw caller turn IDs, or error chains.

#### Scenario: Retry and tool result
- **WHEN** a fake provider retries and actor work invokes a successful or failing tool
- **THEN** correlated records contain typed retry and tool status observations without their fake secret inputs or outputs.

#### Scenario: Cancellation
- **WHEN** an asynchronous turn is cancelled
- **THEN** its turn and child observations close without a long-lived entered span guard across awaits.

#### Scenario: Unoffered tool call settled before dispatch
- **WHEN** a speaking actor proposes a tool call whose name was not offered for that request and the runtime settles it without dispatch
- **THEN** the diagnostics ring holds a `kuru.tool` record with the `admission` operation and `error` status, and no record contains the proposed tool name.
