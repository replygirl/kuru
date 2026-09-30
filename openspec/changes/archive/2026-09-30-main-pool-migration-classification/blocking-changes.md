# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Notes

- PR #136 (branch `test/memory-engine-contract`, head `01509235`) provides the
  S8 engine-contract measurement and reusable `engine_contract_tests.rs`
  fixture helpers (`use_database`, `sole_parent_on`, `instance_as_of`) this
  change's parity task can reuse. It is not an openspec change slug in this
  repository (no `openspec/changes/archive/*engine-contract*` on
  `origin/main`), so it cannot be listed above, and it is not a dependency:
  this change does not depend on its code or its merge (see design.md
  Context). If unavailable, the implementer reproduces the equivalent
  fixtures and the parent-schema form comparison independently.
- Template-born stores (the machine-cache template work,
  `tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md` §3) are a
  separate, not-yet-started change with no openspec change slug yet. This
  change is a prerequisite for it (SD §7.2-7.4: order 1 needs main-pool
  classification before the template lands), not the reverse — listed here
  for context only, not as a blocker.
