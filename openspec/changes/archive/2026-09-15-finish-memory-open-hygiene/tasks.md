## 1. Warm runtime verification

- [x] 1.1 Split managed provisioning into lock-free existing-cache verification and under-lock missing-cache installation with a destination recheck.
- [x] 1.2 Verify full payload digests through retained checked handles, revalidate names and identities immediately before the exact-version probe, and retain them until probe reap.
- [x] 1.3 Add regression coverage for an actual warm open while the installation lock is held, concurrent warm opens, corruption before execution, and existing cold race/cancellation/publication guarantees.

## 2. Data-directory remedy

- [x] 2.1 Generalize the memory-owned Unix privacy-error helper to recognize only real current-user-owned directories with group/other access, independent of legacy SQLite presence.
- [x] 2.2 Add regression and negative-control coverage proving exact-path mode-0700 guidance without mutation and no guidance for links or foreign-owned paths.
- [x] 2.3 Coordinate the minimal CLI call-site wiring with its current owner and verify ordinary and legacy command paths use the shared policy.

## 3. Documentation and verification

- [x] 3.1 Document lock-free warm verification, retained full digests/version checks, serialized cold publication and narrow Unix remediation.
- [x] 3.2 Run focused memory/native checks, measure the actual warm path, and record only observed evidence in the verification ledger.
