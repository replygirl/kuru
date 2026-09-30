# Tasks

## 1. Warm-path implementation

- [ ] 1.1 In `packages/kuru-memory/src/provision.rs`, remove the `progress.report(MemoryOpenStage::CheckingRuntimeVersion)` call and the `checked.probe(binary.clone()).await?` call from `verified_cache_observed`, returning `Ok(binary)` directly after `CheckedCache::open_and_verify` (which already revalidates identities via its own `checked.revalidate()` call) -> code review + warm stage assertion in 3.1
- [ ] 1.2 Delete `CheckedCache::probe` (the warm-path process-spawning method) entirely; keep `CheckedCache::revalidate` as used by `open_and_verify` -> `cargo build -p kuru-memory` compiles clean
- [ ] 1.3 Confirm no other call site references the removed `CheckedCache::probe`, and that `verify_version`, `private_probe` and the cold-path `StagedInstall`/probe struct remain used (grep `verify_version(` and `private_probe(` in `provision.rs`) -> grep output shows only the cold-path (`provision_observed`'s `dolt_binary` override) and the cold `StagedInstall` probe call sites remain
- [ ] 1.4 Confirm the cold path (`provision_with_extractor_observed`, extraction + first activation) is untouched: it still reports `MemoryOpenStage::CheckingRuntimeVersion` and probes the freshly extracted engine before activation -> code review of `provision.rs` around the cold branch
- [ ] 1.5 Grep `provision.rs` for `cfg(windows)` in the touched region and confirm no Windows-only code references the removed probe path -> grep output

## 2. Progress reporting

- [ ] 2.1 Confirm `packages/kuru-memory/src/progress.rs` needs no change: `MemoryOpenStage::CheckingRuntimeVersion` remains a valid stage value, only its warm-path emission is removed -> code review

## 3. Tests

- [ ] 3.1 Update `observed_provision_reports_actual_cold_warm_and_failure_stages` (`packages/kuru-memory/src/provision/tests.rs`) so the warm-open assertion expects `[MemoryOpenStage::VerifyingRuntimeCache]` (dropping `CheckingRuntimeVersion`); leave the cold-open assertion (`[WaitingForRuntimeCache, ExtractingEmbeddedRuntime, CheckingRuntimeVersion]`) and the corrupt-cache assertions unchanged -> `cargo test -p kuru-memory observed_provision_reports_actual_cold_warm_and_failure_stages`
- [ ] 3.2 Confirm `corrupt_cache_is_rejected_before_execution_and_links_are_never_adopted` (`tests.rs`) and `warm_verification_does_not_wait_for_the_installation_lock` (`tests.rs`) still pass unmodified: a warm open still refuses a cached executable whose bytes changed, purely from the hash -> `cargo test -p kuru-memory corrupt_cache_is_rejected_before_execution_and_links_are_never_adopted warm_verification_does_not_wait_for_the_installation_lock`
- [ ] 3.3 Review `actual_warm_cache_verifies_concurrently_while_installation_lock_is_held` and `real_embedded_windows_engine_installs_offline_and_corrupt_cache_fails_before_execution` (`packages/kuru-memory/src/provision/native_tests.rs`) for stage or log-message text that claims warm opens retain "version probes"; update the `eprintln!` observation text at `actual_warm_cache_verifies_concurrently_while_installation_lock_is_held` to describe payload digests only, and confirm the corrupt-cache rejection still fires from the hash -> native test run + diff review
- [ ] 3.4 Add or confirm a deterministic assertion that the cold path (extraction + first activation) still reports `CheckingRuntimeVersion` and fails on a probe that reports the wrong version, so cold-path probing is proven, not just left alone -> existing cold-path coverage in `tests.rs`/`native_tests.rs`, extended if a gap is found
- [ ] 3.5 Run `mise run coverage` (or the package-scoped equivalent) and confirm the 90% workspace line-coverage gate holds with the removed code paths, with no new untested lines in `provision.rs` -> observed coverage report

## 4. Docs

- [ ] 4.1 Update `docs/memory.md` ("Subsequent runs read every cached engine and license byte for its pinned digest, revalidate the checked names and identities, run the exact-version probe and then reuse the engine") to describe the exact-version probe as an install-time property, not a per-open one -> diff review
- [ ] 4.2 Update `apps/kuru-docs/concepts/memory.md` ("Later runs read the complete cached payloads for their pinned digests, revalidate their checked names and identities, run the exact-version probe and reuse the cache") the same way -> diff review
- [ ] 4.3 Confirm `docs/configuration.md` and `apps/kuru-docs/reference/configuration.md` need no change (they describe `dolt_binary`'s own version requirement and cache corruption handling, not a per-open probe) -> read-through, no edit if confirmed
- [ ] 4.4 Run `mise run docs:check` -> observed pass

## 5. Spec sync

- [ ] 5.1 Confirm `openspec/specs/versioned-memory/spec.md` and `openspec/specs/embedded-runtime/spec.md` will be updated to the delta text in this change only through `cospec archive`, not by hand -> N/A at apply time, recorded here as a reminder
