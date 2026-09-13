## 1. Exact public release and ordinary mise installation [critical]

- [x] 1.1 @unit (agent) validate exact release/tag metadata, complete assets, checksums, and selected executable digests -> focused verifier tests passed exact inventory/digest acceptance and missing, duplicate, extra, URL, and digest rejection on 2026-09-13
- [~] 1.2 @integration (agent) exercise native Windows mise acceptance with the shared isolated environment helper -> defer: the source refactor is complete; native Windows candidate acceptance awaits the first branch CI run
- [~] 1.3 @e2e (agent) run `verify:published-windows` in the actual Release workflow -> defer: the exact public release exists only after this workflow change is archived, merged, and dispatched

## 2. Cold persistent application behavior [critical]

- [x] 2.1 @unit (agent) verify the command plan and result parser -> focused tests passed the exact ordinary selector, required machine fields, JSON-only stdout, and acceptance of separate informational stderr on 2026-09-13
- [~] 2.2 @e2e (agent) run the published executable on native Windows with empty data and offline engine caches -> defer: requires the next actual Release run after this change merges

## 3. Isolation, cleanup, and evidence [critical]

- [~] 3.1 @integration (agent) exercise shared environment construction and bounded child failures -> defer: requires native Windows candidate acceptance and the published verifier job; host source/type checks are green
- [x] 3.2 @unit (agent) serialize a completed receipt and rejected result -> focused receipt and held-file tests passed the 64 KiB fixed schema, absence of raw output/credential fields, and bounded checked reads on 2026-09-13; source review confirmed receipt creation follows isolated-root close
- [~] 3.3 @e2e (agent) upload the native Release job receipt -> defer: requires the next actual Release run after this change merges

## 4. Workflow and documentation gating [critical]

- [x] 4.1 @regression (agent) inspect the Release workflow contract -> focused workflow test passed exact bump SHA, plan/bump/publish needs, pinned mise setup-only token scope, package task, receipt upload, and documentation dependency on 2026-09-13
- [~] 4.2 @integration (agent) run delivery static checks and documentation validation -> defer: host delivery lint/typecheck, no-feature library check, task discovery, strict cospec, and diff check passed; Taplo is blocked locally by the sandbox SystemConfiguration panic and docs/native checks await normal hooks
- [~] 4.3 @runtime (human) rerun an interrupted post-publication verifier in its existing Release run -> defer: no verifier run exists before merge; rerun only if a post-publication attempt is interrupted or fails
