## ADDED Requirements

### Requirement: Typed native tool continuations

The connector SHALL translate typed tool-use and tool-result blocks without
re-parsing new receipt prose. Live native continuation MUST validate call IDs,
retain pending call order and preserve current string versus JSON result encoding.
Legacy tool receipt strings MAY be adapted only at the established pending-call
boundary with matching call identity; arbitrary historical JSON-looking text
MUST NOT become executable tool structure. Native opaque reasoning continuation
MUST remain transient and scoped to its producing actor, never reconstructed
from durable history or shared with another actor. Completed text projection and
tool ordering MUST retain existing behavior across both native OpenAI routes.

#### Scenario: Multiple current receipts

- **WHEN** a completion requests multiple tool calls and matching typed receipts arrive in a different order
- **THEN** the next native request sends each result once in pending call order and retains that actor's exact native continuation.

#### Scenario: Receipt mismatch or actor isolation

- **WHEN** a receipt lacks a pending call, duplicates an ID, or belongs to another actor
- **THEN** it cannot complete a different actor's native continuation or bypass validation.

#### Scenario: Legacy text after a new turn

- **WHEN** a new user turn follows a historical JSON-looking tool record without a live pending continuation
- **THEN** the old record remains historical data and does not reconstruct opaque reasoning or authorize tool execution.
