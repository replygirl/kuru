## 1. Authority

- [ ] 1.1 Reject pre-tool rewrites that change the tool name at every hop, and share one rewrite validator between connectors and runtime; verify with the extended connector invalid-intermediate-rewrite test (the name-change case runs no later hook).
- [ ] 1.2 Admit final calls only against the tools offered for that exact request and phase (deliberation, speaker round, parallel-read wave, dream); verify with a regression test in which a deliberation hook rewrites `remember` to `a2a_send`, and there is no a2a dispatch and no approval request. The test fails before the fix and passes after.

## 2. Process cleanup

- [ ] 2.1 Track in-flight hook workers on `HookHost`, and await bounded quiescence at turn and dream completion and in `ToolHost::shutdown`; verify with a readiness-synchronized cancellation test that observes the started hook group, cancels, quiesces, and asserts the group is absent.
- [ ] 2.2 Bound stdout and stderr drains after root exit; verify with a Unix test in which an escaped `setsid` descendant holds stdout, the hook fails within its deadline, and the operation returns.

## 3. Records and diagnostics

- [ ] 3.1 Record pre-turn rewrite provenance before the rewritten current input; verify that the private-history rows carry a `kuru-hook` pre_turn record adjacent to the rewritten input.
- [ ] 3.2 Report suppressed configured hooks as typed `suppressed` observations; verify with a connector test using a child-process environment, plus event validity.
- [ ] 3.3 Apply the PSModulePath policy to Windows hook launches (stock PowerShell only); verify by cross-target compile checking.
- [ ] 3.4 Convert runtime hook-test message-text assertions to typed `Event::Hook` matches; verify the focused runtime hook suite.

## 4. Calibration

- [ ] 4.1 Test `HookBudget` with an injected clock, and replace the wall-clock budget test with a marker-synchronized real-process test; verify the focused connector tests.
- [ ] 4.2 Replace the 120 s and 360 s Windows literals with a HookHost warm-up outside the budget and the 5 000 ms default, and derive observation waits from `timeout_ms`; verify by cross-target compile check (native run remains a CI gate).

## 5. Documentation and evidence

- [ ] 5.1 Document the argument-only rewrite, the offered-set admission, the provenance record, the suppression variable and the Windows environment in `docs/configuration.md` and `docs/protocols.md`; verify `docs:check`.
- [ ] 5.2 Run focused connector and runtime tests, lint, typecheck and format, record observed evidence in `verification.md`, and name unrun checks.
