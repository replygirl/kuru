## 1. Windows owner handoff

- [x] 1.1 Reapply the checked protected private stage DACL before exact-handle owner assignment and preserve post-assignment privacy and owner equality checks
- [x] 1.2 Give the retained-object ACL fixture exact `READ_CONTROL | WRITE_DAC` authority without changing production source opens

## 2. Verification and handoff

- [x] 2.1 Run the Windows all-target check plus formatting and diff validation
- [x] 2.2 Record exact pre-fix hosted Windows failures and hand the corrected native regressions to Delivery's required hosted run
