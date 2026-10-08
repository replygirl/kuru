# Tasks

## 1. Native absent-stream policy

- [x] 1.1 Extend the existing isolated private-console fixture with actual null Input/Output/Error slots, exact null-child-stdio and NotConnected console outcomes, and false VT admission; restore originals and modes before assertions/output. Verify Windows-target platform lint and independent native boundary review. Observed: formatting and Windows-target package lint pass; independent review clears retained originals/duplicates, normal and fallback restoration, missing-stderr positive capture, VT admission and original mode/focus assertions. Native execution remains task1.2.
- [ ] 1.2 Observe the existing console fixture pass on native Windows x64 and ARM, retain every existing behavior test, and inspect the unchanged95% gate. Until those checks execute, report them as pending.
