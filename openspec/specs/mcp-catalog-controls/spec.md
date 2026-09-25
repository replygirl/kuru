# mcp-catalog-controls Specification

## Purpose
Define how Kuru activates, filters, caches, and reports configured MCP tool catalogs while keeping persisted discovery metadata separate from live routing and execution authority. It also bounds environment-referenced static HTTP headers and the shared CLI/TUI status projection without adding a credential store or broader MCP session control plane.

## Requirements

### Requirement: MCP servers are admitted and filtered before activation

Kuru SHALL treat each configured MCP server's enabled state and original-tool-name filters as activation policy. A disabled alias MUST NOT spawn, connect, resolve header values, or publish cached tools. A denied tool MUST be omitted, deny matches MUST take precedence over allow matches, and a non-empty allow set MUST omit every unmatched tool before route or provider-facing tool construction.

#### Scenario: Disabled server is inspected
- **WHEN** a configured MCP alias is disabled and the user or runtime requests the tool catalog
- **THEN** Kuru reports the alias as disabled without starting its process, opening its endpoint, resolving its header references, publishing its cached tools, or creating an executable route

#### Scenario: Allow and deny globs overlap
- **WHEN** one original MCP tool name matches both an allow glob and a deny glob
- **THEN** Kuru omits that tool from the live and cached projections and cannot route a call to it

#### Scenario: Nonempty allow set is selective
- **WHEN** an enabled server has at least one allow glob
- **THEN** only original tool names matching an allow glob and no deny glob enter its catalog

### Requirement: Cached MCP catalogs remain metadata only

Kuru SHALL persist only bounded, versioned, owner-private MCP discovery metadata bound to the exact configured alias, canonical workspace authority context, endpoint, filters, and static-header authentication context. Valid cached metadata MAY be projected as explicitly stale after live discovery fails, but it MUST NOT create a live route, satisfy an availability check, or grant execution. Incompatible, over-limit, malformed, or context-mismatched cache state MUST fail closed for stale projection while a normal authorized live discovery remains able to replace it.

#### Scenario: Restart while a server is offline
- **WHEN** a previously discovered enabled server is unavailable after restart and its cache entry is valid for the current authority context
- **THEN** Kuru projects the filtered metadata as stale, reports the alias unavailable, and refuses execution until a current live discovery recreates the route

#### Scenario: Authority or authentication context changed
- **WHEN** the canonical workspace, reviewed authority, endpoint, filters, header references, or resolved header values differ from the context that wrote a cache entry
- **THEN** Kuru does not project or route that entry

#### Scenario: Corrupt cache and healthy server
- **WHEN** an alias has malformed, incompatible, hostile, or over-limit cached bytes but its configured server can be discovered live
- **THEN** Kuru ignores the invalid cache, completes bounded live discovery, and replaces it with a valid entry without requiring manual cache repair

### Requirement: MCP static headers use environment references

Kuru SHALL accept bounded HTTP MCP static-header declarations whose configured values are environment-variable names rather than literal header values. Kuru MUST validate header names, environment names, reserved protocol-header exclusions, and resolved values before use; apply the resolved headers to initialize, discovery, tool call, protocol-version, and session-close requests; and keep resolved values out of configuration projections, status, diagnostics, errors, and catalog caches.

#### Scenario: Header applies across one HTTP session
- **WHEN** an enabled HTTP MCP alias resolves a valid static-header environment reference and completes initialization, discovery, one call, and session close
- **THEN** the exact resolved header is present on every request while Kuru continues to own the protocol-managed headers

#### Scenario: Header reference is unusable
- **WHEN** a referenced environment variable is absent, non-Unicode, contains an invalid header value, or names a protocol-managed header
- **THEN** that alias degrades without a request and no diagnostic or persisted record contains the resolved value

### Requirement: MCP catalog status is one bounded projection

Kuru SHALL project each configured alias as exactly one of disabled, live, stale, or degraded together with only its filtered bounded metadata and safe diagnostic. The CLI and shared TUI command registry MUST use this projection; status MUST distinguish cached metadata from current availability.

#### Scenario: Mixed catalog health
- **WHEN** enabled aliases are respectively live, offline with valid cached metadata, and failed without usable metadata while another alias is disabled
- **THEN** one catalog operation reports each state independently and retains tools only from the live and valid stale filtered projections

### Requirement: MCP authorization state remains alias local and non-authoritative

The existing MCP catalog projection SHALL add a bounded authorization state for each OAuth-enabled HTTP alias that distinguishes login required, authorized, refresh/relogin required, native-store unavailable and authorization failure without exposing tokens, client secrets, callback codes or raw remote bodies. Authorization state MUST NOT make cached metadata live, grant tool permission, start a provider turn, authorize another alias, or hide the existing disabled/live/stale/degraded availability state. Static-header aliases SHALL keep their existing header/session behavior and MUST NOT inherit OAuth credentials.

#### Scenario: Mixed static and OAuth aliases are inspected
- **WHEN** one HTTP alias is live through static headers, another requires OAuth login, a third has stale metadata and an expired credential, and a fourth is disabled
- **THEN** one catalog/status operation reports each availability and authorization state independently without resolving disabled credentials, creating routes from stale metadata, or printing secret material
