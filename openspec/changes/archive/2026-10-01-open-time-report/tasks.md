# Tasks

## 1. Report-only open-time measurement

- [x] 1.1 Add `sysinfo` (system only) as an optional tooling dependency and verify the shipping `kuru` dependency tree is unchanged on every target
- [x] 1.2 Write the harness unit and fixture tests first and verify they fail on stubs
- [x] 1.3 Implement the `open-time` subcommand and `measure:open-time` task and verify the tests pass
- [x] 1.4 Publish the install job's release binary and add the report-only `open-time` job to ci.yml; verify `lint:tooling` passes and `ci-gate` does not need the job
- [x] 1.5 Run the harness locally against a release build of origin/main and record the numbers with load
- [x] 1.6 Document the job in docs/development.md and verify `docs:check` passes
- [x] 1.7 Run delivery test, lint, lint:windows, typecheck, format:check and cospec validate --strict and record the results

## 2. Review fixes

- [x] 2.1 Derive stages that partition the open from ordered milestones, report nesting spans as totals with their parts, and test the partition property on synthetic and recorded observations
- [x] 2.2 Stop the observer entering install stage, store staging and interrupted directories, close each listing before descending, correct the file-access claim, and add a control mode without file observation
- [x] 2.3 Stamp each tick at start and end, report the recorded bracket and mean tick cost as the error bound, exclude single-sample spans from lifetime lower bounds, repeat a longer CPU probe and report its median, and use one definition of a failed open
- [x] 2.4 Add a census before and after each run and a ramp mode without retirement waits
- [x] 2.5 Add the control, coarse and ramp series to the CI job; correct the docs, including the `ci-gate` critical-path statement and the packaged embedded-runtime test's build profile
- [x] 2.6 Rebase onto origin/main, run the harness tests, delivery tests, lint, lint:windows, typecheck, format:check, lint:tooling, docs:check and cospec validate --strict, and a local smoke run of the three cases and the control

## 3. Narrow CI measurement to ubuntu-latest

