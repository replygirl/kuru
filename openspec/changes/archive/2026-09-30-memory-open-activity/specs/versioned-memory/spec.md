# Spec Delta

## MODIFIED Requirements

### Requirement: Managed full Dolt storage

Kuru SHALL use pinned full Dolt for live memory and include the verified official native archive and runtime licenses in its executable. It MUST provision the matching engine locally without a compiler, separate installation or runtime download, including on a first offline launch. It MUST reject unsafe archives and corrupt executables before execution and SHALL NOT silently fall back to SQLite. Existing verified caches MAY be reused; warm opens MUST verify their complete pinned payloads concurrently without acquiring the exclusive installation lock, while corrupt existing caches MUST fail explicitly without destructive repair. Exact-version verification of a managed cached engine happens once, at install time, before a freshly extracted cache is activated; a warm open does not re-probe it. An explicitly configured `dolt_binary` is not a managed cache and remains version-probed on every open. This is a deliberate trade: a cached executable whose bytes match the pinned digest yet cannot run, or reports another version, is no longer detected by a warm open itself and surfaces instead when the database server fails to start; the install-time probe and the Dolt-version-keyed cache directory are the mitigations. Missing-cache installation and publication MUST remain serialized, including an under-lock destination recheck. An explicit observed open MAY report only fixed, bounded stage values for actual startup work, including a project-ownership wait only while another process holds that ownership, creation of a project store that has no active memory, a schema upgrade of an existing active store, and a client's start of its own memory service; it MUST leave ordinary library opens silent, and an open run by the memory service itself MUST stay silent to its caller and MAY publish its stages only as the owner's activity record, must not weaken any payload digest or the install-time exact-version probe, and MUST report ready only after final store validation and activation succeed.

#### Scenario: First memory use
- **WHEN** memory is opened without an extracted runtime
- **THEN** Kuru extracts its bundled matching verified pinned engine under the installation lock and opens a private project database without network access.

#### Scenario: Offline cache
- **WHEN** offline memory access uses a valid cached runtime
- **THEN** memory opens using the fully digested cache, whose exact version was proven when it was installed, without waiting for an unrelated installer lock; an absent cache is initialized from bundled bytes, while an invalid existing cache gives a clear error.

#### Scenario: Observed startup
- **WHEN** an application requests an observed memory open
- **THEN** it receives fixed stages only where a contended project-ownership wait, managed cache verification or extraction, version probing of a freshly extracted engine or an explicit `dolt_binary`, database preparation, creation of a project with no active store, schema upgrade of an existing store, the client's start of its own memory service, or opening actually begins, and receives ready only with a usable returned store.

#### Scenario: Uncontended ownership
- **WHEN** an observed open acquires project ownership on its first attempt
- **THEN** it reports no project-ownership wait stage.

#### Scenario: Read-only open of a missing project
- **WHEN** a read-only open finds a project with no active store
- **THEN** it fails without reporting creation, and a read-only open of an older store fails without reporting an upgrade.
