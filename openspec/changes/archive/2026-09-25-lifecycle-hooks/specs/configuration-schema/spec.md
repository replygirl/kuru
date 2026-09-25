## ADDED Requirements

### Requirement: Lifecycle-hook schema parity

The native configuration parser and published `configuration.v1.schema.json` SHALL accept the same strict bounded hook tables for `pre_turn`, `post_turn`, `pre_tool`, `post_tool`, and `speaker_selected`. Each entry SHALL contain only the documented command, argument, and optional bound fields; arrays SHALL retain declaration order and follow existing layer replacement and managed-constraint semantics. Both paths MUST reject unknown events or fields, empty commands, invalid arguments, unsupported result options, and values outside the documented count, byte, or duration bounds before hook activation.

#### Scenario: Documented hook chain is accepted

- **WHEN** a documented ordered pre-tool and post-tool chain is parsed from TOML and its JSON-equivalent is checked against the published schema
- **THEN** both accept the same entries, bounds, defaults, and declaration order

#### Scenario: Invalid hook authority is rejected

- **WHEN** a hook entry contains an unknown field, an empty command, or an out-of-range output limit
- **THEN** native validation and schema validation reject it before workspace review or process startup
