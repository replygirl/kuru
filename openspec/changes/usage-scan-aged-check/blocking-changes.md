# Dependencies

## Blocked by

- [x] `owner-open-timeline` — the gated `KURU_OPEN_TIMELINE` owner timeline with its `usage-pool`/`usage-scan-1` stamps and usage row count, and the `measure:age-store` task that ages the fixture *(archived 2026-10-01)*
- [x] `usage-scan-index-range` — the linear primary-key range scan whose growth this job guards *(archived 2026-10-01)*

## Soft-blocked by

None.

## Siblings

- The validation-record change (Unit 7 B, not started) replaces the
  provisional bounds with calibrated ones and their derivation, and extends
  this job to assert that a recorded reopen decodes zero rows.
