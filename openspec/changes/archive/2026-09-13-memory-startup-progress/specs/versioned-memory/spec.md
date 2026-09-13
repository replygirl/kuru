## MODIFIED Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official native archive and runtime licenses in its executable. It MUST provision the matching engine locally without a compiler, separate installation or runtime download, including on a first offline launch. It MUST reject unsafe archives and corrupt executables before execution and SHALL NOT silently fall back to SQLite. Existing verified caches MAY be reused; corrupt existing caches MUST fail explicitly without destructive repair. An explicit observed open MAY report only fixed, bounded stage values for actual startup work; it MUST leave ordinary library opens silent, must not weaken any payload digest or exact-version probe, and MUST report ready only after final store validation and activation succeed.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using that verified cache; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.

#### Scenario: Observed startup
- **WHEN** an application requests an observed memory open
- **THEN** it receives fixed stages only where project ownership, managed cache verification or extraction, version probing, database preparation or opening actually begins, and receives ready only with a usable returned store.

### Requirement: Preserved SQLite migration

Migration MUST validate the legacy store, preserve its original and a consistent snapshot including committed WAL data, and activate only a validated committed Dolt import. It SHALL preserve current-project rows, ordering and JSON exactly; other projects SHALL remain recoverable from the preserved source. Interrupted imports SHALL be recoverable without duplicate activation or source modification. When a legacy Unix data directory is not owner-private, Kuru MUST refuse before opening or importing it with an actionable remedy naming that directory; it MUST not silently change permissions. Windows guidance MUST use the native owner privacy contract rather than a Unix mode command.

#### Scenario: Existing conversations
- **WHEN** an existing project first opens with Dolt
- **THEN** sessions, preferences, topology, private histories and relationship memories remain available in the same order.

#### Scenario: Failed import
- **WHEN** validation or activation is interrupted
- **THEN** the original remains usable and no partial target becomes the active memory store.

#### Scenario: Unsafe legacy directory
- **WHEN** a Unix legacy SQLite layout is in a non-owner-private data directory
- **THEN** Kuru refuses before import and identifies the actual directory and its owner-only repair command.
