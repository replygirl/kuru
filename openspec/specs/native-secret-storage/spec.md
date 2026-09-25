# native-secret-storage Specification

## Purpose
Define Kuru's exact, generation-checked MCP credential records in each supported operating system's native secret facility. Keep tokens and registered client secrets out of configuration, Dolt, catalog caches, output, and diagnostics while making publication, replacement, deletion, and recovery safe across process restarts and cancellation.

## Requirements

### Requirement: MCP credentials use the native operating-system secret store

Kuru SHALL store MCP access tokens, refresh tokens, dynamically registered client secrets and their nonsecret binding metadata behind one `kuru-platform` native secret-store interface backed by the canonical user credential facility on each supported operating system. Records MUST be service-namespaced to Kuru, keyed by a domain-separated opaque alias/binding identity, owner accessible, bounded and redacted from Debug, errors and projections. Kuru MUST NOT write this material to configuration, the workspace, Dolt, catalog caches, logs, command output or a plaintext fallback.

#### Scenario: Credential is published on a supported host
- **WHEN** a valid login completes and the supported operating-system secret store is available
- **THEN** a later Kuru process can retrieve the exact bound credential through the native interface while ordinary files, configuration projection and diagnostics contain no token or client secret

#### Scenario: Native secret store is unavailable
- **WHEN** the platform credential facility is missing, locked, denied or unsupported
- **THEN** OAuth login and refresh are unavailable with actionable platform guidance, no plaintext or session-only token is retained, and configured static-header MCP aliases remain independently usable

### Requirement: Secret replacement is generation checked and cancellation safe

The native secret-store interface SHALL support exact existing-only read, expected-absence create, generation-checked replace and generation-checked delete operations with a caller-supplied binding and generation. Create MUST fail stale if the exact record already exists; replace and delete MUST fail stale unless the existing record has the expected generation. Connector publication MUST prevent an older refresh or login from overwriting a newer credential or resurrecting a logged-out alias, and once a rotating token response is accepted it MUST settle the bounded store replacement before reporting cancellation.

#### Scenario: Concurrent first login wins creation
- **WHEN** two first-login operations both observe absence and one creates the exact alias record first
- **THEN** the second expected-absence create fails stale and cannot overwrite the winner

#### Scenario: Refresh races logout
- **WHEN** a refresh prepared from generation A overlaps a completed logout or a newer login generation
- **THEN** the stale refresh cannot republish generation A or its replacement and the final status reflects the later authorized operation

#### Scenario: Cancellation follows accepted rotation
- **WHEN** cancellation arrives after the authorization server accepted a refresh and returned rotated material
- **THEN** Kuru completes or reconciles the exact native-store replacement before returning cancellation and never retries the refresh request blindly

### Requirement: Native implementations preserve platform identity and isolation

Each supported native implementation SHALL validate that it is addressing Kuru's exact service/account record, map not-found separately from denied/unavailable/corrupt outcomes, and avoid broad enumeration or import of credentials belonging to other applications. Test implementations MAY use isolated fake stores with synthetic secrets but MUST exercise the same record, generation, redaction and failure contract.

Logical MCP credentials larger than one native item SHALL use generation-named native chunks behind one versioned manifest. The logical binary envelope MUST derive its maximum from every accepted client-ID, scope and secret field without escaping growth; every native item MUST remain within the supported cross-platform item bound. The manifest MUST bind the complete byte length, digest, generation and chunk count, MUST be the sole publication point, and MUST retain the prior committed descriptor alongside enough intent to complete or roll back an interrupted replace or delete under the connector's nonsecret cross-process lock. Readers MUST reject partial, mixed-generation, oversized or digest-mismatched credentials and MUST NOT fall back to plaintext files or process memory.

#### Scenario: Publication is interrupted at a native item boundary
- **WHEN** a process stops after publishing an intent, after any credential chunk, after the stable manifest, or while deleting retired chunks
- **THEN** the next locked operation returns the complete prior generation, completes the complete digest-verified new generation, or completes deletion as directed by the manifest, and never returns mixed or partial secret bytes

#### Scenario: Another application's credential exists
- **WHEN** a user's credential store contains records for another harness or service with similar account text
- **THEN** Kuru neither reads, migrates, deletes nor reports those records while operating on its own exact MCP service namespace
