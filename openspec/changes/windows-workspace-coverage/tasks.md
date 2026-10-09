# Tasks

## 1. Existing native contracts

- [ ] 1.1 Extend memory lifecycle/inspection/summary and portable hook-settlement fixtures; prove checked cleanup, one owner, durable isolated reads/writes, exact retry and no peer/public leakage.
- [ ] 1.2 Extend existing delivery fixtures; retain exact profile destination and prove native failed-publication/cleanup refusal preserves owned evidence until successful retry, with actual recovery after stdin EOF.
- [ ] 1.3 Port existing ToolHost MCP and hook contracts and extend terminal public-state/rendering tests; retain exact authorization, protocol, ownership, draft and privacy assertions.

## 2. Verification

- [x] 2.1 Run relevant focused host tests, Windows-target static checks and independent reviews; state native-only checks that cannot run locally.
- [ ] 2.2 Observe all new native contracts and final canonical95% gates on supported CI platforms before archive and merge.

Before implementation: the eight saved Windows line exports reconstruct exactly the CI canonical162174/172114; full inventory is retained. Doctor's existing portable command tests all pass but their profile collection discrepancy needs separate investigation, not duplicate tests. Existing Windows lifecycle creators deliberately terminated by the OS cannot flush normal LLVM exit profiles. The published-command fixture demonstrably clears and omits LLVM_PROFILE_FILE. Missing production routes include actual speaker decisions, post-hook annotation persistence, cold independent starters and attached inspection. Acceptance must remain observable and use existing native authorities; no synthetic permission failures, timer inflation or covered-line prediction is a pass.

Observed locally: summary selection1/1, attached inspection1/1, lost hook reply1/1; connector hooks17/17 and MCP4/4; public UI2/2 and card rendering1/1. Application and all affected package Windows-target lint and independent review pass. Native Windows creator EOF, independent starters, sharing refusal/bootstrap recovery and all promoted portable tests remain pending x64/ARM CI; no local execution is claimed for cfg(windows) tests.
