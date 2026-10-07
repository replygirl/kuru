# Spec Delta

## ADDED Requirements

### Requirement: Bounded memory-service retention configuration

The native parser and published configuration schema SHALL accept the integer
`memory.service_idle_timeout_secs` from 0 through 300 and SHALL default it to 30.
Zero SHALL explicitly select immediate idle retirement. Invalid values MUST be
rejected before configured memory startup. This preference SHALL NOT grant tool,
provider or workspace authority.

#### Scenario: Idle preference boundaries
- **WHEN** an invocation selects zero, 30 or 300 seconds through an existing configuration layer
- **THEN** both parser and schema accept the same bounded value; negative, noninteger and greater-than-300 values are rejected.
