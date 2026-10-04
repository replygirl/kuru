# Tasks

## 1. Causal test before production mutation

- [x] 1.1 Implement the private real-Dolt gated-response relay and current-thread started-return/pre-close controls; keep production Drop unchanged and clean up both assertion outcomes.
- [x] 1.2 Run the actual unchanged-Drop negative regression and record its causal event, pool state and observed failure; send the tiny proposed production diff for root review. Actual exit101: started_return left close Pending, size1/idle0/checked_out0; pre-close passed. Both controls cleaned up before returning. Production was unchanged at that checkpoint; root accepted the actual negative and exact proposed tiny diff before later production mutation.

## 2. Focused correction and acceptance

- [x] 2.1 After root review, flag abnormal retained connections close_on_drop without changing explicit release or deadlines; run the same regression and pre-close control to demonstrate before/after behavior. Reviewed tiny patch applied; corrected controls exit0,4passed/0failed. Relay timeout now aborts and awaits its task; an already-returned JoinError is already joined and is not awaited twice.
- [x] 2.2 Verify explicit success reuse, canceled explicit release, client capacity and exact server-session absence with real fixture controls. All four real-Dolt controls passed; normal successful work reused one idle session, canceled inline release retained then freed its sole client permit, and exact server IDs were observed absent.
- [x] 2.3 Run the existing candidate retirement and uncertain-write receipt controls, then one focused package-owned run of the original failing runtime test; record actual outcomes separately from hosted CI. All four focused package-owned runs exit0, each1passed/0failed; candidate retired exact sessions before rename, receipt fences remained intact, and original runtime test passed locally. Historical CI cause remains unproven.

## 3. Completion

- [x] 3.1 Run relevant package-owned static, format, Cospec and documentation checks; freeze source for independent root review and record limits honestly. All final local checks passed; root independently accepted frozen source and the complete local ledger for the normal-hook intermediate checkpoint. At that checkpoint, hosted acceptance and actual archive remained pending; subsequent hosted acceptance is recorded below.
- [x] 3.2 Obtain fresh exact-head supported-platform hosted acceptance before completing its verification row. PR224 checkpoint CI37239400639 completed success at exact6ddd8dc1fb7a6e87cf527decbacb505318191ecc: all66jobs terminal,62success and4conditional engine jobs skipped; required native macOS/Linux/Windows behavior, coverage, installation/update and gates passed.
- [x] 3.3 Complete the acceptance ledger and final strict validation after required source/check review and hosted acceptance; root retains remote authorization. Local source/ledger review and fresh exact-head hosted acceptance passed; actual final strict validation exited0,0errors/0warnings.

Mandatory closeout: run the actual clear apply gate and Cospec archive, confirm the archive exists with its active directory absent, and make the normal-hook final commit before root publication. This prose preserves their required scope without a self-referential archive-task checkbox; neither action is abandoned or reported executed by the completion checklist.

Actual archive closeout: strict validation exited0 with0errors/0warnings, then the actual apply gate exited0 clear with8/8tasks complete; all returned context files were read. The actual archive command exited0 and printed:

```text
Archived: cancelled-pool-retirement (fix) → openspec/changes/archive/2026-10-04-cancelled-pool-retirement/
Specs:    +1 ~0 -0 →0 applied and verified
```

The archive directory was independently confirmed present and the active directory absent. No archive warning, sibling unblock or skipped spec was reported. Product source remains byte-identical to checkpoint6ddd8dc1. Normal-hook final commit is the next mandatory action; its actual outcome and exact hash are retained in the ignored finalization receipt after execution. Root owns publication, fresh final-head CI and merge.
