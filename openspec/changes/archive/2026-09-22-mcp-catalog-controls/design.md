## Context

`McpHosts` currently owns one client per configured alias, discovers every alias on demand, publishes routes only after a valid live list response, and degrades failures independently. `ToolHost` then intersects the catalog with the permission advertisement and uses the same route to derive a stable permission selector. There is no persistent catalog state, per-server activation control, or HTTP header reference. The application already selects an owner-private data directory and retains a reviewed canonical workspace and authority manifest.

P12 must extend those boundaries without turning cached discovery into authority, adding a credential store, or changing the MCP session lifecycle into a general control plane.

## Goals / Non-Goals

**Goals:**

- Keep activation, catalog visibility, live routing, and execution permission as separate checked decisions.
- Retain bounded useful metadata across restart and make its stale state explicit.
- Carry configured HTTP header values only from environment references through the existing HTTP session.
- Give `kuru tools` and `/tools` one typed catalog/status projection.

**Non-Goals:**

- OAuth, keychain token storage, MCP resources, prompts, roots, progress, or sampling.
- Background health polling, automatic call replay, or a transferable permission grant.
- A generic cache service or MCP session-management API.

## Decisions

### 1. Compile activation policy before client construction

`McpConfig` gains a default-true enabled flag, bounded `allow_tools` and `deny_tools` original-name globs, and an HTTP-only `header_env` map. Configuration validation compiles the glob language and validates header names, environment names, and reserved-header exclusions before runtime activation. `McpHosts` retains disabled aliases as status records but does not construct a client, resolve their environment references, load their cached tool metadata, spawn, or connect.

Filtering happens on original server names immediately after bounded discovery validation and before provider names, permission selectors, routes, or cache entries are built. Deny wins globally; an empty allow set admits unmatched names while a non-empty set requires an allow match.

Rejected alternative: publish everything and enforce filters only at call time. That leaks denied metadata to the provider and leaves stale generated routes that are easy to mistake for authority.

### 2. Model live routes and cached metadata separately

Each alias projects one state: disabled, live, stale, or degraded. Only a successful current discovery can install routes or set the client available. A valid cache can supply filtered `ToolSpec` metadata after live discovery fails, but those descriptions are marked stale and no route is installed. A call against stale metadata therefore fails before permission consumption or transport dispatch.

Rejected alternative: rebuild routes from cache and recheck the server only when called. That makes persisted metadata a current-availability claim and expands the ambiguous-dispatch boundary.

### 3. Use a small connector-owned private catalog store

The application injects a connector `McpCatalogStore` rooted under its already selected owner-private data directory and bound to the retained canonical project. The store uses checked private directories and bounded versioned per-alias records. Each record is limited by encoded bytes, tool count, name/description/schema size, and supported schema version before allocation or projection.

The context binding covers cache format, configured alias, canonical workspace identity, reviewed authority-manifest digest, normalized transport endpoint, enabled/filter configuration, static-header names and environment references, and a domain-separated digest of each resolved header value. The raw resolved value is never serialized or used in diagnostics. A changed component makes the record ineligible rather than migrating it in place.

Cache read errors and invalid records are alias-local metadata misses. Live discovery proceeds normally and, on success, publishes a complete replacement through checked private staging. Thus corruption cannot become a permanent outage. Cache write failure does not revoke a valid live catalog, but is reported as a safe degraded-cache diagnostic.

Rejected alternative: cache inside the workspace or Dolt. The workspace is a tool root with unrelated authority, while memory storage should not become MCP configuration state. A process-global cache would also mix reviewed roots.

### 4. Resolve static headers inside the HTTP client boundary

The config stores `header name -> environment variable name`. For an enabled HTTP alias, client activation reads the named variable, validates it as one bounded HTTP header value, and retains it only in memory. Kuru rejects names it owns for framing, negotiation, host selection, or MCP session identity. The unchanged HTTP request builder applies approved static headers to initialize, list, call, protocol-version, and DELETE close requests while continuing to set Kuru-owned protocol headers itself.

Missing or invalid references degrade only the alias before a request. Errors name the alias/header/reference category but never the environment value. Configuration projection may show the reference name; status, cache, protocol errors, and diagnostics never show the resolved value.

Rejected alternative: literal header values or string interpolation in configuration. Both enlarge the configuration secret surface and can leak through ordinary projection and provenance reporting.

### 5. Keep status and commands as a projection of connector state

`McpCatalog` returns filtered specs plus typed alias status, including whether metadata is live or stale and a safe diagnostic. `ToolCatalog` carries that unchanged. The existing `kuru tools` command and a new working `/tools` registry entry render the same bounded projection. `/tools` performs catalog inspection only; it does not start a provider turn or acquire execution permission.

Rejected alternative: add TUI-only catalog state or a second dispatch table. That would recreate the divergence the shared command registry removed.

## Risks / Trade-offs

- [Stale metadata can invite a call that cannot run] → Mark cached specs and alias status as stale, install no route, and return the existing bounded unavailable result before dispatch.
- [A cache fingerprint derived from a header value could expose information] → Store only a domain-separated digest inside an owner-private record, never render it, and treat any mismatch as a miss; raw values remain process memory only.
- [Hostile discovery can amplify persistent storage] → Bound the wire body, validated tool count and fields, encoded record size, aliases, and total selected records before publication.
- [Cache publication can fail after live discovery] → Keep the live route usable, report only the cache persistence failure, and retry replacement on later explicit discovery.
- [Headers could override protocol ownership] → Reject reserved names during config validation and construct all requests through one audited HTTP helper.

## Operational surface

P12 adds no listener, bind address, daemon, container, or runner topology. Configured stdio commands and HTTP URLs keep their existing launch and connection limits, timeout/body budgets, retained workspace, and supported MCP protocol versions. The only required secret input is an existing process environment variable named by `header_env`; no CI secret name, credential file, login flow, or product default is added. Disabled aliases perform no transport setup. Cache files live under the application-selected owner-private data directory and have fixed per-record/tool/field bounds; they are not mounted into a tool root or packaged artifact.

## Integration contract

`kuru-core` owns the parsed enabled/filter/header-reference shape and parser-schema parity. `kuru-connectors` owns original-name filtering, environment resolution, MCP request headers, alias state, live routes, and the versioned private cache codec. `apps/kuru-tui` supplies the checked data/project/authority binding and renders `ToolCatalog`; it does not parse connector cache files or create a second MCP client.

The provider-facing name remains the existing UUIDv5 projection of `(alias, original name)`, while permission identity remains the typed alias/original selector. Cache schema v1 stores only validated `ToolSpec` metadata and context fingerprints; unknown schema versions are misses. Local stdio and Streamable HTTP fixture servers exercise unchanged JSON-RPC initialize/list/call shapes, plus header capture for HTTP. P08 summary work consumes provider/actor settlement and does not alter this route or identifier contract.
