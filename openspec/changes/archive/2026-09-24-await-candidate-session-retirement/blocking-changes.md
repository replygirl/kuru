# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

`locate-windows-candidate-transition-failure` is an archived diagnostic-only test change on the same #71 branch. Its fixed owner-stage record helped identify the macOS failure but is not a runtime prerequisite for safe session retirement. The required managed owner, candidate transition, and exact-session teardown contracts are already present in archived project-memory-service and candidate-lifecycle changes.
