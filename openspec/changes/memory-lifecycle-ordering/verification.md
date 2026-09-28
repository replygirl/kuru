# Verification

## 0. Before-state reproduction and baseline (local macOS aarch64 host, 14 cores)

- [x] 0.1 @integration (agent) run the ignored measurement `store::lifecycle_measurement_tests::measure_lingering_session_consequence_for_delete_and_rename` with one injected lingering branch session at 20/100/1000 ms plus a control -> observed on commit 29028a98 (rebased as 284ec7cc) 2026-09-27: `delete_candidate_ref` failed in 3-4 ms with `candidate_owner stage=cleanup class=database sqlstate=HY000 vendor=1105 reason=other` 15/15; gated `transition_candidate` waited and succeeded 15/15; control passed the in-use check (`not fully merged`) 5/5.
- [x] 0.2 @benchmark (agent) trace close-time panics with `KURU_TEST_DOLT_LOG_DIR` over the full suites and a 100-iteration loop of kuru-runtime `dream::cancellation_tests::managed_lost_promotion_reply_keeps_report_and_publishes_only_typed_revision` (one process per iteration: `KURU_TEST_SUPERVISOR_PREPARED=1 RUST_TEST_THREADS=2 KURU_TEST_DOLT_LOG_DIR=<dir>/iter-N mise x -- cargo test -p kuru-runtime --lib --all-features --locked -- --exact <test> --test-threads=2 --show-output`) -> observed before-baseline: kuru-memory suite 1/778 and 2/782 servers panicked (4 and 4 directories removed before Dolt exited); kuru-runtime suite 2/337 (5); loop 169/895 panicked, 133 directories removed before exit, 132 unsuccessful supervisor exits.

## 1. Candidate refs are deleted only after their sessions end [critical]

- [x] 1.1 @regression (agent) non-ignored test injects one lingering real server session on the candidate branch, then runs product `delete_candidate_ref` in the promote cleanup context, before and after the fix -> fails before with the exact `stage=cleanup … vendor=1105` record; after, waits the linger out and reaches the control outcome. Observed 2026-09-27, round 1 at 220997cf: `operational_gc_tests::promoted_cleanup_deletes_status_ref_only_after_its_lingering_session_ends` red before the fix (`candidate_owner stage=cleanup class=database sqlstate=HY000 vendor=1105 reason=other`, lane log `fix/h1-red.log`), green in both full kuru-memory runs of round 1. The 0.1 measurement repeated after the fix: the injected-linger delete column now matches the control in 15/15 (`sql=HY000/1105/not_merged`), with latency tracking the linger: 20 ms -> 29-31 ms, 100 ms -> 107-111 ms, 1000 ms -> 1006-1021 ms (control 4-5 ms); the gated rename is unchanged (65-69 / 141-156 / 1058-1071 ms) and `abandon()` succeeded 20/20 (`m1-consequence/consequence-*.csv`).
- [x] 1.2 @regression (agent) same injection on the abandon path, `cleanup_abandoned_candidate` (the `-m b b` exclusion probe and the `-D` delete), before and after -> fails before; after, the ref is removed only once the session is absent from `information_schema.processlist`. Observed `abandoned_cleanup_probes_and_deletes_only_after_its_lingering_session_ends` red before (the same record, from the `-m b b` exclusion probe), green in both round-1 kuru-memory runs; the test asserts the session is still in `information_schema.processlist`, the ref intact and no `DOLT_BRANCH` issued while the session lives.
- [x] 1.3 @integration (agent) a session that never ends, with a test-supplied retirement deadline -> the step fails as `CandidateFailureStage::PoolRetirement`, no ref changes, no `--force` is issued, and the managed client's uncertain-write fence still trips. Observed `candidate_deletion_refuses_a_session_that_outlives_retirement` passes for `-d` and `-D` with a 500 ms test deadline: record `candidate_owner stage=pool_retirement class=non_sql sqlstate=none vendor=0 reason=other`, ref intact, no uncertain write pending, no `--force`. Recorded limits: through `promote()`/`abandon()` the outer `Cleanup` context shadows the inner stage, so the same timeout records as `stage=cleanup class=non_sql vendor=0 reason=other` (still distinct from a Dolt 1105). The managed-client fence leg was not exercised end to end by a new test: by reading, rpc.rs maps any untyped error to `ServiceFault::StorageFailed` and `ServiceAttachment::has_definite_mutation_reply` (service.rs ~305) refuses to treat a `StorageFailed` reply as mutation proof, and neither file changed on this branch; the fence mechanism itself is exercised by `facade::tests::candidate_promotion_recovery_uses_exact_target_and_releases_clone_fence` (passed).
- [x] 1.4 @integration (agent) run the canary `store::operational_gc_tests::candidate_pool_retirement_observes_exact_server_sessions_before_rename` with its assertion unchanged -> passes; `active_sessions=0` holds by product guarantee. Observed canary unchanged byte for byte; 200/200 in the round-1 memory loop with `active_sessions=0` after `retire_pool` in 200/200, and passed in both full suites. It calls `Server::retire_pool` directly, so it still detects natural linger rather than being covered by the store-level wait; catalogue item 11's canary sub-item is therefore not resolved by this change (recorded, not changed).
- [x] 1.5 @unit (agent) failure record for the in-use message at `Cleanup` -> `reason=branch_in_use`; the reply sent to the client is unchanged. Observed `branch_rename_reason_requires_the_exact_pinned_dolt_error` passes (`Cleanup` + exact message -> `branch_in_use`; other stage, message, sqlstate or vendor -> `other`). No rpc or reply code changed.

