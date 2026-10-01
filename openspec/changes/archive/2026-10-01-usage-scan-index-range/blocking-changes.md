# Dependencies

## Blocked by

- [x] `usage-ledger-guarantee-tests` — fault-injection reopen tests that pin the validate-before-activation guarantee this change must preserve *(archived 2026-10-01)*

## Soft-blocked by

None.

## Siblings

- PR #154 (`perf/owner-start-split`, not a cospec change on this branch) adds
  the `KURU_OPEN_TIMELINE` instrument. It is not on main, so this change
  measures the scan directly instead of through the open timeline.
