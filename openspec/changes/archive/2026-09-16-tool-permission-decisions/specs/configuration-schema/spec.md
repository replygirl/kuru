## ADDED Requirements

### Requirement: Strict permission-rule schema parity

The published versioned configuration schema and native TOML parser SHALL accept a bounded `permissions` array whose entries have only action `allow`, `ask` or `deny`, a stable exact tool selector and an optional anchored project-relative file pattern. They SHALL reject unknown rule keys, invalid actions/selectors, absolute or traversing patterns and an unsupported pattern grammar; documented examples SHALL validate in both. Existing config layering SHALL replace a lower-priority rule array with a higher-priority one rather than merging entries, and unknown configuration keys SHALL remain errors.

#### Scenario: Published rule example is accepted
- **WHEN** a documented native or MCP rule is parsed from TOML and the equivalent JSON is checked against the published schema
- **THEN** both accept the same shape without turning a tool selector into a free-form description match.

#### Scenario: Invalid rule is rejected on both paths
- **WHEN** a rule contains an unknown key, action or selector, or an absolute or traversing file pattern
- **THEN** schema validation and native parsing/validation reject it before tool activation.

#### Scenario: Higher-priority rule array replaces lower-priority entries
- **WHEN** layered configuration supplies a later `permissions` array
- **THEN** only that effective array participates in matching and workspace manifest derivation.
