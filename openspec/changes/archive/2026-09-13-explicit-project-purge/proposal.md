## Why

Deleting one selected note is intentionally a normal Dolt revision, but a person
also needs an explicit way to remove one managed project's current database and
revision history. Removing only the active directory is unsafe: recovery copies
can remain, and the ordinary legacy importer can silently restore the deleted
project on its next open.

## What Changes

- Add a confirmed, provider- and engine-free `kuru memory purge --yes` path for
  one canonical project.
- Record that project's legacy-import suppression and any incomplete removal
  inventory in one checked durable control record, so interrupted cleanup is
  retried against the originally verified identities rather than replacement
  paths.
- Add a narrow platform checked-tree removal primitive and use retained
  lifecycle leases to quarantine then remove only verified managed Dolt trees.
- Remove the matching bounded TUI diagnostics ring under the already held CLI
  writer lease, reporting incomplete application cleanup truthfully.
- Document that purge removes managed current memory and Dolt history but
  retains shared/original SQLite, user exports and backups, engine cache, and
  stable lock objects; it is not secure physical erasure.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `versioned-memory`: explicit project-scoped managed-store purge and recovery.
- `native-platform`: checked, identity-retaining directory-tree removal.
- `public-documentation`: project-purge command and history boundary.

## Impact

`kuru-memory` gains an associated purge entrypoint and private control receipt;
`kuru-platform` gains one checked directory removal primitive; `kuru` gains a
confirmed memory subcommand and app-owned diagnostics cleanup. No schema
migration, provider call, engine provisioning, history rewrite, automatic expiry,
credential access, or new dependency is introduced.

## Surfaces

- [x] interactive — confirmed CLI command and help/documentation.
- [ ] deploy — no deployment topology changes.
- [ ] integration — no third-party contract changes.
- [ ] agent-behavior — no prompt, tool, or model behavior changes.
