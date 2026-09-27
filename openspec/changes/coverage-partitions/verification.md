## 1. Every listed test runs exactly once across an OS's partitions [critical]

- [ ] 1.1 @unit (agent) `mise run //packages/kuru-delivery:test` partition tests -> `parse_libtest_list` accepts only `<name>: test` lines and rejects `: bench`, duplicates, control characters and malformed lines; `assign` is deterministic across calls and independent of the executable path, every name lands in exactly one of N partitions for N in 1..=16, and the union over partitions equals the list
- [ ] 1.2 @unit (agent) chunking tests -> each chunk's Windows command-line units stay under 30,000 including the quoted program path; the chunks' disjoint union equals the assigned list in order; a single name longer than the budget fails; empty assignments produce `omit` without an exact run
- [ ] 1.3 @unit (agent) dispatcher tests through the fake `Launcher` -> the list run precedes the chunks, a `running N tests` count different from the chunk size fails with exit 0, the deadline is re-checked between chunks, and each ledger record carries the list hash, assigned hash, invocations, timestamps and profile counts
- [ ] 1.4 @integration (agent) the live libtest test runs the delivery test executable itself with `--list --format terse` and `--exact` over two no-op probe tests -> the parsed list contains both probes and the exact run announces `running 2 tests`; on Windows a budget-sized chunk spawns successfully
- [ ] 1.5 @runtime (agent) GitHub-hosted ubuntu-latest, macos-latest and windows-latest on this PR's CI run -> 8 / 4 / 8 `Coverage partition` jobs succeed; each OS's partition plans list identical per-executable test lists and the merge reports disjoint, complete assignments (log line per OS with executable and test totals)

## 2. The Ubuntu merge accepts only agreeing, complete receipts [critical]

- [ ] 2.1 @unit (agent) merge tests over temp evidence trees -> a missing partition index, an extra or unknown index, a foreign OS label, a receipt differing in source, tree, Cargo.lock, rustc, cargo, cargo-llvm-cov, profile environment, mode or inventory digest, a tampered plan, ledger or LCOV (hash mismatch), overlapping assignments and an incomplete union each fail with a named error and leave no report file
- [ ] 2.2 @unit (agent) layout tests -> both download layouts are accepted, each index takes its latest attempt no later than the run attempt, a diagnostics artifact name never matches the evidence pattern, and the flat single-artifact layout works for N = 1
- [ ] 2.3 @runtime (agent) GitHub-hosted runs on a throwaway draft commit that removes one partition's evidence upload on one OS -> that OS's `Coverage merge (<os>)` fails naming the missing partition and uploads no LCOV, while the other OSes' merges pass; the drill commit is reverted
- [ ] 2.4 @runtime (agent) the same hosted runs after "Re-run failed jobs" of one partition -> the merge refuses while a partition job failed and accepts the later successful attempt

## 3. Merged LCOV enforces 90% per OS and matches the rebuilt report [critical]

