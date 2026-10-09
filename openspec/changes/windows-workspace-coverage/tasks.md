# Tasks

## 1. Existing native contracts

- [x] 1.1 Extend memory lifecycle/inspection/summary and portable hook-settlement fixtures; prove checked cleanup, one owner, durable isolated reads/writes, exact retry and no peer/public leakage.
- [x] 1.2 Extend existing delivery fixtures; retain exact profile destination and prove native failed-publication/cleanup refusal preserves owned evidence until successful retry, with actual recovery after stdin EOF.
- [x] 1.3 Port existing ToolHost MCP and hook contracts and extend terminal public-state/rendering tests; retain exact authorization, protocol, ownership, draft and privacy assertions.
- [ ] 1.4 Verify rejected managed switch recovery retains the exact old claim; remote exports retain captured revision and reject foreign cursors without exposing later or candidate rows. Use existing managed owner/factory fixtures and await every claim/client/owner cleanup.
- [ ] 1.5 Exercise existing public command/session contracts through ConPTY with completed frames and durable exact session state; verify hook cancellation while its native peer cannot consume the bounded request, output/privacy failures, and the original denied call/receipt reaching the next provider request without a file effect. Reuse existing helpers, bounds and cleanup scopes; add no production seams.

## 2. Verification

- [x] 2.1 Run relevant focused host tests, Windows-target static checks and independent reviews; state native-only checks that cannot run locally.
- [ ] 2.2 Observe all new native contracts and final canonical95% gates on supported CI platforms before archive and merge.

Before implementation: the eight saved Windows line exports reconstruct exactly the CI canonical162174/172114; full inventory is retained. Doctor's existing portable command tests all pass but their profile collection discrepancy needs separate investigation, not duplicate tests. Existing Windows lifecycle creators deliberately terminated by the OS cannot flush normal LLVM exit profiles. The published-command fixture demonstrably clears and omits LLVM_PROFILE_FILE. Missing production routes include actual speaker decisions, post-hook annotation persistence, cold independent starters and attached inspection. Acceptance must remain observable and use existing native authorities; no synthetic permission failures, timer inflation or covered-line prediction is a pass.

Observed locally: summary selection1/1, attached inspection1/1, lost hook reply1/1; connector hooks17/17 and MCP4/4; public UI2/2 and card rendering1/1. Application and all affected package Windows-target lint and independent review pass. Native Windows creator EOF, independent starters, sharing refusal/bootstrap recovery and all promoted portable tests remain pending x64/ARM CI; no local execution is claimed for cfg(windows) tests.

CI37870584478 exposed two new test mistakes: the hook test imported before the mandatory closing scope, and bootstrap recovery compared equivalent Windows receipt path spellings. Move the import inside the existing scope and compare concurrently retained native file identities before releasing both receipt handles for recovery. The unchanged cleanup-layout guard passes locally1/1; Windows-target lint and independent receipt-identity review pass. Native rerun and canonical95% results remain pending.

Subsequent861811ef CI37872687339 passes the unchanged cleanup guard on Ubuntu, macOS and Windows x64/ARM; Ubuntu canonical167299/175911 passes95.10%. Bootstrap's native identity/restoration assertions now pass, but its last assertion incorrectly expects -Recover to retire the receipt. Stock bootstrap intentionally retains the verified rolled_back receipt until a fresh install. Correct the test to require the same operation/original evidence in that retained receipt and the absence of displaced/candidate/backup images. Windows-target lint and independent source-policy review pass; the corrected native recovery rerun and full Windows95% gate remain pending.

Subsequent6ae691c2 CI37875015919 passes all eight Windows behavior/coverage partitions and corrected bootstrap recovery on x64/ARM. The actual canonical merge is164668/173739 (94.778950%), below95%; all eight original exports agree. Doctor381/461 includes the now-proven14/14 invocation-error group. Current full exports and source review identify the additional1.4/1.5 contracts before implementation; new gains require a fresh complete canonical merge, not prediction or changed inventory/gates.

New scoped host acceptance: captured managed export1/1, exact rejected-switch recovery1/1, denied original tool call/error receipt1/1, and connector hooks17/17 pass. Application, memory, connector and runtime Windows-target lint pass. Independent source review verifies revision/cursor isolation, exact recovered authority, denied-effect/privacy receipts, cancellation before timeout, and exact public session IDs after fork/refusal/restore/restart. The switch check proves client-close/transfer and eventual checked teardown; it does not claim an owner EOF acknowledgement. New ConPTY and undrained Windows request execution require native CI and are unrun locally.
