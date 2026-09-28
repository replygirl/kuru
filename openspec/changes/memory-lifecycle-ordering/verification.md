# Verification

## 0. Before-state reproduction and baseline (local macOS aarch64 host, 14 cores)

- [x] 0.1 @integration (agent) run the ignored measurement `store::lifecycle_measurement_tests::measure_lingering_session_consequence_for_delete_and_rename` with one injected lingering branch session at 20/100/1000 ms plus a control -> observed on commit 29028a98 (rebased as 284ec7cc) 2026-09-27: `delete_candidate_ref` failed in 3-4 ms with `candidate_owner stage=cleanup class=database sqlstate=HY000 vendor=1105 reason=other` 15/15; gated `transition_candidate` waited and succeeded 15/15; control passed the in-use check (`not fully merged`) 5/5.
- [x] 0.2 @benchmark (agent) trace close-time panics with `KURU_TEST_DOLT_LOG_DIR` over the full suites and a 100-iteration loop of kuru-runtime `dream::cancellation_tests::managed_lost_promotion_reply_keeps_report_and_publishes_only_typed_revision` (one process per iteration: `KURU_TEST_SUPERVISOR_PREPARED=1 RUST_TEST_THREADS=2 KURU_TEST_DOLT_LOG_DIR=<dir>/iter-N mise x -- cargo test -p kuru-runtime --lib --all-features --locked -- --exact <test> --test-threads=2 --show-output`) -> observed before-baseline: kuru-memory suite 1/778 and 2/782 servers panicked (4 and 4 directories removed before Dolt exited); kuru-runtime suite 2/337 (5); loop 169/895 panicked, 133 directories removed before exit, 132 unsuccessful supervisor exits.

## 1. Candidate refs are deleted only after their sessions end [critical]

- [ ] 1.1 @regression (agent) non-ignored test injects one lingering real server session on the candidate branch, then runs product `delete_candidate_ref` in the promote cleanup context, before and after the fix -> fails before with the exact `stage=cleanup … vendor=1105` record; after, waits the linger out and reaches the control outcome.
- [ ] 1.2 @regression (agent) same injection on the abandon path, `cleanup_abandoned_candidate` (the `-m b b` exclusion probe and the `-D` delete), before and after -> fails before; after, the ref is removed only once the session is absent from `information_schema.processlist`.
- [ ] 1.3 @integration (agent) a session that never ends, with a test-supplied retirement deadline -> the step fails as `CandidateFailureStage::PoolRetirement`, no ref changes, no `--force` is issued, and the managed client's uncertain-write fence still trips.
- [ ] 1.4 @integration (agent) run the canary `store::operational_gc_tests::candidate_pool_retirement_observes_exact_server_sessions_before_rename` with its assertion unchanged -> passes; `active_sessions=0` holds by product guarantee.
- [ ] 1.5 @unit (agent) failure record for the in-use message at `Cleanup` -> `reason=branch_in_use`; the reply sent to the client is unchanged.

## 2. Directories outlive their Dolt servers [critical]

- [ ] 2.1 @regression (agent) force a `MemoryStore::open` error after server start (for example the malformed reserved receipt schema writable reopen), then attempt the store directory's lifecycle lease without waiting, before and after -> before, the lease is still held by the background reaper (fails); after, the lease is acquirable immediately.
- [ ] 2.2 @regression (agent) the fixture-directory invariant against the unfixed `kuru-runtime/src/dream.rs` fixture and after the fix -> before, it FAILS the test and names and keeps the live directory; after, it passes.
- [ ] 2.3 @benchmark (agent) repeat 0.2 exactly (same suites, same loop command, same host) after the fix -> 0 close-time panics attributed to a removed directory, 0 directories removed before Dolt exited, 0 unsuccessful supervisor exits caused by a deleted directory; any residual panic reported with its trace.

## 3. Windows cancelled activation tears down stage before lock

- [ ] 3.1 @runtime (agent) windows-latest native CI runs kuru-memory `provision::native_tests::cancelling_checked_activation_recovery_drops_stage_before_cache_lock` (assertion `!stage_path.exists()` unchanged) and `persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage` -> both pass; run and job ids recorded.
- [ ] 3.2 @integration (agent) force a stage cleanup failure during a cancelled activation -> surfaced as a retained, reported stage decided before the cache lock releases; not swallowed.
- [ ] 3.3 @manual (agent) re-examine item 1's CI log (run 36270828072) -> its assertion's cause recorded as this teardown or as a separate, routed cause.

## 4. CI evidence and repository gates

- [ ] 4.1 @runtime (agent) named macOS evidence set at the fixed head: at least 12 macOS coverage partitions (three full native-tests runs × 4 macOS partitions), every run and job id recorded -> zero `stage=cleanup … vendor=1105` records and zero uncertain-write fences (corroboration only; natural rate was about 1 in 3-5 runs, so 1.1 and 1.2 are the proof).
- [ ] 4.2 @runtime (agent) Ubuntu and Windows native partitions and install jobs at the same head -> green; run ids recorded.
- [ ] 4.3 @integration (agent) `cargo fmt --all --check`, `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-runtime:lint`, full `mise run //packages/kuru-memory:test` and `//packages/kuru-runtime:test`, `mise run cospec -- validate memory-lifecycle-ordering --strict` -> all clean; no test excluded.
- [ ] 4.4 @integration (agent) workspace coverage gate including the branch's ignored measurement code -> at or above 90% with no exclusions; observed percentage recorded.