- [ ] 3.1 @unit (agent) LCOV tests -> `SF:` paths are normalized to root-relative forward-slash form (Windows drive case folded) and a source outside the root fails; differing SF sets, DA line sets or FN sets across partitions fail; DA/FNDA counts are summed and LF/LH/FNF/FNH recomputed; `BRDA` is refused; the gate passes at exactly 90.00% and fails just below
- [ ] 3.2 @runtime (agent) GitHub-hosted ubuntu-latest, macos-latest and windows-latest on this PR's CI run -> each `Coverage merge (<os>)` job runs on ubuntu-latest, passes at ≥ 90% lines for its OS and uploads `ci-coverage-<os>-attempt-<n>`; the per-OS totals are recorded next to #111's run 36270435157 figures
- [ ] 3.3 @equivalence (agent) by hand on one machine and SHA (macOS arm64 locally is sufficient): run all N instrumented partitions keeping each partition's target and raw profiles, export and DA-merge their N LCOVs, then build one fresh instrumented target, copy every partition's raw profiles into it and run `llvm-cov report --failure-mode any --lcov` plus its summary (the #111 collect recipe by hand) -> identical DA line sets per file and identical LF/LH totals between the two LCOVs, the merge's Σ LH / Σ LF equals cargo-llvm-cov's printed line percentage, and at least one executable has zero assigned tests in some partition

## 4. The seeded dependency cache is evidence-neutral and failure-free [critical]

- [ ] 4.1 @unit (agent) seed tests over temp trees -> only `debug/{deps,build,.fingerprint}` entries of non-workspace packages are imported; workspace crate outputs (hyphen and underscore forms), `*.profraw`, symlinks, `kuru-shard-state`, `kuru-test-supervisors`, other seed-root entries and malformed names are refused and counted; an absent seed directory is a recorded miss; export writes exactly what import accepts and refuses an existing destination
- [ ] 4.2 @runtime (agent) the first PR run after a `main` run that saved the seed, on each OS -> the seed restore reports a hit, `job-ledger.json` shows imported entries and `dependency_units_rebuilt` (target 0), and receipts' `inventory_sha256` equals a cold run's on the same SHA
- [ ] 4.3 @runtime (agent) a run with the seed key absent (new key prefix on a draft commit) -> every partition records `seed: absent`/`miss`, rebuilds dependencies and passes with the same receipts

## 5. Uninstrumented partitions keep the arm64 memory suite whole [critical]

- [ ] 5.1 @unit (agent) orchestrator tests through the fake `Host` -> `--uninstrumented` requires a package scope, builds with `-p <scope>`, runs `prefetch` with `CARGO_TARGET_DIR` set to the fresh target before `cargo test`, never calls `mise bin-paths` or `show-env`, writes a receipt with `mode: uninstrumented` and no LCOV, and the merge refuses a receipt of the other mode
- [ ] 5.2 @runtime (agent) GitHub-hosted ubuntu-24.04-arm on this PR's CI run -> three `native-memory` partitions and the uninstrumented merge pass; the union of assigned tests equals the full `kuru-memory` list; `native-build` still builds, packages and passes the embedded-runtime check without the memory suite; `ci-gate` requires both

## 6. Workflows, tables and ledgers stay pinned [critical]

- [ ] 6.1 @integration (agent) `release_workflow` tests in `mise run //packages/kuru-delivery:test` -> the partition matrix expression equals `PARTITIONS` for each OS, artifact names follow the partition/diagnostics scheme, the merge has one download step and `runs-on: ubuntu-latest`, the fixed target path, the seed restore everywhere and save only on `refs/heads/main` for partition 1, `KURU_COVERAGE_JOB_MINUTES` equals `timeout-minutes`, identical `CARGO_PROFILE_TEST_DEBUG` in every partition step, and no `coverage:collect`, `SHARDS` or five-download layout remains
- [ ] 6.2 @unit (agent) job-ledger tests -> phase timestamps are ordered, empty cache inputs record `unknown` without failing, and the merge summary lists every partition's cache state, compile and test seconds, profile count and rebuilt dependency units
- [ ] 6.3 @integration (agent) `mise run coverage` on macOS arm64 with the new modules -> the local 90% workspace gate passes and the new `coverage/*.rs` modules are each at or above the workspace threshold

## 7. Critical-path effect is measured, not assumed

- [ ] 7.1 @benchmark (agent) the first green hosted run with warm caches -> per-OS wall clock (first partition start to merge end), per-partition compile and test seconds, max/mean partition test wall, profile counts per partition and the run-level `ci-gate` time are recorded with run and job IDs against the predicted 10.9 / 13.7 / 15.3 min and arm64 ≈ 9-10.5 min
- [ ] 7.2 @benchmark (agent) the same measurements on an evicted-seed run -> the degraded paths are recorded against 12.9 / 16.0 / 19.6 min
- [ ] 7.3 @regression (agent) the #111 stall drill on one partition (throwaway commit, reverted) -> the partition that holds the stalled test fails at its inner deadline with a stall report in its diagnostics artifact
