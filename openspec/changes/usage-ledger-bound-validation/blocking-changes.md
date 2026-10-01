# Dependencies

## Blocked by

- [x] `owner-open-timeline` — the gated `KURU_OPEN_TIMELINE` owner timeline and the aged-store fixture *(archived 2026-10-01)*
- [x] `usage-scan-index-range` — the linear primary-key range scan the first open after an upgrade depends on *(archived 2026-10-01)*
- [x] `usage-scan-aged-check` — the gating `usage-scan-scaling` job whose provisional bounds this change calibrates *(archived 2026-10-01)*

## Soft-blocked by

None.

## Siblings

- `ci/usage-scan-fixture-cache-budget` (separate small PR, in parallel) edits the fixture cache steps in `ci.yml` and the "fixture cache" bullets of `docs/development.md`. This change edits the same job's bounds text and comments. Expect textual overlap in `ci.yml` and `docs/development.md`; whichever lands second rebases.
