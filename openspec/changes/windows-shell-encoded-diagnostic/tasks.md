## 1. Failure-only Windows diagnostic

- [x] 1.1 Add at most three fresh isolated controlled launches after the existing Kuru shell failure and verify they preserve the original success assertions.
- [ ] 1.2 Run the relevant native fixture when scheduled and record only observed encoded/environment/console outcomes. Native execution remains pending; the macOS Windows-target check stopped before this fixture at the known `aws-lc-sys` missing-MSVC-header boundary.
