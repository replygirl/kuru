# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

- `fix/memory-lifecycle-ordering` (not yet a cospec change on main): kuru-memory lifecycle fixture work in the same flake slice. It touches no provisioning file (`provision.rs`, `provision/*`, `files.rs`) or `docs/memory.md`, so neither change orders the other.
- `2026-09-28-delivery-bundle-lock-release` (archived): lists `CacheLock` release-by-close as a follow-on. This change keeps that release mechanism and only orders it after the stage teardown.
