# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

Not gating; recorded so later rebases are deliberate.

- `memory-open-activity` (archived 2026-09-30): defines the open markers and the tagged activity record beside the endpoint. This change adds a separate, gated file in the same owner-private directory and touches neither; its `project-memory-owner` requirement text is unchanged.
- The open-time harness (assistant3, branch `ci/open-time-report`): Ubuntu numbers come from it later. This change adds nothing to that branch and does not wait for it.
- The pin bump (unit 5): if it has not merged, local mise runs use `MISE_LOCKED=1`; `mise.lock` is never committed.
