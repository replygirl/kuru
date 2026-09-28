# Tasks

## 1. Confirm the mechanism

- [x] 1.1 Read cargo-llvm-cov v0.9.1 `show-env` and `set_env` source and run the pinned `show-env` locally, and verify the `%<N>m` token equals the host CPU count
- [x] 1.2 Reconstruct the neutralised map for run 36426742472 from the tree at `95c79127` and verify both CI digests are reproduced by varying only `N`
- [x] 1.3 Enumerate every host-dependent value in the digested set and verify only the merge-pool size varies by hardware

## 2. Regression first

- [x] 2.1 Add `profile_environment_digests_ignore_the_host_merge_pool_size_only` and `captured_show_env_digests_agree_across_merge_pool_sizes` and verify both fail on unfixed code (`red.log`)

## 3. Fix

- [x] 3.1 Neutralise the `%<digits>m` pool size in `show-env:LLVM_PROFILE_FILE`'s file name before digesting and verify the new tests and `profile_environment_digests_ignore_target_paths_only` pass
- [x] 3.2 Upload the digested map as `profile-env.json`, derive the receipt digest from it, verify it in the merge and name differing keys, and verify `profile_environments_that_differ_are_refused_naming_each_key` and the evidence/merge tests pass
- [x] 3.3 Correct the negative-counter wording in `lines.rs` and `docs/development.md` and document the neutralisation and persisted map, and verify the refusal code is unchanged

## 4. Verification

- [x] 4.1 Run `//packages/kuru-delivery:test`, `format:check`, `//packages/kuru-delivery:lint`, `//packages/kuru-delivery:typecheck`, `lint:tooling`, `docs:check` and strict cospec validation and record observed evidence
