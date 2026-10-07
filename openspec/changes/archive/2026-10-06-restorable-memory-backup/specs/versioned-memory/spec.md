# Spec Delta

## ADDED Requirements

### Requirement: Restored storage authority and historical scope

A verified restored activation SHALL distinguish target canonical storage identity from retained historical namespace and SQL-origin provenance. Its backward-compatible activation/identity records MUST bind the checked backup/root, source instance/scope and authorized target remap. Fresh external credentials/store instance/generation SHALL own the target; copied SQL-origin rows and durable receipts remain historical data, not source attachment authority. Runtime/CLI namespace consumers and owner session-mutation checks SHALL use the checked retained history scope while attachment, native driver and maintenance checks retain target-canonical identity. Ordinary unrestored format-1 stores MUST remain compatible. Stale source/target handles MUST NOT open, recreate or mutate the other store through matching namespace text.

#### Scenario: Resume a remapped restored session

- **WHEN** a runtime admits an old session from a verified remapped target
- **THEN** it reads the retained session/private history keys under a fresh target claim and cannot reuse a source connection or native lease as target authority

#### Scenario: Backup of an already restored target

- **WHEN** the target later creates another backup
- **THEN** the manifest distinguishes its immediate source storage authority from the original SQL/history provenance, and another restore creates fresh authority again
