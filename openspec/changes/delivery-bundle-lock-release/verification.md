# Verification

## 1. The stable bundle lock is released by its owner, not by the last descriptor [critical]

- [ ] 1.1 @regression (agent) `bundle::tests::dropped_lock_is_released_despite_an_inherited_duplicate` (hold a `try_clone` duplicate of the product lock, drop the product lock, `try_lock` a fresh handle) on unfixed code and after the fix -> FAILS with `WouldBlock` before, PASSES after; red output saved to the scratchpad `red.log`
- [ ] 1.2 @regression (agent) with `isolate_lock_release!` removed, the `bundle::` tests of the library test binary 20 times at default test threads, on unfixed product code and after the fix -> failures before (catalogue baseline 13/20 on 9def614b), 0/20 after, no retry or serialization
- [ ] 1.3 @regression (agent) the exact macOS partition-3 bundle trio (`build::tests::host_mode_and_provenance_are_refused_before_any_work`, `recovery_tests::cancellation_after_dropping_error_headers_releases_stage_and_stable_lock`, `tests::replaced_lock_and_directory_are_rejected_without_adopting_new_objects`) 20 times -> 0/20 failures after the fix

## 2. Order and platform parity

- [ ] 2.1 @unit (agent) existing cancellation tests (`cancelling_a_stalled_download_removes_stage_and_releases_stable_lock`, `cancellation_after_dropping_error_headers_releases_stage_and_stable_lock`) -> the stage is present while the lock is held and gone once it is released; lock identity preserved
- [~] 2.2 @runtime (agent) Windows compile and behavior of the shared guard (`bundle.rs`, `update.rs`) -> defer: no Windows host and no push in this workflow; proved by the `Coverage partition (windows-latest, N)` jobs and the Windows install/update jobs on the PR's CI run

## 3. Gates

- [ ] 3.1 @integration (agent) `mise run //packages/kuru-delivery:test`, `format:check`, `lint`, `typecheck`, `lint:tooling`, `docs:check`, `cospec validate --strict` -> all exit 0
