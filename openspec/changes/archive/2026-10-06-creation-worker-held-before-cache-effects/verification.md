# Verification

## 1. Causal fixture synchronization [critical]

- [x] 1.1 @regression (agent) observe the actual creation worker waiting on an isolated existing marker -> task28385 exit0; isolated actual-worker test passed, template root and stage absent at entered Notify, same worker completed after marker removal with injected lock refusal and returned its startup File; no engine started in this causal test
- [x] 1.2 @regression (agent) run real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds -> app73434 exit0; unchanged exact case passed1/1 in12.16s, including original any-entry emptiness, creating sentence, build interval, completed frame/erasure, one template, restoration and actual quiescence assertions
- [x] 1.3 @regression (agent) run existing marker-budget and creation-forwarding cases -> task28385 exit0; both named cases passed with original budget and stage forwarding assertions
- [x] 1.4 @regression (agent) run cancelled_build_releases_key_lock_only_after_reap and observer-drop case -> task28385 exit0; both named cases passed, including original actual-engine key-lock/reap and independent open ownership assertions
- [x] 1.5 @unit (agent) run existing application closing guard -> app73434 exit0; tests::every_store_opening_async_test_runs_its_body_in_the_closing_scope passed1/1 in0.03s

## 2. Required scoped checks

- [x] 2.1 @integration (agent) run memory host/Windows lint and all-target typecheck -> static99199 exit0; host lint, Windows target lint and all-target/all-feature typecheck passed; release builds have no marker code and no public API changed
- [x] 2.2 @integration (agent) run docs, format, managed drift and final strict/apply context review -> static99199 docs/content/format/managed0; final52654e strict/apply both0, all four returned contexts reread; frozen source review remains clear
- [~] 2.3 @runtime (agent) execute corrected Linux/Windows native case -> defer: local macOS cannot establish native Linux or Windows behavior; Product owns fresh full PR CI

## Before-fix evidence

Official PR249 head0ad635e3 Ubuntu coverage5 job112530984704 failed the original cold PTY with `the store template cache was used before creation began`. The downloaded artifact preserves that primary error but no entry names or marker/frame state. The independent worker starts before its first filesystem effect while the activity observer alone holds CreatingDatabase; this is a source-proven causal gap, not proof of which historical cache entry appeared. No duplicate baseline is required.

## Local checks

Pre-edit strict9ad5e8 and applyd8adc4 both exited0 with all four returned contexts read. Root and independent frozen three-file source review found no material issue. The original terminal fixture is unchanged. Memory owning task28385 passed exactly5/5 selected tests in3.73s, total80.66s including required bundle/prepared-supervisor preparation; all remaining targets selected0. Native task uses FD4096, MBX_TARGET_VIEWS=0 and the existing isolated PXfSjA cache.

Initial sandbox formatting2d0273 exited101 on macOS system-configuration access, before completion. The ordinary escalated task7819d8 caught a draft missing semicolon; corrected Rust formatting213491 exited0. These are setup/syntax outcomes, not native passes. The frozen corrected source is used for the subsequent tests and statics.

Scoped static99199 exited0 in121.95s: host lint69.98s, Windows target lint47.17s and all-target/all-feature typecheck24.93s; docs build/content, all formatting checks and managed drift also passed. Windows compilation/lint is not native Windows execution. No source changed after the frozen checks began.

App owning task73434 exited0 in136.51s, selecting only the unchanged cold PTY and application closing guard. All remaining targets selected0. The actual worker owns the startup File during the cfg-only marker wait before ensure_private_directory/create_in; the progress observer remains independent. Normal builds without test-support have no marker read or wait. Exact old-head and later queue refs remain preserved. Both native tasks and all scoped static handles are terminal; no broader native suite, coverage, paid call, deadline, tool configuration or original PTY assertion changed.
