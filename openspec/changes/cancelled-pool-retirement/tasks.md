# Tasks

## 1. Causal test before production mutation

- [x] 1.1 Implement the private real-Dolt gated-response relay and current-thread started-return/pre-close controls; keep production Drop unchanged and clean up both assertion outcomes.
- [x] 1.2 Run the actual unchanged-Drop negative regression and record its causal event, pool state and observed failure; send the tiny proposed production diff for root review. Actual exit101: started_return left close Pending, size1/idle0/checked_out0; pre-close passed. Both controls cleaned up before returning. Production was unchanged at that checkpoint; root accepted the actual negative and exact proposed tiny diff before later production mutation.

## 2. Focused correction and acceptance

- [x] 2.1 After root review, flag abnormal retained connections close_on_drop without changing explicit release or deadlines; run the same regression and pre-close control to demonstrate before/after behavior. Reviewed tiny patch applied; corrected controls exit0,4passed/0failed. Relay timeout now aborts and awaits its task; an already-returned JoinError is already joined and is not awaited twice.
- [x] 2.2 Verify explicit success reuse, canceled explicit release, client capacity and exact server-session absence with real fixture controls. All four real-Dolt controls passed; normal successful work reused one idle session, canceled inline release retained then freed its sole client permit, and exact server IDs were observed absent.
- [x] 2.3 Run the existing candidate retirement and uncertain-write receipt controls, then one focused package-owned run of the original failing runtime test; record actual outcomes separately from hosted CI. All four focused package-owned runs exit0, each1passed/0failed; candidate retired exact sessions before rename, receipt fences remained intact, and original runtime test passed locally. Historical CI cause remains unproven.

## 3. Completion

- [x] 3.1 Run relevant package-owned static, format, Cospec and documentation checks; freeze source for independent root review and record limits honestly. All final local checks passed; root independently accepted frozen source and the complete local ledger for the normal-hook intermediate checkpoint. Hosted acceptance and actual archive remain pending.
- [ ] 3.2 Obtain fresh exact-head supported-platform hosted acceptance before completing its verification row.
- [ ] 3.3 Complete the ledger, validate, perform actual Cospec archive and normal-hook final commit after required review and checks; root retains remote authorization.
