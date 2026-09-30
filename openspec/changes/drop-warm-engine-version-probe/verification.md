# Verification

## 1. Warm open launches no version-probe process and still refuses a byte-changed cache [critical]

- [ ] 1.1 @unit (agent) `observed_provision_reports_actual_cold_warm_and_failure_stages` (`packages/kuru-memory/src/provision/tests.rs`), warm branch -> observed warm stages equal exactly `[VerifyingRuntimeCache]`, no `CheckingRuntimeVersion`
- [ ] 1.2 @integration (agent) `actual_warm_cache_verifies_concurrently_while_installation_lock_is_held` (`packages/kuru-memory/src/provision/native_tests.rs`) against the real embedded/extracted Dolt engine -> two concurrent warm opens against a real cache both succeed and return the same binary path with no child process spawned by the warm branch
- [ ] 1.3 @regression (agent) `corrupt_cache_is_rejected_before_execution_and_links_are_never_adopted` and the in-place executable rewrite case in `observed_provision_reports_actual_cold_warm_and_failure_stages` -> a cached executable whose bytes were rewritten in place (same size) is still rejected with "cache is invalid" / "checksum mismatch", from the hash alone

## 2. Cold path still probes the freshly extracted engine before activation [critical]

- [ ] 2.1 @unit (agent) `observed_provision_reports_actual_cold_warm_and_failure_stages`, cold branch -> observed cold stages remain `[WaitingForRuntimeCache, ExtractingEmbeddedRuntime, CheckingRuntimeVersion]`, unchanged
- [ ] 2.2 @integration (agent) `real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution` or the equivalent native cold-install test on the current host target -> first extraction of the real bundled engine still runs `dolt version` before activation and fails closed on a wrong report

## 3. Behavior is otherwise unchanged (equivalence)

- [ ] 3.1 @equivalence (agent) full `kuru-memory` test suite (`cargo test -p kuru-memory`) -> passes unchanged except for the one updated stage assertion named in tasks 3.1 and the updated log text in 3.3
- [ ] 3.2 @unit (agent) `warm_verification_does_not_wait_for_the_installation_lock` -> unchanged pass, warm open still proceeds while the installation lock is held

## 4. Documentation matches the new behavior

- [ ] 4.1 @manual (human) read `docs/memory.md` and `apps/kuru-docs/concepts/memory.md` after the edit -> neither page claims a per-open (warm) version probe; both describe the install-time probe and the unconditional per-open payload hash

## 5. Coverage gate holds

- [ ] 5.1 @runtime (agent) `mise run coverage` -> workspace line coverage remains at or above 90% with the removed warm-path probe code and no newly dead lines in `provision.rs`
