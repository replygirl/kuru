# Dependencies

## Blocked by

- [x] `owner-open-timeline` — the gated `KURU_OPEN_TIMELINE` owner timeline and the aged-store fixture *(archived 2026-10-01)*
- [x] `usage-scan-index-range` — the linear primary-key range scan the first open after an upgrade depends on *(archived 2026-10-01)*
- [x] `usage-scan-aged-check` — the gating `usage-scan-scaling` job whose provisional bounds this change calibrates *(archived 2026-10-01)*

## Soft-blocked by

None.

## Siblings

- `ci/usage-scan-fixture-cache-budget` (#165, merged 2026-10-01) removed the fixture cache steps from `ci.yml` and rewrote the fixture-cache text of `docs/development.md`. This change was rebased onto it: the job keeps #165's uncached in-job ageing, with this change's calibrated bounds, job and step names, comments and shape-test names.
