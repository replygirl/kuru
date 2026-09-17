## 1. Checked Windows cleanup

- [x] 1.1 Implement identity-checked, OS32-only bounded cleanup for the exact private child and empty outer stage while retaining the cache lease.
- [x] 1.2 Make fixture-only cache-binary invalidation prove the recorded identity and recover only bounded OS32 sharing violations.
- [x] 1.3 Add controlled held-handle release, persistent-handle, and replacement regressions without changing production retry or timeout policy.

## 2. Verification

- [x] 2.1 Run the focused host activation fixture plus memory typecheck, lint, formatting, and diff checks; record Windows-native cases as pending.
- [x] 2.2 Obtain independent review and validate strictly before archive.
