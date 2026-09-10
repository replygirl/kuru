## MODIFIED Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official
native archive and runtime licenses in its executable. It MUST provision the
matching engine locally without a compiler, separate installation or runtime
download, including on a first offline launch. It MUST reject unsafe archives
and corrupt executables before execution and SHALL NOT silently fall back to
SQLite. Existing verified caches MAY be reused; corrupt existing caches MUST fail
explicitly without destructive repair.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using that verified cache; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.
