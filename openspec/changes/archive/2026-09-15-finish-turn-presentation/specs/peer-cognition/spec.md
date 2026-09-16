## MODIFIED Requirements

### Requirement: Resource budgets

The runtime MUST bound parallel model calls, peer rounds and tool calls, and
permit cancellation. `TurnOutput` MUST name tool-call and peer-round exhaustion
separately, while an empty final model response MUST be a separate response
outcome rather than a third budget. The existing `limited` field SHALL remain
for compatibility. A v0.3.2 output carrying only `limited: true` MUST remain
readable with an explicitly unspecified legacy limit reason rather than a
guessed attribution.

#### Scenario: Cyclic peers
- **WHEN** peers repeatedly request each other
- **THEN** processing ends at the configured peer-round budget with that reason observable.

#### Scenario: Tool exhaustion
- **WHEN** a turn attempts more tool calls than configured
- **THEN** processing remains bounded and the output identifies the tool-call limit.

#### Scenario: Empty response
- **WHEN** the speaking loop produces no final text
- **THEN** the output identifies an empty-response outcome separately from both budgets.

#### Scenario: Legacy limited output
- **WHEN** a stored v0.3.2 output has `limited: true` without a typed reason
- **THEN** replay reports a legacy unspecified limit and does not invent a budget.

## ADDED Requirements

### Requirement: Projected semantic turn events

Every semantic turn event MUST cross the shared bounded recognizable-secret
projection before broadcast, returned `TurnOutput`, journal persistence or
journal replay. Structured state, peer and relationship detail SHALL be
projected as JSON before serialization into the existing event detail string so
useful nonsecret fields remain. Text detail SHALL use the shared text projection.
The response event MUST contain only a completion marker; `TurnOutput.text`
remains the authoritative answer. Projection or structured parsing failure MUST
produce a fixed withheld marker and MUST NOT fall back to raw data. This
semantic event contract is separate from the operational JSONL trace.

#### Scenario: Structured secret-bearing event
- **WHEN** state, peer or relationship detail contains recognizable credentials in keys or values
- **THEN** broadcast, JSON output, journal state and retry replay preserve safe structure and contain no raw credential.

#### Scenario: Completed response event
- **WHEN** a turn completes with answer text
- **THEN** that text appears in `TurnOutput.text` and the transcript, while its response event contains only a completion marker.

#### Scenario: Legacy journal replay
- **WHEN** an old journal contains raw structured or textual event detail
- **THEN** replay applies the current projection before any event leaves the runtime without rewriting historical revisions.
