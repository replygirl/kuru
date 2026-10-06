# Verification

## 1. Owner-observed release before resume competition [critical]

- [x] 1.1 @regression (agent) run `busy_resume_and_continue_refuse_before_cli_provider_catalog` -> owning task 84527 terminal 0, exact case 1/1 passed in 5.76s: observed empty owner inventory precedes both competing launches; busy resume/continue contact no provider, retained revision is unchanged, loser never contacts catalog/inference, one winner settles the exact public turn.
- [x] 1.2 @integration (agent) run `tests::every_store_opening_async_test_runs_its_body_in_the_closing_scope` -> owning task 84527 terminal 0, exact closing guard 1/1 passed in 0.03s; total owning task 80.49s including preparation/compile, all other targets selected zero cases.

## 2. Scoped source and static checks

- [x] 2.1 @manual (agent) review the single fixture hunk and run affected host/Windows lint/type/docs/format/strict/managed checks -> Root frozen-hunk review clear; app host lint 71561, Windows-target lint 61412, typecheck 68002, docs 49566, format 55757 and managed 12030 all terminal 0; no production change, deadline broadening or safety assertion removal.
- [~] 2.2 @runtime (agent) native Windows ARM/x64 and combined coverage -> defer: Product owns subsequent repair PR and exact-main CI; local host checks are not native Windows evidence.

## Before-fix evidence

Main c2786319 CI37509354314 Windows ARM behavior partition 3 job112426709226 failed at the immediate `memory.live_session_drivers().await?.is_empty()` assertion after both local closes. Official existing log `/private/tmp/phase2-o3-main-arm3-112426709226.log` lines880–909 and extracted fixture stdout confirm this exact failure; no baseline or blind retry is needed.

Source establishes the causal distinction: the presence task drops its socket before its local close acknowledgment, while owner `serve_attached` drops the exact claim only after reading EOF. The corrected observation uses checked owner queries, never elapsed time as completion. Production release semantics and all original race assertions remain unchanged.

## Source and gate evidence

Actual strict validation 629e66 and apply 0e4708 each returned 0; all four required context files were read before editing. Root's independent frozen-hunk review is clear: twelve fixture-only added lines, unchanged final assertion, production APIs, authority and operation budgets. Source stayed frozen throughout task 84527. No paid inference, local coverage or native Windows execution was attempted.

Both documentation and format tasks passed; simultaneous ordinary setup emitted one nonfatal hk postinstall warning, followed by successful setup and read-only verification that all three normal hk hooks remain installed. No hook bypass, manual tool/configuration edit or extra native run was used. All local behavior and static handles are terminal. Final actual strict/apply/context closure and physical archive verification precede the normal hooked commit; Product owns subsequent repair PR/native CI/exact-main evidence.
