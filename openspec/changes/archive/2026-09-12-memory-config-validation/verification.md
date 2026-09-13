## 1. Validated memory overrides fail before mutation [critical]

- [x] 1.1 @regression (agent) run the focused `kuru-core` configuration test first against the pre-policy implementation, then after the correction -> prior memory-agent evidence observed the relative-path regression fail before the policy correction and pass after; native absolute nonexistent overrides remain accepted.
- [x] 1.2 @integration (agent) call `MemoryStore::open` with an invalid memory override and an uncreated data directory -> the combined coverage run passes `open_rejects_invalid_memory_config_before_creating_store_state`.
- [x] 1.3 @integration (agent) call public provisioning with an invalid memory override and an uncreated cache directory -> the combined coverage run passes `invalid_memory_config_fails_before_provision_creates_cache`.

## 2. Configuration-boundary compatibility

- [x] 2.1 @unit (agent) load an inherited memory configuration with a native absolute cache path and invalid startup timeout -> combined coverage passes `memory_options_validate_paths_timeouts_and_unknown_fields` and `memory_bootstrap_merges_files_without_validating_unloaded_saved_mode`.
- [x] 2.2 @manual (agent) review the configuration references alongside existing CLI and tool-path contracts -> integration docs validation passes after the memory agent’s focused configuration-reference update; only the two memory overrides were changed.

## 3. Scoped package verification

- [x] 3.1 @integration (agent) run `mise run //packages/kuru-core:test` -> prior memory-agent evidence reports the scoped core suite passed, including the configuration regression.
- [x] 3.2 @integration (agent) run `mise run //packages/kuru-memory:test` after coordinating its prepared real-Dolt fixture use -> prior memory-agent evidence reports 79 prepared-fixture tests passed in 94.98 seconds (56 library, 6 bundle, 4 memory, 12 lifecycle, 1 snapshot) without fixture weakening.
- [x] 3.3 @integration (agent) run the single coordinated combined coverage check after integrating concurrent Phase 0 changes -> observed `mise run coverage` exits 0 in 265.58 seconds, runs the full 79-test memory breakdown, and writes `target/coverage.lcov` under the enforced 90-percent line gate.

## Observed evidence

- 2026-09-12: the prior memory agent supplied the scoped core and prepared
  real-Dolt evidence recorded above. The integration verifier did not rerun
  those ordinary suites separately.
- 2026-09-12: the one coordinated workspace coverage run independently passed
  the relevant memory configuration, provisioning, store, and lifecycle tests.
