## ADDED Requirements

### Requirement: Managed configuration policy schema

Kuru SHALL accept a managed TOML document only from an explicitly provisioned absolute path outside the workspace. It SHALL contain optional `defaults` using supported ordinary configuration keys and `constraints` using schema-valid configuration leaves as exact-value locks. Arrays and each named MCP alias table SHALL lock atomically; scalar external-agent endpoints SHALL lock by alias, and empty managed tables SHALL lock the whole empty table. Unknown keys, invalid types and contradictory managed values MUST fail before activation. Ordinary managed defaults SHALL be lower precedence than user and project preferences; constraints SHALL be checked against the final typed configuration after every override and MUST NOT be disabled by those layers.

#### Scenario: Locked shell policy
- **WHEN** a managed constraint locks `allow_shell` to false and a project, local file or CLI input sets it true
- **THEN** configuration fails before a shell or tool host is constructed, with a bounded diagnostic that identifies the constrained setting but not sensitive values.

#### Scenario: Managed ordinary default
- **WHEN** managed defaults select a model and the user selects another model without locking it
- **THEN** the user's model is effective.

#### Scenario: Locked permission array and budget
- **WHEN** managed constraints lock `permissions` and `max_tool_calls`, and a later layer changes either
- **THEN** Kuru rejects the final configuration before tool activation, even if the changed rule set or budget would otherwise be valid.

### Requirement: Typed invocation configuration overrides

Kuru SHALL accept repeatable `-c key=value` inputs for supported dotted configuration leaves. It SHALL parse values using TOML types, merge maps and replace arrays as ordinary layers do, reject unknown paths and invalid values with bounded diagnostics, and record the final writer of each effective leaf as command-line provenance. A dedicated CLI flag for the same leaf SHALL have final precedence.

#### Scenario: Nested typed override
- **WHEN** `-c memory.offline=true` and `-c max_rounds=4` are supplied
- **THEN** the final configuration contains boolean and integer values, and both leaves have command-line provenance.

#### Scenario: Invalid typed override
- **WHEN** an unknown key or a string value for an integer setting is supplied
- **THEN** Kuru rejects it before workspace authority activation without echoing the supplied value.
