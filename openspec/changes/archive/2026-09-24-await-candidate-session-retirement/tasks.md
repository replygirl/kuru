## 1. Prove the branch lifecycle gap

- [x] 1.1 Add a focused real-Dolt regression that holds a source-branch session while candidate retirement and a new source-pool acquisition race; pre-fix hosted macOS candidate cases had Dolt's branch-in-use refusal, while the corrected synchronized native fixture passed 1/1 with no rename or new admission until exact server teardown.
- [x] 1.2 Verify pinned Dolt's candidate `DATABASE()` and processlist `DB` identify the same exact branch, including multiple candidate-pool connections; the synchronized held-session fixture and four-session retirement probe each passed 1/1.

## 2. Fence and wait before checked rename

- [x] 2.1 Add a source-branch admission guard shared by candidate pool acquisition and status transition; the synchronized native fixture proved source acquisition blocked while unrelated main acquired successfully.
- [x] 2.2 Close the source pool and await a zero exact source-branch processlist count within the existing deadline before issuing the one checked rename; the held-session native fixture passed 1/1 with active exact DB/ID, bounded pre-Pending timeout, unchanged source/status refs and history, and no force.
- [x] 2.3 Preserve post-dispatch uncertain reconciliation and exact recovery; the held-session fresh checked abandon and the two previously failing memory/runtime candidate cases all passed with real Dolt.

## 3. Review and deliver

- [x] 3.1 Run owning memory checks and independent source review of admission, deadline, and recovery boundaries; memory all-target Clippy, cargo fmt/diff-check and Toolcards bounded review passed, with exact local outcomes recorded in verification.md.
- [x] 3.2 Record the exact prior hosted failure and corrected local real-Dolt regressions, complete strict validation, and prepare the truthful record for archive before the final branch commit. Exact CI `36029830458` was recorded, five corrected local real-Dolt cases passed, and strict validation passed 0/0. Exact final-head macOS/Windows coverage and report remain post-push merge gates, not claimed prearchive evidence.