- [x] 3.1 Reduce the `open-time` job's matrix in ci.yml to `ubuntu-latest` only; keep it report-only (job-level continue-on-error, absent from ci-gate's needs)
- [x] 3.2 Condition native-tests.yml's release-binary upload step on `runner.os == 'Linux'` (the workflow's convention: step-level OS selection uses the runner, never the caller's `inputs.os` label), keeping step-level continue-on-error
- [x] 3.3 Remove every Windows section, table, claim and caveat from the open-time documentation (docs/development.md, this change's proposal/verification, the PR body); document local runs as the way to measure on a developer machine; add the second review's milestone-order caveat
- [x] 3.4 Verify lint, lint:windows, typecheck, format:check, lint:tooling, docs:check, delivery tests and cospec validate --strict all pass on the narrowed tree

## 4. Rebase for the store-creation acceptance work

- [x] 4.1 Fetch origin, rebase onto origin/main (b3ca1d02), resolve the `bundle_build` cache-key test's new `open_time` module classification (add it to `UNKEYED_MODULES`, alongside the other main.rs-only subcommand modules) and verify `//packages/kuru-delivery:test` passes clean twice in a row

## 5. `new-project` case: a warm-engine baseline for the template cache

- [x] 5.1 Write the harness and fixture tests first for a `new-project` case (a second, different project created in the same scratch root, with its own stage partition and expected engine-start count) and verify they fail on stubs
- [x] 5.2 Add `Case::NewProject` and run it after the three existing cases and their own retirement wait, with a retirement wait of its own: first-launch, then cold-existing, then warm-reopen, then retire, then new-project, then retire. This is append-only: first-launch, cold-existing and warm-reopen keep the same predecessor and gap they have today (so cold-existing still runs right after first-launch, page cache warm, per the non-goal that nothing about the existing cases' measurement changes), and new-project still sees a warm engine cache with nothing else in between. Name the stores already in `data/memory` when a `new-project` run starts `<prior-project>` in the observer's keys (every other case passes no prior hash, so its keys are unchanged), so `report::derive`'s existing milestone chain and `Counts` see the new store appear and report `new-project`'s partition and engine-start count like the other cases
- [x] 5.3 Verify the tests pass and record a local smoke run showing 4 engine starts for `new-project` today (expected to drop to 2 once the template cache lands, per `store-creation-design-machine-cache-2026-09-29.md` section 11's P0 row)

## 6. Second readiness signal: `kuru-open-marker`

- [x] 6.1 Write the harness and fixture tests first: a run ending on the `ready` marker, one ending on `waiting-ownership` then `ready`, one with only the legacy `Memory: ...` lines, and one with neither (a failed open) — and verify they fail on stubs
- [x] 6.2 Parse `kuru-open-marker v1 <event> <monotonic_ns>` stderr lines as progress (never the recorded error line, never dropped from the kept `stderr`; the five agreed plain open sentences likewise), feed `open-start` (a new `open start` milestone, marker binaries only), `waiting-ownership` and `ready` into the milestone chain in place of the matching legacy line when present, name the final milestone `ready` for either signal so old and new binaries' partitions share row names, record which signal (`marker` or `legacy`) ended the open in `stages.ready_signal` and the parsed markers in `stages.markers`, count the signal per case in the summary, and bump the schema to `kuru.open-time.v3`; keep the harness's own stderr read-time as the interval end so old and new binaries stay comparable, keeping `monotonic_ns` as data only
- [x] 6.3 Set `KURU_OPEN_MARKERS=1` in the explicit environment the harness passes to every launched command and document it
- [x] 6.4 Extend `tests/fixtures/delivery.rs` to emit marker lines only when `KURU_OPEN_MARKERS=1` reaches it, with prompts selecting both signals (the default), legacy lines only (a binary from before the marker change), markers and the plain sentence only (a binary from after it) and a failed open with either, and to name its store by the hash of its `-C` directory; extend `tests/open_time.rs` to cover each against a live process
- [x] 6.5 Update `Record::opened` and `Report.failed_opens`'s doc comments ("Memory: ready." → either readiness signal) and verify unit and integration tests pass

## 7. Observer: skip the template cache's private working directories

- [x] 7.1 Write a test asserting `templates/.build-*` and `templates/.stage-*` directories under the engine cache are recorded from the parent listing (normalised to `.build-<tmp>` and `.stage-<tmp>`, their appearance and disappearance timed as an install stage's are) but never entered, while the same names outside the engine cache's `templates`, a published template's own files and a store's `*.staging-*` directory are observed as before — verify it fails on the current observer
- [x] 7.2 Extend the observer's `unentered`/walk logic in `observe.rs` to skip those two patterns only when the parent segment is `templates` under the engine cache, and update its module doc comment
- [x] 7.3 Verify the test passes and no existing observer or census test regresses

## 8. Documentation and full gate

- [x] 8.1 Update `docs/development.md`'s open-time section: the fourth case and its expected engine starts, both readiness signals and `KURU_OPEN_MARKERS`, the template-cache directories the observer skips, and the schema version if it changes; verify `docs:check` passes
- [x] 8.2 Re-run delivery tests, lint, lint:windows, typecheck, format:check, lint:tooling, docs:check and `cospec validate --strict`; record a local smoke run of all four cases and record results in verification.md

## 9. Phase 2: the gate

- [x] 9.1 Write the harness and fixture tests first for gate mode: one post-fix `testdata/` fixture set that passes the gate, and four failing sets (a count mismatch on any case, a new-project median over budget, a cold-existing median over budget, a warm-reopen `OwnerPath` mismatch) — verify each new test fails against the pre-change command
- [x] 9.2 Add `KURU_OPEN_TIME_`-prefixed gate environment variables: exact expected `Counts` per case (first-launch, new-project, cold-existing, and warm-reopen keyed on the record's `OwnerPath` — `attached` or `spawned-owner` carry separate expectations), `KURU_OPEN_TIME_BUDGET_NEW_PROJECT_MS` and `KURU_OPEN_TIME_BUDGET_COLD_EXISTING_MS` for the main series' median open-to-ready time per case. Absent gate variables keep the command exactly report-only
- [x] 9.3 Check counts first, across every case; only once every count matches, check the new-project median against its budget and then the cold-existing median against its budget. Any violation writes the records and a readable summary naming the case, the observed value, the expected value or budget, and the offending record(s), then exits non-zero
- [x] 9.4 Verify the new unit tests pass against the implementation and the existing report-only tests still pass unmodified
- [x] 9.5 `.github/workflows/ci.yml`: rename the `open-time` job (e.g. "Open-time gate (ubuntu-latest)"), remove its job-level `continue-on-error` and its `if: ${{ !cancelled() }}`, keep `needs: native-tests`, add it to `ci-gate.needs`, remove the control/coarse/ramp steps (main series only, `n=10`), and pass the gate environment variables with both budget constants in the job's `env`, each commented with the two derivation run ids and the rule: highest post-fix median over those two runs plus three times the largest within-run spread (max minus min), rounded up to a whole ms
- [x] 9.6 `.github/workflows/native-tests.yml`: remove `continue-on-error` from the ubuntu-latest release-binary upload step (Linux-only condition unchanged)
- [x] 9.7 Run two post-fix derivation runs of the main series (n=10) on this branch's own CI and record their run ids, per-case counts and per-case median/spread in verification.md; compute both budget constants from those two runs per the rule in 9.5 and fill them into ci.yml, replacing the `<ids>`/derivation placeholders
- [x] 9.8 Update `packages/kuru-delivery/mise.toml`'s task description/comment, `ci.yml`'s job comment, and `docs/development.md#open-time-report` together: describe the gate, the two budgets and their rule, how to re-derive a budget after a deliberate startup change, that control/coarse/ramp remain local-only modes, and that macOS is local-only and Windows open time is unmeasured; verify `docs:check` passes
- [x] 9.9 Verify `lint:tooling` shows `open-time` present in `ci-gate.needs`, no job-level or step-level `continue-on-error` on the gate job or the upload step, and no workflow gains `workflow_dispatch`
- [x] 9.10 Run delivery tests, lint, lint:windows, typecheck, format:check, lint:tooling, docs:check and `cospec validate --strict` on the final tree; record results in verification.md
- [x] 9.12 Address the independent review of pull request #133: pin the median budget's equality boundary with whole-ms medians (budget equal to the median passes, one ms less fails) for both gated cases; restore the `if-no-files-found: warn` line the rebase dropped from `ci.yml`'s unrelated usage-scan-scaling upload step; rename `docs/development.md`'s section heading to "Open-time gate" (anchor `#open-time-gate`) and update its links in `docs/development.md`, the `measure:open-time` task comment and the `ci.yml` job comment; rerun the checks in 9.10 and record the results in verification.md
- [x] 9.11 Push and record this pull request's own `open-time` gate run as the acceptance evidence that the counts and both budgets hold on main's current startup path; archive the change once that evidence is recorded (recorded in verification 6.7: run 36960479656, head eced4faf)
- [x] 9.13 Apply the lead's ruling on runner noise (vendor-side; a bounded retry at the measurement is the allowed mitigation, a real regression misses twice): add a pure `gate::Decision::decide(first, second)` over at most two evaluated series, with deterministic tests on the recorded fixtures for each branch (first passes -> pass, no second; first count violation or failed open -> fail, no second requested; first median miss -> remeasure, then second passes -> pass; both miss a median -> fail; first median miss and second count violation or failed open -> fail); in gate mode only, when the decision requests it, measure one more full main series in the same command (same n and cases, fresh scratch root), writing its records to `<output>/retry/records.jsonl` and both series' summaries, titled by series, plus a line stating which series decided to `summary.md` and `GITHUB_STEP_SUMMARY`; document the rule beside the budgets in `ci.yml`'s job comment (retry for median budgets only, never for counts; re-derivation fixes a persistent miss; `timeout-minutes: 60` still covers two series plus compile), in `docs/development.md#open-time-gate` and in the `measure:open-time` task comment; no third series, no change to n, the cases, the budgets or the derivation rule; run the delivery checks and record the results in verification 6.10
- [x] 9.14 Address the independent review of head 0910c326 (verdict ACCEPT, four should-fix items) after rebasing onto main 845d7cde: when `<output>/retry` or the second scratch root cannot be created, keep no scratch root and say so, rather than keeping the first series' root as the stopped series'; pin an unexpected owner path and a missing case in the first series as `Decision::Failed`, decided by series 1, never measured again; align the remeasure wording in the `ci.yml` job comment, the `gate` module comment and the remeasure decision line to "fails only if that series fails a check too"; wrap the `docs/development.md` line a rebase joined; rerun the delivery checks and record the results in verification 6.11
