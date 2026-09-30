# Spec Delta

## MODIFIED Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official native archive and runtime licenses in its executable. It MUST provision the matching engine locally without a compiler, separate installation or runtime download, including on a first offline launch. It MUST reject unsafe archives and corrupt executables before execution and SHALL NOT silently fall back to SQLite. Existing verified caches MAY be reused; warm opens MUST verify their complete pinned payloads concurrently without acquiring the exclusive installation lock, while corrupt existing caches MUST fail explicitly without destructive repair. Exact-version verification of the cached engine happens once, at install time, before a freshly extracted cache is activated; a warm open does not re-probe it. Missing-cache installation and publication MUST remain serialized, including an under-lock destination recheck. An explicit observed open MAY report only fixed, bounded stage values for actual startup work; it MUST leave ordinary library opens silent, must not weaken any payload digest or the install-time exact-version probe, and MUST report ready only after final store validation and activation succeed.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine under the installation lock and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using the fully digested cache, whose exact version was proven when it was installed, without waiting for an unrelated installer lock; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.

#### Scenario: Observed startup
- **WHEN** an application requests an observed memory open
- **THEN** it receives fixed stages only where project ownership, managed cache verification or extraction, install-time version probing, database preparation or opening actually begins, and receives ready only with a usable returned store.
