# configuration-schema Specification

## Purpose
Publish a versioned, strict structural contract for Kuru configuration while TOML parsing and native semantic validation remain authoritative.

## Requirements

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

### Requirement: Bounded context configuration parity

P7 SHALL reuse the existing bounded `assumed_context_window_tokens` setting and add only an optional bounded `context_output_reserve_tokens` override for fitting. The native configuration parser and published `configuration.v1.schema.json` SHALL accept the same supported fields, reject unknown keys and out-of-range values, and retain ordinary model and effort values as open strings. An override SHALL not grant tool authority or mutate the frozen provider route; an output reserve greater than the selected effective window SHALL cause a pre-dispatch fit refusal.

#### Scenario: Explicit window override
- **WHEN** a user configures a valid context window for an unfamiliar model
- **THEN** fit uses that explicit bound with its configured provenance and both parser and published schema accept the equivalent configuration.

#### Scenario: Invalid bound
- **WHEN** the window or reserve is outside documented bounds or uses an unsupported key
- **THEN** native validation and published schema both reject the configuration.

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
