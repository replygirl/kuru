# Tasks

## 1. Preserve observed startup evidence

- [x] 1.1 Add a deterministic real-Dolt regression stalling the directory probe after a confirmed foreign mismatch; observe failure before the fix and await fixture teardown.
- [x] 1.2 Preserve the retained mismatch when the directory probe times out, and clear it after a correct directory is verified; keep all original bounds and fail-closed checks.
- [x] 1.3 Prove a later own-server bootstrap timeout does not retain the earlier foreign mismatch, with a real Dolt connection and a deterministic bootstrap stall.

## 2. Verify and archive

- [x] 2.1 Run focused foreign-listener regressions and the full memory package suite, package lint and typecheck, formatting, managed cospec and documentation checks; record observed passes and any unrun checks honestly.
- [x] 2.2 Complete the evidence ledger and validate strictly; archive through cospec before the final branch commit.
