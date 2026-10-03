# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Related

- `owner-lock-release-after-failed-open` (archived 2026-10-03): explicit
  owner-lock release on every ending, which the lock reads here rely on.
- The bound replacement (wait on the owner's lock release) was blocked on a lead
  decision about an outer backstop for `ServiceCleanup` and the mise acceptance
  fixture; the lead chose option (b) and this change lands it (see the
  proposal's Decision). Option (c), deriving the product maintenance permit's
  election deadline from `close_budget()`, is a separate follow-on PR.
