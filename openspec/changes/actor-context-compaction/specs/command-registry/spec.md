## ADDED Requirements

### Requirement: Manual compaction is a working registered command

The shared invocation-local command registry SHALL expose `/compact [ID]` only with the real actor-context compaction backend. Help, leading-token completion and dispatch MUST use that same definition. The handler MUST remain a local runtime control, MUST NOT send the slash text as a provider prompt, and MUST retain permission/instruction modal priority and the current session identity.

#### Scenario: Registered compact command
- **WHEN** a user views help, completes `/comp`, then submits `/compact` or `/compact ID`
- **THEN** the same usage is shown at each surface and the bounded manual compaction handler runs without an ordinary prompt turn.

#### Scenario: Invalid target
- **WHEN** `/compact ID` names an inactive, malformed or unknown identity
- **THEN** the command returns an actionable error before snapshot, provider dispatch or cursor mutation.
