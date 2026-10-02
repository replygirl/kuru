# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

- The local branch `feat/readiness-owner-timeline` (not on origin, no active change on main) edits the same two readiness `bail!` sites in `packages/kuru-memory/src/service.rs`. By lead decision D3 neither depends on the other: whichever lands second rebases those sites and that branch's T4 test. Its extra stamp sites become progress points here once both have landed.
- The active `openspec/changes/` held no other change when this one was created (2026-10-02, base `b9453c5b`); the shipped providers this change builds on are `2026-09-30-memory-open-activity` (the tagged record), `2026-09-29-readiness-failure-diagnostics` (the phase split) and `2026-10-01-client-readiness-poll-cadence` (the 10 ms poll), all archived.
