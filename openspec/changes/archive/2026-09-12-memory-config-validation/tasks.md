## 1. Core validation policy

- [x] 1.1 Reject relative optional memory cache and Dolt-binary paths in `MemoryConfig::validate`, while retaining the existing timeout and malformed-path checks.
- [x] 1.2 Add a focused core regression that fails before the policy correction and proves native absolute nonexistent overrides remain valid.

## 2. Public memory boundaries

- [x] 2.1 Invoke `MemoryConfig::validate` before `MemoryStore::open` can create store directories or acquire a lifecycle lock.
- [x] 2.2 Invoke `MemoryConfig::validate` before public provisioning can inspect or create cache state, with isolated direct-entry regression coverage.

## 3. Documentation and verification

- [x] 3.1 Document the absolute-path requirement for the two memory overrides without changing relative CLI data-directory or tool-path behavior.
- [x] 3.2 Run the relevant package-scoped mise checks and record the observed results; leave combined coverage to the coordinated integration check.
