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

### Requirement: MCP catalog-control schema parity

The native parser and published configuration schema SHALL define the same bounded MCP enabled flag, original-tool-name allow and deny glob arrays, and HTTP static-header environment-reference map. Existing MCP configurations SHALL remain enabled by default. Both paths MUST reject unknown fields, invalid globs, invalid header or environment names, reserved protocol headers, excess entries, and static headers on a stdio transport.

#### Scenario: Bounded HTTP catalog controls are valid
- **WHEN** configuration declares an HTTP MCP alias with an explicit enabled value, valid allow and deny globs, and valid header-to-environment references within every bound
- **THEN** the native parser and published schema accept the same document and its effective projection contains references but no resolved header value

#### Scenario: Literal or reserved header declaration is invalid
- **WHEN** an MCP header declaration uses an invalid environment reference, a reserved MCP or HTTP framing header, exceeds a bound, or is attached to a stdio server
- **THEN** the native parser and published schema reject it without resolving or printing a secret value

### Requirement: MCP OAuth configuration has strict parser-schema parity

The native parser and published configuration schema SHALL define the same bounded optional OAuth table for HTTP MCP aliases, including explicit enablement, configured client identity, optional environment reference for a preregistered client secret, optional HTTPS Client ID Metadata Document identity and a bounded scope allowlist. A nonempty allowlist MUST constrain rather than override authoritative challenge scopes and MUST restrict protected-resource metadata fallback and later step-up unions; the default empty list adds no configuration ceiling. Both paths MUST reject unknown fields, literal client secrets or tokens, invalid or duplicate scopes, non-HTTPS metadata identities, mutually incompatible registration choices, OAuth on STDIO, and OAuth combined with static `Authorization` or `Proxy-Authorization` header references. Other validated static headers MAY remain configured for protected MCP resource requests but MUST NOT be forwarded to OAuth discovery, registration, authorization, device, token or revocation endpoints. OAuth configuration and its final-leaf provenance SHALL participate in the alias's reviewed workspace authority and catalog context without resolving a secret before trust preflight; a disabled alias MAY retain valid OAuth configuration but MUST NOT resolve its secret reference, discover authority or access the native store.

#### Scenario: Configured public client is valid
- **WHEN** an enabled HTTP MCP alias declares a bounded preregistered public client and OAuth scopes with no literal secret
- **THEN** the native parser and published schema accept the same shape and the authority projection identifies the registration fields without credential values

#### Scenario: Secret or incompatible flow is configured
- **WHEN** an alias contains a literal token/client secret, combines incompatible client-registration choices, requests OAuth on STDIO, combines OAuth with a static authorization header, or uses an unknown or over-limit field
- **THEN** parsing or semantic validation fails before connector construction and does not echo a supplied secret value

#### Scenario: OAuth alias retains a non-authorization resource header
- **WHEN** an HTTP OAuth alias maps a validated non-authorization header to a trusted environment reference
- **THEN** the parser and schema accept it for protected MCP resource requests while the connector withholds it from every OAuth authority endpoint

### Requirement: Lifecycle-hook schema parity

The native configuration parser and published `configuration.v1.schema.json` SHALL accept the same strict bounded hook tables for `pre_turn`, `post_turn`, `pre_tool`, `post_tool`, and `speaker_selected`. Each entry SHALL contain only the documented command, argument, and optional bound fields; arrays SHALL retain declaration order and follow existing layer replacement and managed-constraint semantics. Both paths MUST reject unknown events or fields, empty commands, invalid arguments, unsupported result options, and values outside the documented count, byte, or duration bounds before hook activation.

#### Scenario: Documented hook chain is accepted

- **WHEN** a documented ordered pre-tool and post-tool chain is parsed from TOML and its JSON-equivalent is checked against the published schema
- **THEN** both accept the same entries, bounds, defaults, and declaration order

#### Scenario: Invalid hook authority is rejected

- **WHEN** a hook entry contains an unknown field, an empty command, or an out-of-range output limit
- **THEN** native validation and schema validation reject it before workspace review or process startup
