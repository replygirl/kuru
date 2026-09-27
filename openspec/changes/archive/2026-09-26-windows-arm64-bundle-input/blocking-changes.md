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

Status (2026-09-27): the Windows arm64 engine is no longer awaiting an input.
This change's from-source pipeline landed in #113, and #119 rebuilt the
`aarch64-pc-windows-msvc` archive at Dolt v2.3.4 with committed pins. The
PR6b target wiring (#115) is in progress.
