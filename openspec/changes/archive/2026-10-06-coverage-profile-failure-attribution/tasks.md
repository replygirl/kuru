# Tasks

## 1. Repair failed-export evidence

- [x] 1.1 Record the actual Windows selection PID and retain bounded profile
  metadata/header bytes with existing attribution on export failure; verify
  successful exports and strict failure outcomes remain unchanged.
- [x] 1.2 Verify a failed export reports the mapped module, exact run PID and
  malformed profile evidence through the existing artifact, with explicit
  missing/reused identities and bounded partial evidence; run focused delivery
  regressions and affected host/Windows lints, typecheck, docs and format checks.
- [x] 1.3 Complete independent source review, record actual evidence and
  limitations, validate strictly and read the complete apply context. Archive
  and the clean hooked commit follow local completion.
