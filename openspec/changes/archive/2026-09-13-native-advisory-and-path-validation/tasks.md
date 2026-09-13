## 1. Advisory clone compatibility

- [x] 1.1 Extend the private advisory configuration allowlist with only the exact stock Git-for-Windows `core.symlinks=false` value, retaining rejection of true and unknown values.
- [x] 1.2 Add and run a focused regression that accepts the false value and rejects the true value through the real validator.

## 2. Portable fixture assertions

- [x] 2.1 Compare advisory CLI fixture directories by canonical path identity rather than a verbatim display string.
- [x] 2.2 Derive the core provenance expectation through the existing escaped native display contract.

## 3. Scoped evidence

- [x] 3.1 Run relevant delivery and core granular checks; record local evidence and defer native Windows execution to CI without claiming a local pass.
