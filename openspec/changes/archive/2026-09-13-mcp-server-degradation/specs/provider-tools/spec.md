## ADDED Requirements

### Requirement: Alias-local MCP availability

MCP discovery SHALL validate and publish each configured server alias as one complete unit. A start, initialization, catalog transport, pagination, size, or schema failure MUST mark only that alias unavailable, omit all of its tools from the returned catalog, and report a fixed alias-specific status while built-in tools and completely validated healthy aliases remain usable. Previously known routes for an unavailable alias MUST reject without dispatch until a later explicit catalog operation completely validates and republishes that alias.

Initialization, every catalog page, tool calls, close, and alias-state publication MUST be serialized as logical operations per alias without serializing independent aliases. A call transport or protocol failure after possible dispatch, including caller cancellation, MUST make that alias unavailable and MUST NOT automatically resend the call. An MCP `isError=true` response MUST remain a projected application result and MUST NOT degrade a healthy session.

#### Scenario: One failed alias does not abort the turn

- **WHEN** catalog discovery visits one unavailable server and one healthy server in either configured order
- **THEN** the provider receives built-in and healthy-server tools, receives none from the failed alias, and the runtime records fixed unavailable metadata for that alias outside provider prompts, conversation messages, and authored notes

#### Scenario: Invalid partial catalog is not published

- **WHEN** one alias returns valid pages followed by a malformed, repeated, oversized, or otherwise invalid page
- **THEN** none of that alias's candidate tools becomes advertised or callable and another alias's published routes remain intact

#### Scenario: Ambiguous call is not replayed

- **WHEN** a server receives a tool call and the response fails or the caller disappears before completion
- **THEN** Kuru sends that call at most once, marks the alias unavailable, and rejects another call on its prior route without network or stdin dispatch until explicit successful rediscovery

#### Scenario: Application error keeps the alias healthy

- **WHEN** a server completes a tool call with `isError=true`
- **THEN** Kuru returns its recognizable-secret-projected useful content and later calls may use the same healthy session

### Requirement: Owned MCP stderr diagnostics

A configured stdio MCP session MUST keep one independent connector owner for the native child tree, stdin, stdout, stderr, retained workspace capability, request framing, and bounded cleanup. The owner MUST revalidate the original retained workspace immediately before process creation, continuously drain stderr until EOF or cleanup, and survive loss of a request future or its parent asynchronous runtime. Unix cleanup MUST consume group and root signal authority before exact root reap and perform no destructive numeric signal afterward; Windows cleanup MUST retain Job ownership through tree quiescence. Shutdown MUST reject new sessions before attempting every existing client within one aggregate bound, and any unconfirmed result MUST leave the independent owner retaining authority until later confirmation.

Recognizable-secret scanning MUST occur on stderr byte chunks before bounded retention. Only a bounded, terminal-escaped human diagnostic tail with saturating byte-count and truncation metadata MAY reach direct CLI stderr. Runtime events, tool errors, provider prompts, memory, serialization, and ordinary error chains MUST contain fixed Kuru-owned status metadata and MUST NOT contain captured stderr, command, arguments, environment, endpoint, response, or raw process error text.

#### Scenario: Stderr is drained and projected before retention

- **WHEN** a stdio server emits more than the diagnostic limit including a recognizable fake credential split across read boundaries, invalid UTF-8, and terminal controls
- **THEN** the server does not block, the human diagnostic is bounded and escaped with the credential replaced, and no raw captured byte reaches runtime metadata, prompts, memory, errors, or debug formatting

#### Scenario: Owner survives caller and runtime loss

- **WHEN** a request caller or its parent runtime disappears after possible stdin dispatch while a same-group or Job descendant retains stderr
- **THEN** the independent owner disables the session, drains or closes its pipes, performs bounded native cleanup without stale PID authority, and either confirms quiescence or honestly retains ownership

#### Scenario: Shutdown closes admission once

- **WHEN** shutdown races starting, active, and idle aliases
- **THEN** no new alias is admitted after closure, every already-admitted client receives a cleanup attempt, successful shutdown confirms their cleanup, and the aggregate wait is bounded once rather than once per configured server
