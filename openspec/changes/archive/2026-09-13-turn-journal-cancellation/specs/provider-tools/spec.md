## ADDED Requirements

### Requirement: Turn cancellation stops dispatch admission

Provider completion, built-in tool, MCP, and outbound A2A dispatch initiated by a turn MUST observe the same explicit cancellation signal before admission and while awaiting a cancellable response. Cancellation MUST NOT replay an ambiguous request. It MUST release logical runtime permits and locks while connector-owned subprocess workers retain responsibility for bounded cleanup and honest unconfirmed state.

#### Scenario: Cancel before dispatch
- **WHEN** cancellation becomes visible before an actor, provider, tool, MCP alias, or A2A request is admitted
- **THEN** that operation sends no request or process input and the next valid operation can proceed.

#### Scenario: Cancel after observed dispatch
- **WHEN** a local fixture observes one provider, shell, MCP, or A2A dispatch before cancellation
- **THEN** Kuru sends it at most once, records the turn as interrupted and possibly dispatched, and leaves any process cleanup with its existing retained owner.

#### Scenario: Accepted memory operation during cancellation
- **WHEN** a cognitive state or note mutation is accepted before cancellation is observed
- **THEN** the mutation is completed or reconciled before interruption is recorded and a later mutation begins.
