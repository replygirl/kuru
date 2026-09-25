## 1. Authority

- [x] 1.1 Reject pre-tool rewrites that change the tool name at every hop, and share one rewrite validator between connectors and runtime (`PreTurnValue::checked`, `PreToolValue::checked`); verify with the extended connector invalid-intermediate-rewrite test (the name-change case runs no later hook).
- [x] 1.2 Admit the calls the runtime dispatches itself (deliberation, dream and cognitive speaking calls) only against the tools offered for that exact request and phase, leaving other speaking calls to ToolHost permission evaluation of the exact final call; verify with a regression test in which a deliberation hook rewrites `remember` to an allowed `a2a_send` and nothing is sent. The test fails before the fix and passes after.

## 2. Process cleanup

- [x] 2.1 Track in-flight hook workers on `HookHost`, and await bounded quiescence at turn and dream completion and in `ToolHost::shutdown`; verify with a readiness-synchronized cancellation test that observes the started hook group, cancels, quiesces, and asserts the group is absent, plus a runtime cancellation test that finds no in-flight hook when the turn returns.
- [x] 2.2 Bound stdout and stderr drains after root exit; verify with a Unix test in which an escaped `setsid` descendant holds stdout, the hook fails within its deadline, and the operation returns.

## 3. Records and diagnostics

- [x] 3.1 Record pre-turn rewrite provenance before the rewritten current input; verify that the private-history rows carry a `kuru-hook` pre_turn record adjacent to the rewritten input.
- [x] 3.2 Report suppressed configured hooks as typed `suppressed` observations; verify with a connector test that passes the origin value explicitly (the test runner's environment stays unchanged), plus event validity.
- [x] 3.3 Apply the `PSModulePath` policy to Windows hook launches (stock PowerShell only); verify the selection function with a host-independent unit test. The native launch remains a Windows CI gate because the local cross-target check is blocked by `aws-lc-sys`.
- [x] 3.4 Convert runtime hook-test message-text assertions to typed `Event::Hook` matches, and share the unresolved-annotation value between engine and CLI; verify the focused runtime hook suite.

## 4. Calibration

- [x] 4.1 Test `HookBudget` with an injected clock, and replace the wall-clock budget test with a marker-synchronized real-process test; verify the focused connector tests.
- [x] 4.2 Replace the 120 s and 360 s Windows literals with a shared hook-launch warm-up outside the budget and the 5 000 ms default, and derive observation waits and aggregates from `timeout_ms`. The native run remains a Windows CI gate, as it cannot be compiled on this host.

## 5. Documentation and evidence

- [x] 5.1 Document the argument-only rewrite, the offered-set admission, the provenance record, the suppression variable and the Windows environment in `docs/configuration.md`, `docs/protocols.md` and the public configuration reference; verify `docs:check`.
- [x] 5.2 Run focused connector, runtime and TUI hook tests, lint and format; record observed evidence in `verification.md`, and name unrun checks.