## 2. Directories outlive their Dolt servers [critical]

- [x] 2.1 @regression (agent) force a `MemoryStore::open` error after server start (for example the malformed reserved receipt schema writable reopen), then attempt the store directory's lifecycle lease without waiting, before and after -> before, the lease is still held by the background reaper (fails); after, the lease is acquirable immediately. Observed the three `store::open_error_reap_tests` (staged `.staging-*` pool, active unsupported schema version, post-store usage ledger) failed before (`MemoryStore::open returned an error while its Dolt server was still live`, lane log `fix/h2a-red.log`) and pass in both round-1 kuru-memory runs.
- [x] 2.2 @regression (agent) the fixture-directory invariant against the unfixed `kuru-runtime/src/dream.rs` fixture and after the fix -> before, it FAILS the test and names and keeps the live directory; after, it passes. Observed before the fixture fixes the invariant failed 5 kuru-runtime tests, each naming its live owner (lane log `fix/runtime-before.log`); after, 0 invariant hits in kuru-memory (rerun), kuru-runtime, kuru-tui and kuru-delivery suites, and 0 in both loops. One hit in the first round-1 kuru-memory run was a false positive of the invariant itself, fixed in 220997cf (see round 1 below).
- [x] 2.3 @benchmark (agent) repeat 0.2 exactly (same suites, same loop command, same host) after the fix -> 0 close-time panics attributed to a removed directory, 0 directories removed before Dolt exited, 0 unsuccessful supervisor exits caused by a deleted directory; any residual panic reported with its trace. Observed (round 1, same host, same commands as 0.2; see the before/after table below): 0 close-time panics, 0 directories removed before Dolt exited and 0 unsuccessful supervisor exits in every traced run. No residual to explain.

## 3. Windows cancelled activation tears down stage before lock

- [ ] 3.1 @runtime (agent) windows-latest native CI runs kuru-memory `provision::native_tests::cancelling_checked_activation_recovery_drops_stage_before_cache_lock` (assertion `!stage_path.exists()` unchanged) and `persistent_held_descendant_exhausts_checked_recovery_and_preserves_stage` -> both pass; run and job ids recorded.
- [ ] 3.2 @integration (agent) force a stage cleanup failure during a cancelled activation -> surfaced as a retained, reported stage decided before the cache lock releases; not swallowed.
- [ ] 3.3 @manual (agent) re-examine item 1's CI log (run 36270828072) -> its assertion's cause recorded as this teardown or as a separate, routed cause.

## 4. CI evidence and repository gates

- [ ] 4.1 @runtime (agent) named macOS evidence set at the fixed head: at least 12 macOS coverage partitions (three full native-tests runs × 4 macOS partitions), every run and job id recorded -> zero `stage=cleanup … vendor=1105` records and zero uncertain-write fences (corroboration only; natural rate was about 1 in 3-5 runs, so 1.1 and 1.2 are the proof).
- [ ] 4.2 @runtime (agent) Ubuntu and Windows native partitions and install jobs at the same head -> green; run ids recorded.
- [ ] 4.3 @integration (agent) `cargo fmt --all --check`, `mise run //packages/kuru-memory:lint`, `mise run //packages/kuru-runtime:lint`, full `mise run //packages/kuru-memory:test` and `//packages/kuru-runtime:test`, `mise run cospec -- validate memory-lifecycle-ordering --strict` -> all clean; no test excluded. Observed locally in round 1 (all exit 0 unless noted): `format:check`, `lint` (workspace clippy), `typecheck`, `lint:tooling`, `docs:check`, `cospec:managed:check`, `cospec -- validate memory-lifecycle-ordering --strict`; `//packages/kuru-memory:test` 324 pass / 1 fail / 3 ignored at 171b1903, then 325 pass / 0 fail / 3 ignored at 220997cf; `//packages/kuru-runtime:test` 220/0; `//apps/kuru-tui:test` 245/0 (4 ignored); `//packages/kuru-delivery:test` 313/0 (1 ignored); `//packages/kuru-memory:lint` rerun at 220997cf clean. `//packages/kuru-runtime:lint` is covered by the workspace `lint`. No test excluded. Open: this item closes once the pushed head's CI static jobs agree.
- [ ] 4.4 @integration (agent) workspace coverage gate including the branch's ignored measurement code -> at or above 90% with no exclusions; observed percentage recorded.

