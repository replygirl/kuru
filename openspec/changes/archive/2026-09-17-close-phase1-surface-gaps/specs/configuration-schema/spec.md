## MODIFIED Requirements

### Requirement: Strict parser-schema parity

The published schema SHALL reject unknown root, nested memory, and per-MCP-entry
keys and SHALL accept the supported documented configuration examples after
TOML-to-JSON conversion. Its supported key inventory and default values SHALL
remain aligned with `Config` serialization and defaults; model and effort values
SHALL remain open strings rather than catalog-derived enums. A parser rejection
of an unknown key SHALL carry the documented forward-compatibility policy — that
unknown keys are rejected, that a configuration needing a new key requires a
newer Kuru version, and that authority never changes silently — without echoing
the offending key, and an in-range key with an invalid value SHALL NOT borrow
that message.

#### Scenario: Unknown authority field is rejected
- **WHEN** configuration contains an unknown root, memory, or MCP-entry key
- **THEN** both schema validation and Kuru's parser SHALL reject it, and the
  parser's message SHALL state the documented forward-compatibility policy

#### Scenario: Invalid numeric bound is rejected
- **WHEN** configuration supplies a startup timeout or configured fallback window
  outside its documented bounds
- **THEN** both schema validation and Kuru's semantic validation SHALL reject it
  without presenting it as a forward-compatibility rejection

#### Scenario: Cross-field rules retain native enforcement
- **WHEN** a configuration violates an MCP command-versus-URL exclusivity rule,
  endpoint restriction, or mode-dependent part-count rule
- **THEN** `Config::validate` SHALL reject it even when the JSON Schema alone
  cannot express the semantic relationship
