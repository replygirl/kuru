# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Dependents

The Windows on Arm target change (PR6b: `aarch64-pc-windows-msvc` target, native
`windows-11-arm` jobs, installer, updater, release and docs) depends on this change
being archived and merged first. It consumes the schema v2 manifest, the pinned
built arm64 entry, `bundle build` and the Linux build job.
