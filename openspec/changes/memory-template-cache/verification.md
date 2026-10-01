# Verification

## 1. A template builds once per key and is byte-identical to a cold store's schema [critical]

- [ ] 1.1 @integration (agent) run `template_born_store_matches_cold_store` (T10) against the real pinned Dolt engine, comparing schema, receipts minus operation UUIDs, branch/record version sets and usage version to a separately built cold store -> parity holds and `template_shape.rs::BASE_COMMITS` is confirmed, not guessed
- [ ] 1.2 @unit (agent) `template_key_tracks_schema_engine_statements_and_format_only` (T4) -> each keyed input changes the key; supervisor bytes and an unrelated source file's text do not

## 2. A build never publishes on a failed assertion, and never leaks secrets or paths [critical]

- [ ] 2.1 @integration (agent) `non_table_objects_fail_the_template_shape` (T29) against the real engine: a view, trigger, procedure, or dolt_schemas/dolt_procedures/dolt_ignore entry created before the shape check refuses publication -> no `<key>/` written; also records which of those objects the pinned 2.3.5 actually reports as present vs. absent (settling spike item S11, unresolved in engine-contract-findings.md)
- [ ] 2.2 @integration (agent) `template_holds_data_only_without_build_path_secret_or_host_name` (T12) against a real built template -> byte scan finds none of the build path, either engine secret, or the host name, including inside `.dolt/stats`

## 3. Quarantine fires only on a verdict against bytes, never on an engine or I/O failure [critical]

- [ ] 3.1 @integration (agent) `quarantine_is_bound_to_the_judged_template` (T21) -> a template republished between the verdict and the exclusive lock is never condemned; the fresh template stays published
- [ ] 3.2 @integration (agent) copy-corruption and copy-I/O-error cases (T8's library half) against real file corruption and an injected I/O error -> digest/size mismatch quarantines; I/O error does not, and preserves the partial copy as a remnant
- [ ] 3.3 @unit (agent) every non-verdict failure path (engine crash, lost reply, deadline, auth failure, lock contention) reports Engine/Io, never TemplateVerdict, and leaves a previously published template untouched -> assertion in each injected-failure test case

## 4. No open path removes another key's template, quarantined directory, or any lock file [critical]

- [ ] 4.1 @integration (agent) `open_never_removes_other_keys_or_lock_files` (T14) -> a private root pre-populated with two other keys' published templates, a `.rejected-*` directory and lock files for all three keys is byte-identical after a build and a copy for other keys; no lock file removed

## 5. Concurrent template operations never wait and never race a build

- [ ] 5.1 @integration (agent) `concurrent_template_lock_acquisition_never_waits` (T5's library half) -> a second `ensure_in` call under a held exclusive lock returns at once as "no template available," without reading `.build-*`/`.stage-*`
- [ ] 5.2 @integration (agent) `warm_runtime_cache_builds_one_template_across_processes` (T17) -> child-process pattern shows exactly one build across processes sharing a cache

## 6. Fixture warm-up is proven on every OS and under coverage, with a deterministic guard [critical]

- [ ] 6.1 @e2e (agent) native CI run on all four OSes: the ordinary test task (depending on prefetch) shows the template warmed before the first fixture's fresh open on each OS -> no in-process template build recorded by the first fixture's engine_ledger counters
- [ ] 6.2 @integration (agent) coverage run: the instrumented process warms the template using the instrumented supervisor, with the template key excluding the supervisor -> build code runs instrumented and counts toward the 90% gate; no uninstrumented supervisor compiled first
- [ ] 6.3 @unit (agent) `unwarmed_fixture_open_fails_the_guard_before_any_start` (T23) -> fails at once, 0 engine starts in the ledger
- [ ] 6.4 @unit (agent) `fixture_open_that_builds_a_template_fails_the_guard` (T18) -> fixture teardown fails naming the counter, unless opted in
- [ ] 6.5 @unit (agent) `warm_blocking_refuses_inside_a_runtime` (T28) -> named error, no thread started, when called from inside a Tokio runtime
- [ ] 6.6 @integration (agent) `temporary_warms_before_instantiate` (T24), child process with a fresh private KURU_DOLT_CACHE -> first temporary() publishes the production template during warm-up; the test-template build() open records 2 starts and 0 in-open builds

## 7. Every fixture call site across kuru-memory, kuru-runtime and kuru-tui is warmed

- [ ] 7.1 @regression (agent) full test suite of all three packages, with the deterministic guard active, after switching every test_support::open_options call site to the warmed form -> no guard failure across any package's suite
- [ ] 7.2 @manual (agent) spawn-gate audit of every spawning() site in service.rs, facade.rs and operational_gc_tests.rs that encloses an open -> none runs an unwarmed first warm-up under a held gate; counts recorded against the roadmap design's prior count, drift named

## 8. No product behavior changes (nothing selects the template path yet)

- [ ] 8.1 @regression (agent) existing fresh_open_budget/fixture_deadline tests and every existing product-path test (facade, service, store) pass unchanged -> no start-count or deadline assertion in the existing suite changes value

## 9. Documentation states what lands here and what does not

- [ ] 9.1 @manual (human) read docs/development.md, docs/memory.md and apps/kuru-docs/concepts/memory.md passages added by this change -> confirm they describe the cache, the warm-up, the S7 byte-scan caveat, and explicitly that no product open uses the template until the next change
