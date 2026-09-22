## ADDED Requirements

### Requirement: MCP catalog-control schema parity

The native parser and published configuration schema SHALL define the same bounded MCP enabled flag, original-tool-name allow and deny glob arrays, and HTTP static-header environment-reference map. Existing MCP configurations SHALL remain enabled by default. Both paths MUST reject unknown fields, invalid globs, invalid header or environment names, reserved protocol headers, excess entries, and static headers on a stdio transport.

#### Scenario: Bounded HTTP catalog controls are valid
- **WHEN** configuration declares an HTTP MCP alias with an explicit enabled value, valid allow and deny globs, and valid header-to-environment references within every bound
- **THEN** the native parser and published schema accept the same document and its effective projection contains references but no resolved header value

#### Scenario: Literal or reserved header declaration is invalid
- **WHEN** an MCP header declaration uses an invalid environment reference, a reserved MCP or HTTP framing header, exceeds a bound, or is attached to a stdio server
- **THEN** the native parser and published schema reject it without resolving or printing a secret value

