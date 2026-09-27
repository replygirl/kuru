## 1. T1 Receipts and partition assignment

- [x] 1.1 Add `coverage/partition.rs` with `ArtifactKey`, `parse_libtest_list`, `assign` (`sha256-artifact-test-v1`) and the canonical list hash, and verify with the partition unit tests of verification 1.1
- [x] 1.2 Bump `SCHEMA` to 2; add `PartitionScheme`, `ExecutableTests`, `PartitionPlan`, the schema-2 `Receipt` (profile summary, optional LCOV receipt, `profile_env_sha256`, `tests_sha256`) and `WORKSPACE_PACKAGES`/`PARTITIONS`/empty `EXCLUDED_ARTIFACTS`; remove `Selection` and `SHARDS`; widen `artifact_os_label` from `[a-z0-9-]+` to also admit `.` so `ubuntu-24.04-arm` (and PR6b's labels) are valid while the `…-partition-<k>-attempt-<n>` name parser stays unambiguous; verify v1 files are refused by name and `workspace_packages()` still covers the eight packages in `mise run //packages/kuru-delivery:test`
- [x] 1.3 Write `partition-plan.json` from the inventory and recorded lists, recompute assignments in `validate_run_ledger`, and handle `exclude` records with reasons; verify a stale exclusion and a tampered plan fail in unit tests

## 2. T2 `--list`/`--exact` dispatch with chunking

- [x] 2.1 Introduce the `Launcher` trait with a `SystemLauncher` wrapping today's Unix group and Windows native supervision unchanged, and route `dispatch_test` through `dispatch_with`; verify the existing supervision, stall and signal tests still pass
- [x] 2.2 Implement the list run, `windows_command_line_units`, 30,000-unit chunking, per-chunk deadline checks, the `running N tests` equality check and the expanded `RunnerRecord`; verify with verification 1.2 and 1.3
- [x] 2.3 Add the live libtest probe tests (own test executable, two no-op probes; Windows budget-sized chunk); verify verification 1.4 on macOS locally and on hosted Windows (CI run 36290089575, Windows partition 1, job 108538385630)

## 3. T3 Ubuntu merge with receipt agreement

- [x] 3.1 Add the partition LCOV export to the instrumented shard (`llvm-cov report --failure-mode any --lcov`, profile limits checked first) and `coverage/lcov.rs` normalization of `SF:` paths; verify with verification 3.1 normalization cases
- [x] 3.2 Add `coverage/merge.rs` (artifact layout, latest attempt per index, exact entries and hashes, field-by-field agreement, OS-label/target consistency, disjoint and complete lists, ledger re-validation) and the LCOV union and 90% gate written through a temp file; verify with verification 2.1, 2.2 and 3.1
- [x] 3.3 Replace `coverage collect` with `coverage merge` in `main.rs`, `orchestrate.rs` and `mise.toml` (`coverage:merge` without bundle depends); verify the orchestrator fake-host tests for merge sequencing and that the task builds and runs without a prepared bundle

## 4. T4 Seeded dependency cache

- [x] 4.1 Add `coverage/seed.rs` allow-list import and export with refusal counts, and call import in `prepare` after `cargo metadata` and before the first build; verify with verification 4.1 and a fake-host ordering assertion
- [x] 4.2 Use the fixed `KURU_COVERAGE_TARGET` path and add export on `main` partition 1 after the receipt; verify the orchestrator refuses an existing target and an existing export destination
- [x] 4.3 Degrade every seed read, create or move failure to a counted `io` refusal that removes any partial copy, so an unreadable or corrupt seed only slows the build; verify with `seed::tests::unreadable_or_unmovable_seed_entries_degrade_to_refusals`

## 5. T5 Uninstrumented mode and arm64 wiring

- [x] 5.1 Add `coverage shard --uninstrumented` (scope-bound inventory, no `show-env`, prefetch into the fresh target, `mode: uninstrumented`) and the `test:partition` task with `KURU_TEST_SUPERVISOR_PREPARED=1`; verify with verification 5.1
- [x] 5.2 Move the arm64 `kuru-memory` suite from `native-build` into a `native-memory` partition matrix (3) plus an uninstrumented Ubuntu merge in `ci.yml`, and add both to `ci-gate`; leave `bundle:verify-native-build` unchanged per design D9; verify with verification 5.2
- [x] 5.3 Require `KURU_COVERAGE_PACKAGES` for an uninstrumented merge and refuse any receipt whose scope differs from it; pass `kuru-memory` to the arm64 merge and pin it in `release_workflow.rs`; verify with `merge::tests::uninstrumented_partitions_must_build_the_configured_scope` and the orchestrator input tests

## 6. T6 Workflows and shard counts

- [x] 6.1 Rewrite `native-tests.yml`: partition matrix from `PARTITIONS` (8/4/8), fixed target, seed restore/save steps pinned by SHA, cache-hit env for the ledger, evidence and diagnostics artifact names, the Ubuntu `Coverage merge (<os>)` job with one download step, `native-gate` needs; verify with verification 6.1
- [x] 6.2 Update `release_workflow.rs` pins for every workflow change and record the wave arithmetic for the PR body from design D7; verify `mise run //packages/kuru-delivery:test` and `mise run lint:tooling` pass

## 7. T7 Ledger and cache visibility

- [x] 7.1 Add `coverage/ledger.rs` job ledger (phases, helper/seed cache state, imported/refused entries, rebuilt dependency and workspace units from `compiler-artifact` `fresh` flags, profile totals, per-executable timings) and the merge's per-partition table and `merge-summary.json`; verify with verification 6.2

## 8. T8 Documentation

- [x] 8.1 Update `docs/development.md`: partition topology and counts, the agreement rule stated as "N independent shard builds agreeing rather than a separate rebuild", per-OS 90% gates on the merged LCOV, hand-run partition and merge instructions, the seed cache and its eviction behaviour, the uninstrumented arm64 memory partitions and the ledgers; re-read `AGENTS.md` and record whether any line changes; verify `mise run docs:check` passes. Recorded: the page also states that the merged gate counts unique instrumented lines (union of `DA` records) and why cargo-llvm-cov's summary reads about 0.7 points lower; the page now leads with the fact that the merged gate's metric differs from `--fail-under-lines 90`, names both figures per OS, states which metric each gate enforces, and says the reconciliation awaits the lead (design D4a). `AGENTS.md` is unchanged pending that decision

## 9. M Measurement and hosted acceptance

- [x] 9.1 Run `mise run //packages/kuru-delivery:test`, `mise run lint`, `mise run typecheck`, `mise run lint:tooling` and `mise run coverage` locally before pushing, and record verification 6.3 (all exited 0 on macOS arm64 at de9bae0e; coverage 94.33% cargo-llvm-cov summary, 95.01% unique lines)
- [x] 9.2 Before pushing, diff `feat/windows-arm64`'s `coverage.rs` against #111 and reconcile the `PARTITIONS`/`EXCLUDED_ARTIFACTS` tables; record the result in blocking-changes.md coordination notes
- [ ] 9.3 Run the equivalence drill of verification 3.3 by hand (all partitions on one machine, rebuilt profile-merge report versus DA-merge, and the merge total versus cargo-llvm-cov's summary) and record it
- [ ] 9.4 Record the hosted evidence of verification 1.5, 2.3, 2.4, 3.2, 4.2, 4.3, 5.2, 7.1, 7.2 and 7.3 with run and job IDs, naming any unrun check and why
