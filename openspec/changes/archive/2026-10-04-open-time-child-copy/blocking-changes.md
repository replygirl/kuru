# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Related

- `unix-snapshot-stand-in-spawn` (archived 2026-10-01): established the same writer-in-parent fork race and child-only executable preparation.
- Open PR #207 isolates owned Unix pipes; it does not cover the writable executable descriptor in this test and is not required here.
- Open PR #216 changes the open-time implementation; this change touches only its integration fixture.
- Open PR #217 changes delivery fixture wait derivations; preserve both changes when integrating.
