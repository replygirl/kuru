# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Notes

- `perf/memory-main-pool-classification` (PR #141, not tracked as a cospec
  change) touches `validate_active` and related signatures in
  `packages/kuru-memory/src/store/migrations.rs`, which the `validate-and-mark`
  job calls. It is developed on a separate branch outside this repo's cospec
  history. This change is not stacked on it: it rebases onto `origin/main` at
  apply/push time and, if #141 has not merged yet, the extraction is written
  against `validate_active`'s current signature and the expected conflict
  area (the `validate-and-mark` job's call into `store/migrations.rs`) is
  called out in the PR body rather than resolved here.
- The per-machine template design
  (`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`, P4b) is
  the consumer this extraction prepares for. It is not yet an active cospec
  change, so it is not listed as a blocking or soft-blocking dependency; this
  change does not depend on it and changes no behaviour it would need.
