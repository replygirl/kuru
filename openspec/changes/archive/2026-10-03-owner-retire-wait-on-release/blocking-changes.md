# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Related

- `owner-lock-release-after-failed-open` (archived 2026-10-03): explicit
  owner-lock release on every ending, which the lock reads here rely on.
- The bound replacement (wait on the owner's lock release) is blocked on a lead
  decision about an outer backstop for `ServiceCleanup` and the mise acceptance
  fixture; see `tmp/roadmap/owner-retire-bound-2026-10-04.md` (untracked notes).
