## ADDED Requirements

### Requirement: Native search selector schema parity

The published configuration schema and native parser SHALL recognize `grep`
and `glob` as exact native permission-selector names. They SHALL retain the
existing bounded optional native-file path-pattern grammar and reject unknown
native tool names.

#### Scenario: Search permission rule is published and parsed
- **WHEN** a configuration permits or denies `grep` or `glob` with an anchored
  project-relative pattern
- **THEN** both the JSON schema and native configuration validation accept the
  same rule shape and apply its normal matching semantics.

