# Verification

Local evidence: macOS 27.0 arm64 (shared, loaded host), debug profile, pinned
Dolt 2.3.5; the full `mise run //packages/kuru-memory:test` task (453 lib
tests passed, 4 ignored) and the named tests below. Linux, Windows and
coverage evidence comes only from CI on the pull request; none is claimed here.

## 1. A template builds once per key and is byte-identical to a cold store's schema [critical]

- [x] 1.1 @integration (agent) run `template_born_store_matches_cold_store` (T10) against the real pinned Dolt engine, comparing schema, receipts minus operation UUIDs, branch/record version sets and usage version to a separately built cold store -> parity holds and `template_shape.rs::BASE_COMMITS` is confirmed, not guessed Observed (local, macOS arm64, Dolt 2.3.5): passed; cold store history main 8 / usage 5, so BASE_COMMITS = 2 confirmed; template-born 9 / 6.
- [x] 1.2 @unit (agent) `template_key_tracks_schema_engine_statements_and_format_only` (T4) -> each keyed input changes the key; supervisor bytes and an unrelated source file's text do not Observed: passed.

## 2. A build never publishes on a failed assertion, and never leaks secrets or paths [critical]

- [x] 2.1 @integration (agent) `non_table_objects_fail_the_template_shape` (T29) against the real engine: a view, trigger, procedure, or dolt_schemas/dolt_procedures/dolt_ignore entry created before the shape check refuses publication -> no `<key>/` written; also records which of those objects the pinned 2.3.5 actually reports as present vs. absent (settling spike item S11, unresolved in engine-contract-findings.md) Observed: passed; refusal reads `branch "main" has uncommitted changes`, nothing published. S11 on 2.3.5: all six object reads answer zero on a store that never held the object; a view and a trigger also count in `dolt_schemas`, a procedure in `dolt_procedures`, an ignore rule in `dolt_ignore`; absent and empty read alike.
- [x] 2.2 @integration (agent) `template_holds_data_only_without_build_path_secret_or_host_name` (T12) against a real built template -> byte scan finds none of the build path, either engine secret, or the host name, including inside `.dolt/stats` Observed: passed as part of `template_builds_once_with_data_only_and_is_reused_without_build`; no hit for the build path, both secrets (whole and 16-character fragments) or the host name.

## 3. Quarantine fires only on a verdict against bytes, never on an engine or I/O failure [critical]

- [x] 3.1 @integration (agent) `quarantine_is_bound_to_the_judged_template` (T21) -> a template republished between the verdict and the exclusive lock is never condemned; the fresh template stays published Observed: passed.
- [x] 3.2 @integration (agent) copy-corruption and copy-I/O-error cases (T8's library half) against real file corruption and an injected I/O error -> digest/size mismatch quarantines; I/O error does not, and preserves the partial copy as a remnant Observed: `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines` passed (changed byte, extra byte, extra hard link and symlink on Unix quarantine; injected read error does not).
- [x] 3.3 @unit (agent) every non-verdict failure path (engine crash, lost reply, deadline, auth failure, lock contention) reports Engine/Io, never TemplateVerdict, and leaves a previously published template untouched -> assertion in each injected-failure test case Observed: `only_a_template_verdict_is_a_verdict`, the lock-fault test and the I/O cases passed; templates kept their directory identity.

## 4. No open path removes another key's template, quarantined directory, or any lock file [critical]

- [x] 4.1 @integration (agent) `open_never_removes_other_keys_or_lock_files` (T14) -> a private root pre-populated with two other keys' published templates, a `.rejected-*` directory and lock files for all three keys is byte-identical after a build and a copy for other keys; no lock file removed Observed: passed.

## 5. Concurrent template operations never wait and never race a build

- [x] 5.1 @integration (agent) `concurrent_template_lock_acquisition_never_waits` (T5's library half) -> a second `ensure_in` call under a held exclusive lock returns at once as "no template available," without reading `.build-*`/`.stage-*` Observed: passed; both calls returned `Busy` well inside 10 s.
- [x] 5.2 @integration (agent) `warm_runtime_cache_builds_one_template_across_processes` (T17) -> child-process pattern shows exactly one build across processes sharing a cache Observed: passed; exactly one `Built` and one `Published` across the two child processes.

## 6. Fixture warm-up is proven on every OS and under coverage, with a deterministic guard [critical]

- [ ] 6.1 @e2e (agent) native CI run on all four OSes: the ordinary test task (depending on prefetch) shows the template warmed before the first fixture's fresh open on each OS -> no in-process template build recorded by the first fixture's engine_ledger counters
- [ ] 6.2 @integration (agent) coverage run: the instrumented process warms the template using the instrumented supervisor, with the template key excluding the supervisor -> build code runs instrumented and counts toward the 90% gate; no uninstrumented supervisor compiled first
- [x] 6.3 @unit (agent) `unwarmed_fixture_open_fails_the_guard_before_any_start` (T23) -> fails at once, 0 engine starts in the ledger Observed: passed.
- [x] 6.4 @unit (agent) `fixture_open_that_builds_a_template_fails_the_guard` (T18) -> fixture teardown fails naming the counter, unless opted in Observed: `fixture_open_that_builds_a_template_fails_the_guard` passed. No open reaches `create_in` yet, so the event is recorded directly; the open-path trigger is P4b-ii.
- [x] 6.5 @unit (agent) `warm_blocking_refuses_inside_a_runtime` (T28) -> named error, no thread started, when called from inside a Tokio runtime Observed: passed.
- [x] 6.6 @integration (agent) `temporary_warms_before_instantiate` (T24), child process with a fresh private KURU_DOLT_CACHE -> first temporary() publishes the production template during warm-up; the test-template build() open records 2 starts and 0 in-open builds Observed (as amended for P4b-i): the child's first `temporary()` warmed and published the production template and no template event was charged; the test template's `build()` open is still the 4-start cold path until P4b-ii, so the "2 starts" clause is not asserted here.

## 7. Every fixture call site across kuru-memory, kuru-runtime and kuru-tui is warmed

- [x] 7.1 @regression (agent) full test suite of all three packages, with the deterministic guard active, after switching every test_support::open_options call site to the warmed form -> no guard failure across any package's suite Observed: kuru-memory 453 lib tests, kuru-runtime 220 and every kuru-tui target passed with the guard active (see tasks 14.2); no guard failure.
- [x] 7.2 @manual (agent) spawn-gate audit of every spawning() site in service.rs, facade.rs and operational_gc_tests.rs that encloses an open -> none runs an unwarmed first warm-up under a held gate; counts recorded against the roadmap design's prior count, drift named Observed: audit recorded in tasks 11.1; the scan test `no_spawn_guard_encloses_a_test_cache_warm_up` passes and failed on a deliberate violation.

## 8. No product behavior changes (nothing selects the template path yet)

- [x] 8.1 @regression (agent) existing fresh_open_budget/fixture_deadline tests and every existing product-path test (facade, service, store) pass unchanged -> no start-count or deadline assertion in the existing suite changes value Observed: the existing budget, deadline and product-path tests passed unchanged in the kuru-memory, kuru-runtime and kuru-tui runs; no start-count or deadline value changed.

## 9. Documentation states what lands here and what does not

- [ ] 9.1 @manual (human) read docs/development.md, docs/memory.md and apps/kuru-docs/concepts/memory.md passages added by this change -> confirm they describe the cache, the warm-up, the S7 byte-scan caveat, and explicitly that no product open uses the template until the next change
