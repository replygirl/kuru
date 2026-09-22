## ADDED Requirements

### Requirement: Effective custom entries share the working registry

The invocation-local command registry SHALL combine the built-in catalog with only effective, approved custom prompt entries. Help, leading-token completion and dispatch MUST use that same registry and deterministic collision order. Modal permission/instruction review MUST retain priority over custom-command completion and execution.

#### Scenario: Custom command parity
- **WHEN** an approved custom entry is listed in help, completed from its slash prefix, and invoked
- **THEN** the same name and description identify the captured prompt command at every step.

#### Scenario: Unapproved project entry
- **WHEN** project custom-command authority has not passed exact-root preflight
- **THEN** its name, description and body do not appear in help, completion, dispatch or a provider prompt.
