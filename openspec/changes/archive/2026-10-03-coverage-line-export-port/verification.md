# Verification

## 1. Diagnosis [critical]

- [x] 1.1 @runtime (agent) CI run 37147327774 artifacts and logs -> observed: partition 3's diagnostics hold only `coverage.summary.json` (no raw export); job log reports finished at 19:26:42.63 (lcov), 19:26:45.94 (json), 19:26:48.08 (summary) after tests ended 19:26:39.06; `kuru_memory` ran 19:24:17-19:26:14 including stand-in test `progress_wait::a_retired_record_fails_at_once`; macOS partitions 1, 2 and 4 of the same commit each printed "reproduces cargo-llvm-cov's summary exactly" (130748 mapped lines, 22399 instantiations)
- [x] 1.2 @equivalence (agent) local instrumented `a_retired_record_fails_at_once` (cargo-llvm-cov 0.9.1, LLVM 22.1.8-rust-1.98.1), export A before and B after the stand-in's late profile (written 119 s after the test exited), real port over each -> observed: port reproduces A (service.rs 465/7612) and B (480/7612) against their own summaries and B against `B.summary.json`; port(A) vs `B.summary.json` fails "service.rs: llvm-cov 480/7612, port 465/7612" with mapped equal, the CI shape; late-only lines 161, 163, 164, 169, 249, 250, 252, 258, 263, 264, 267, 269 (`service_entry`, `stand_in_owner`)

## 2. Stand-in owner never outlives its test [critical]

- [x] 2.1 @regression (agent) `a_stand_in_has_consumed_its_release_when_its_start_returns` with the fix reverted, then with it -> observed: reverted, FAILED "the stand-in's release was still unconsumed when its start returned" and 6 stand-in processes alive after the binary exited; fixed, all 7 `progress_wait` tests passed
- [x] 2.2 @runtime (agent) the `service::` lib tests, then `--internal-memory-service` processes after the binary exits -> observed: before the fix 4 stand-ins lived 121 s past their start (14:49:13-14:51:14) after a `service::` run; with the fix 136 passed and 0 owners at every sample from t+0.06 s to t+11 s after exit

## 3. Partition refuses a changed profile set [critical]

- [x] 3.1 @regression (agent) `a_profile_written_during_export_or_receipt_fails_the_partition_by_name` with the guard removed, then with it -> observed: removed, FAILED with "coverage partition line export does not reproduce cargo-llvm-cov's summary" (the CI message) for a profile written during `--json`; with the guard, every case (during lcov, json, summary, receipt) fails naming "new kuru-late-1.profraw" and no case blames the port
- [x] 3.2 @unit (agent) `profile_sets_name_new_changed_and_removed_profiles` and the coverage module tests -> observed: `cargo test -p kuru-delivery --all-features --lib coverage` 102 passed, 0 failed (includes `clang_exports_with_every_region_kind_are_reproduced_exactly` and `hosted_linux_and_windows_partitions_are_reproduced_exactly`, unchanged)

## 4. Designed behaviour unchanged

- [x] 4.1 @integration (agent) `mise run //packages/kuru-delivery:test` and `mise run //packages/kuru-memory:test` -> observed: delivery exit 0, 380 passed, 0 failed; memory exit 0, 728 passed, 0 failed, 6 ignored; 0 owner processes afterwards
- [~] 4.2 @runtime (agent) a coverage shard partition on a native runner with this change -> defer: the instrumented shard was not run locally (only one instrumented stand-in test); PR CI runs every partition and must show each one reproducing its summary with an unchanged profile set

## 5. Static checks

- [x] 5.1 @unit (agent) `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck`, `mise run lint:tooling`, `mise run docs:check`, `mise run cospec -- validate --all --strict` -> observed: each exit 0 (run with `NODE_OPTIONS` unset); validate 0 errors, 0 warnings
