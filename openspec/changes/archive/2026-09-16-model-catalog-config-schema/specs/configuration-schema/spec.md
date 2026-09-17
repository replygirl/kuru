## ADDED Requirements

### Requirement: Published versioned configuration schema

Kuru SHALL publish a versioned JSON Schema asset for the JSON-equivalent shape
of its supported TOML configuration through the existing documentation release
path. The documentation SHALL state that TOML parsing and `Config::validate`
remain authoritative for semantic validation, and that unsupported keys require
a newer Kuru version rather than silently changing authority.

#### Scenario: Supported configuration has a public schema contract
- **WHEN** a user or editor loads `configuration.v1.schema.json`
- **THEN** it SHALL describe the supported root, memory, and alias-keyed MCP and
external-agent configuration structures, including documented defaults and
integer bounds

### Requirement: Strict parser-schema parity

The published schema SHALL reject unknown root, nested memory, and per-MCP-entry
keys and SHALL accept the supported documented configuration examples after
TOML-to-JSON conversion. Its supported key inventory and default values SHALL
remain aligned with `Config` serialization and defaults; model and effort values
SHALL remain open strings rather than catalog-derived enums.

#### Scenario: Unknown authority field is rejected
- **WHEN** configuration contains an unknown root, memory, or MCP-entry key
- **THEN** both schema validation and Kuru's parser SHALL reject it

#### Scenario: Invalid numeric bound is rejected
- **WHEN** configuration supplies a startup timeout or configured fallback window
  outside its documented bounds
- **THEN** both schema validation and Kuru's semantic validation SHALL reject it

#### Scenario: Cross-field rules retain native enforcement
- **WHEN** a configuration violates an MCP command-versus-URL exclusivity rule,
  endpoint restriction, or mode-dependent part-count rule
- **THEN** `Config::validate` SHALL reject it even when the JSON Schema alone
  cannot express the semantic relationship
