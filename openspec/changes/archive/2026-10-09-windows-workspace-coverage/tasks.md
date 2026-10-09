# Tasks

## 1. Existing native contracts

- [x] 1.1 Extend memory lifecycle/inspection/summary and portable hook-settlement fixtures; prove checked cleanup, one owner, durable isolated reads/writes, exact retry and no peer/public leakage.
- [x] 1.2 Extend existing delivery fixtures; retain exact profile destination and prove native failed-publication/cleanup refusal preserves owned evidence until successful retry, with actual recovery after stdin EOF.
- [x] 1.3 Port existing ToolHost MCP and hook contracts and extend terminal public-state/rendering tests; retain exact authorization, protocol, ownership, draft and privacy assertions.
- [x] 1.4 Verify rejected managed switch recovery retains the exact old claim; remote exports retain captured revision and reject foreign cursors without exposing later or candidate rows. Use existing managed owner/factory fixtures and await every claim/client/owner cleanup.
- [x] 1.5 Exercise existing public command/session contracts through ConPTY with completed frames and durable exact session state; verify hook cancellation while its native peer cannot consume the bounded request, output/privacy failures, and the original denied call/receipt reaching the next provider request without a file effect. Reuse existing helpers, bounds and cleanup scopes; add no production seams.
- [x] 1.6 Verify cold advisory refresh against the existing local origin, exact detached revision and allowlisted config; refuse executable config before changing checkout/source/adjacent state. Verify real Windows sharing-refused replace/delete retain exact original identity/bytes and persisted uncertain receipts, forbid replay, and permit only exact explicit discard before a fresh effect.
- [x] 1.7 Promote completed-answer/failed-post-hook CLI acceptance to Windows with settled private memory and projected diagnostics; exercise nonblocking stdout pending/success/error acknowledgement in the existing writer fixture.
- [x] 1.8 Exercise selected-peer stop/observe/invalid-hook authority and rewritten final-root refusal with existing portable runtime helpers; round-trip actual peer events through the public v2 adapter and reject mismatched or malformed envelopes without disclosing their payload. Verify canonical relationship history through v1/v2 adapters; withhold forged IDs, invalid membership and oversized projections without payload disclosure.
- [x] 1.9 Extend the existing glob/grep actor search contract: review only permitted candidate directories; accepted and unchanged review returns exact permitted matches, while denial returns no result/instructions. On Windows create a real second hard link during review and require revalidation refusal before any proposed instruction publication or result exposure, preserving original/adjacent bytes.

## 2. Verification

- [x] 2.1 Run relevant focused host tests, Windows-target static checks and independent reviews; state native-only checks that cannot run locally.
- [x] 2.2 Observe all new native contracts and final canonical95% gates on supported CI platforms before archive and merge.

Observed native acceptance: exact `01524423895fda853f763ed3176528facf5373c2`, [CI37891627860](https://github.com/replygirl/kuru/actions/runs/37891627860), passes every checked native behavior partition and installation/offline/update leg on Windows x64/ARM, all Linux ARM memory partitions and both native Windows platform jobs. This includes the real hard-link review refusal, sharing-refused file-effect recovery, selected-peer authority, canonical relationship adapters and all earlier scoped contracts.

Complete canonical line gates pass with the full unchanged Kuru source inventory and metric:

- Ubuntu:168363/176792 (95.23%),2657 tests across8 disjoint complete partitions.
- macOS:168482/176902 (95.24%),2649 tests across4 disjoint complete partitions.
- Windows:166265/174989 (95.01%),2431 tests across8 disjoint complete partitions.

Focused host checks pass: summary/attached inspection/lost reply1/1 each; connector hooks17/17 and MCP4/4; public UI2/2 and card rendering1/1; captured export1/1, rejected switch1/1, denied receipt1/1; stdout acknowledgement1/1, completed-answer/failed-post-hook CLI1/1, runtime hooks9/9, event adapters7/7, four-mode routing1/1, cold advisory1/1, actor search review1/1. All affected Windows-target lints and independent source reviews pass. Windows-only execution is established by native CI, not claimed locally.

The rejected-switch fixture proves client-close/transfer and eventual checked teardown, not owner EOF acknowledgement. The speaker fixture captures raw JSON in stock PowerShell and validates the exact actor/speaker/reason in Rust; removing redundant cmdlet parsing preserves product deadlines and all assertions. The Windows search fixture drops its host after awaited shutdown before unlinking the real alias. Production behavior, dependency/tool pins and the coverage inventory/metric remain unchanged by these final test additions.

Remaining workflow verification: the final archive commit must pass every exact-head PR check before squash merge, then every exact-main check. No merge, main pass or release is claimed by this archive evidence.
