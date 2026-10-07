# Spec Delta

## ADDED Requirements

### Requirement: Strict layered presentation configuration

The native parser and published configuration schema SHALL accept the same
`ui.theme` dark/light selector and `ui.palette` overrides for the finite semantic
role set. Overrides MUST be hexadecimal #RRGGBB colors; unknown UI fields, role
names, theme names and malformed colors MUST fail bounded validation. Existing
layer precedence, typed invocation overrides, final-leaf provenance and managed
constraints SHALL apply unchanged. Presentation settings MUST NOT grant authority
or affect saved framework/provider/model selections.

#### Scenario: Layered theme override
- **WHEN** ordinary files select one theme and an explicit local or typed invocation override selects another
- **THEN** the invocation uses the final theme with its correct provenance, while enforced managed constraints remain effective.

#### Scenario: Invalid presentation settings
- **WHEN** a theme, semantic role or RGB color is unsupported or malformed
- **THEN** native parsing and published schema both reject it before presentation starts without exposing unrelated configuration values.
