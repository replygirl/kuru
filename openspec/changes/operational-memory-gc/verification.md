## 1. Receipt replacement and reconciliation [critical]

- [x] 1.1 @integration (agent) perform repeated actual-Dolt mutations and inspect active receipts and revisions -> exactly one current receipt remains while every committed revision is readable
- [x] 1.2 @integration (agent) lose a committed mutation reply, then submit another mutation -> the prior receipt reconciles before the next transaction replaces it and neither mutation replays

## 2. Candidate lifecycle recovery [critical]

- [x] 2.1 @integration (agent) recreate persisted equal and mismatched dual-ref states corresponding to the non-atomic copy/delete boundary -> equal refs reconcile under one status authority, while mismatched heads, dirty state or uncertain teardown delete nothing
- [x] 2.2 @integration (agent) stop and reopen around promotion merge and cleanup -> startup never performs an unmerged promotion, reclaims only an eligible head already reachable from live, preserves ordinary/unrelated/ineligible candidates exactly and permits ordinary main use after settled inspection
- [x] 2.3 @integration (agent) explicitly abandon after an accepted candidate write and while another view is held -> accepted content settles, live remains unchanged, held-view uncertainty preserves the branch, and owned release permits exact cleanup without replay

## 3. Runtime dream outcomes [critical]

- [x] 3.1 @runtime (agent) cancel and fail real-Dolt dreams before and after accepted candidate writes -> explicit abandonment settles each accepted write and leaves live history valid while a later turn succeeds
- [x] 3.2 @regression (agent) compose confirmed-result/dirty-cleanup recovery with promotion-winning runtime cancellation -> exact main memory and topology publish once, repeated promotion returns the confirmed target, and cleanup remains independently retryable

## 4. Pinned automatic GC and retention [critical]

- [x] 4.1 @integration (agent) read the effective pinned-engine setting and run actual `DOLT_GC('--full')` with live, candidate, historical and revision-pinned export reads -> automatic GC is enabled, the collector completes, referenced views remain byte-exact and existing connections remain usable
- [x] 4.2 @integration (agent) interrupt owned startup candidate recovery and reopen -> supervisor reaping retains lifecycle authority and the reopened store reports only states justified by exact refs
- [x] 4.3 @regression (agent) inspect both memory guides and built docs -> no note/chat expiry, reachable-revision pruning or secure-erasure claim appears and retained-history growth is explicit

## 5. Quality and portability

- [x] 5.1 @integration (agent) run focused memory/runtime Rust format, lint, typecheck and actual-Dolt tests -> affected checks pass and exact observed outcomes are recorded
- [ ] 5.2 @integration (agent) run one coordinated workspace coverage task plus native macOS/Linux and Windows real-Dolt lifecycle fixtures -> workspace line coverage remains at least 90 percent and each platform proves effective GC, ref recovery, session ownership and historical readability

## Observed evidence

- 2026-09-13 macOS arm64: `mise run //packages/kuru-memory:test -- operational_gc_tests` passed all eight focused actual-Dolt receipt, branch lifecycle, startup cancellation and full-GC fixtures. Log: `/private/tmp/kuru-operational-gc-memory-tests.log`.
- 2026-09-13 macOS arm64: the focused runtime tests `accepted_dream_promotion_wins_cancellation_and_publishes_exactly`, `explicit_cancellation_keeps_live_dream_state_and_private_histories_isolated` and `stale_promotion_keeps_later_live_data_and_discards_all_candidate_effects` passed against actual Dolt. Log: `/private/tmp/kuru-operational-gc-runtime-tests.log`.
- 2026-09-13 macOS arm64: `configured_startup_budget_is_not_preempted_by_a_shorter_query_timer` passed and observed `@@GLOBAL.dolt_auto_gc_enabled = 1`. Log: `/private/tmp/kuru-operational-gc-server-setting.log`.
- 2026-09-13 local static checks: `mise run //packages/kuru-memory:typecheck`, `mise run //packages/kuru-runtime:typecheck`, both package lint tasks, Rust formatting, strict cospec validation and the actual acknowledged apply gate passed. `mise run docs:check` built and checked the curated site successfully. The root `format:check` aggregate reached an unchanged TOML check whose pinned Taplo process panicked in macOS `system-configuration` while creating a dynamic-store object; no TOML file is changed and the affected Rust format gate passes.
- Pending: the single coordinated workspace coverage run and full native macOS/Linux/Windows acceptance. The focused macOS cases above establish local native behavior for the changed paths but do not replace the coordinated platform row.