## Round 1 local verification (2026-09-27, macOS 27.0 arm64, 14 cores)

Tree under test: `fix/memory-lifecycle-ordering` at 34a1a9b4 + f5ec9c1b + ddeea3dc,
plus 171b1903 (docs and `measure:lifecycle` task) and 220997cf (round-1 fix).
The Windows lane's uncommitted draft (task 4.1) was stashed for the round
(`stash windows-lane-uncommitted`, backup `$S/fix/verify-round1/windows-lane.patch`)
and restored afterwards, so no result below includes it. Logs, traces and
CSVs: `$S/fix/verify-round1/` (`$S` = the investigation scratch directory named in measure.md).
Steps ran strictly one at a time, with no other build in the worktree.

| measure | before (0.1 / 0.2) | after (round 1) |
|---|---|---|
| injected-linger `delete_candidate_ref`, 20/100/1000 ms | fails in 3-4 ms, 1105 `stage=cleanup reason=other`, 15/15 | waits, then control outcome (`not_merged`) 15/15; 29-31 / 107-111 / 1006-1021 ms |
| gated rename, same lingers | waits, ok 15/15 | waits, ok 15/15 |
| kuru-runtime loop, 100 x `managed_lost_promotion_reply...` | 99 pass / 1 fail (readiness); 169 panics of 895 servers; 133 dirs removed before exit; 132 unsuccessful supervisor exits | 100 pass / 0 fail; 0 panics of 900 servers; 0; 0 |
| kuru-memory loop, 200 x 5 tests | 1000/1000; 0 of 3,000; 0; canary `active_sessions=0` 200/200 | 1000/1000; 0 of 3,000; 0; 0 unsuccessful exits; canary `active_sessions=0` 200/200 |
| kuru-memory full suite (traced) | 1 and 2 panics of 778/782; 4 and 4 dirs removed | 0 of 798 (run 1) and 0 of 794 (rerun); 0 and 0 |
| kuru-runtime full suite (traced) | 2 panics of 337; 5 dirs removed | 0 of 337; 0 |
| kuru-tui suite (traced) | not measured | 0 panics of 350 servers; 0 dirs removed |

Round-1 fix. The first kuru-memory run failed
`service::tests::purge_refuses_a_live_service_owner_before_writing_intent`
through the new fixture invariant, naming the test's own
`.service-owner.lock` as live after the test had dropped it. That test (and
`inspection_waits_for_a_booting_owner_without_starting_dolt`) took a real
owner flock under a guarded root without `spawn_gate`; a sibling test's spawn
duplicated the descriptor, keeping the flock held past release (the mechanism
`spawn_gate.rs` documents). 220997cf makes both hold
`spawn_gate::locking_async` for their whole body, as the other lock-taking
tests do; neither spawns. No assertion changed and the invariant still never
waits. The rerun passed 325/0. Remaining exposure: in-process owners in tests
that hold the shared `spawning` guard can still race another spawner; 0 hits
across the other round-1 suite runs (1,427 test results) and 300 loop iterations.

Remaining in-process `owner_dropped_live` events (4 in each kuru-memory run,
111 in kuru-runtime, 16 in kuru-tui) never left a directory missing at Dolt's
stop or exit; the kuru-memory four are the deliberate cancellation-handoff
tests (`retained=true`).

Readiness family: no `memory service readiness deadline exceeded` in any
round-1 log (baseline: 1 in 100 runtime iterations). Not a change target here.

Unrun in round 1, with the job that proves each:
- All Windows behaviour (3.1-3.3, the Windows branches of the invariant and
  `open_error_reap_tests`): windows-latest native test partitions and install
  jobs in `ci.yml`; blocked on the task 4.1 maintainer decision.
- The macOS 26 3-vCPU natural linger trigger (4.1): the macOS coverage
  partitions of three full native-tests runs at the pushed head. This host
  never produced natural linger, so 1.1/1.2 remain the proof.
- Ubuntu native partitions (4.2) and the coverage gate (4.4): the CI coverage
  partitions and Ubuntu merge job at the pushed head; coverage was not run locally.
