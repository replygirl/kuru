# Verification

## 1. Warm open launches no version-probe process and still refuses a byte-changed cache [critical]

- [x] 1.1 @unit (agent) `observed_provision_reports_actual_cold_warm_and_failure_stages` (`packages/kuru-memory/src/provision/tests.rs`), warm branch -> observed in `mise run //packages/kuru-memory:test` on the macOS host (2026-09-29): pass, warm stages equal exactly `[VerifyingRuntimeCache]`, no `CheckingRuntimeVersion`

- [x] 1.2 @integration (agent) `actual_warm_cache_verifies_concurrently_while_installation_lock_is_held` (`packages/kuru-memory/src/provision/native_tests.rs`) against the real embedded/extracted Dolt engine -> observed pass on the macOS host: two concurrent warm opens of a real cache return the same binary path without waiting for the held installation lock, and the in-place corruption is refused with "payload checksum mismatch"; this test asserts path equality and lock independence only, the no-process property is proven by 1.4

- [x] 1.3 @regression (agent) `corrupt_cache_is_rejected_before_execution_and_links_are_never_adopted` and the in-place executable rewrite case in `observed_provision_reports_actual_cold_warm_and_failure_stages` -> observed both pass unmodified: a cached executable rewritten in place (same size) is still rejected with "cache is invalid" / "checksum mismatch", from the hash alone

- [x] 1.4 @unit (agent) new `only_installation_executes_the_engine_and_warm_opens_still_refuse_changed_bytes` (`packages/kuru-memory/src/provision/tests.rs`), an engine fixture that appends one marker line per execution -> observed pass: installation executes it exactly once (stages `[WaitingForRuntimeCache, ExtractingEmbeddedRuntime, CheckingRuntimeVersion]`), two warm opens execute it zero times (each `[VerifyingRuntimeCache]`), and a same-length in-place rewrite that would still write the marker is refused with "cache is invalid" and "checksum mismatch" without executing

## 2. Cold path still probes the freshly extracted engine before activation [critical]

- [x] 2.1 @unit (agent) `observed_provision_reports_actual_cold_warm_and_failure_stages`, cold branch -> observed pass with the cold assertion unchanged, `[WaitingForRuntimeCache, ExtractingEmbeddedRuntime, CheckingRuntimeVersion]`; its wrong-version (`MISMATCH`, 2.3.2) branch and `failing_exact_version_probe_never_activates_and_releases_installation_authority` also pass, so the cold probe still fails closed

- [x] 2.2 @integration (agent) `real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution` or the equivalent native cold-install test on the current host target -> observed in CI run 36747697223 on head `2db4cfb7`: `provision::native_tests::real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution ... ok` on windows-latest (coverage partition 3, job 109998333670) and windows-11-arm (behavior partition 3, job 109998458908); on the macOS host, `actual_warm_cache_verifies_concurrently_while_installation_lock_is_held` cold-installed the real bundled engine through the unchanged cold probe and passed

## 3. Behavior is otherwise unchanged (equivalence)

- [x] 3.1 @equivalence (agent) full `kuru-memory` test suite via `mise run //packages/kuru-memory:test` -> observed exit 0: lib 343 passed, 0 failed, 3 ignored; integration binaries 10, 5, 12 and 1 passed, 0 failed; the only changed expectations are the warm stage assertion (tasks 3.1), the log text (tasks 3.3) and the added test in 1.4

- [x] 3.2 @unit (agent) `warm_verification_does_not_wait_for_the_installation_lock` -> observed unchanged pass, a warm open still proceeds while the installation lock is held

## 4. Documentation matches the new behavior

- [x] 4.1 @manual (human) read `docs/memory.md` and `apps/kuru-docs/concepts/memory.md` after the edit -> neither page claims a per-open (warm) version probe; both describe the install-time probe and the unconditional per-open payload hash -> observed 2026-09-30 by the orchestrating session on head 5456e2d0: `git diff origin/main` of both pages shows the warm-open sentences now say the engine is reused "without running the engine again for a version check" and installation "runs the new engine once to confirm its exact version"; no remaining sentence claims a per-open probe

## 5. Coverage gate holds

- [x] 5.1 @runtime (agent) `mise run coverage` -> observed in CI run 36754939511 on head `23f2bc3a` (the PR head): ubuntu-latest merge (job 110027307510) "coverage ubuntu-latest: 94.64% of lines (104035 of 109927) ... gate 90%", `provision.rs` 1049/1097 = 95.62%; macos-latest merge (job 110028268969) "coverage macos-latest: 94.62% of lines (104126 of 110036) ... gate 90%", `provision.rs` 1049/1097 = 95.62%; windows-latest merge (job 110029780538) "coverage windows-latest: 93.52% of lines (105565 of 112875) ... gate 90%", `provision.rs` 849/1093 = 77.67% (the lower figure and 1093-vs-1097 total reflect `cfg(windows)`/`cfg(unix)` conditional compilation on that target, not new untested lines). All three coverage merges report at or above the 90% gate on one run of the final code; tasks.md 3.5 records the same evidence
