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

- [ ] 5.1 Write the harness and fixture tests first for a `new-project` case (a second, different project created in the same scratch root, with its own stage partition and expected engine-start count) and verify they fail on stubs
- [ ] 5.2 Add `Case::NewProject` and run it after the three existing cases and their own retirement wait, with a retirement wait of its own: first-launch, then cold-existing, then warm-reopen, then retire, then new-project, then retire. This is append-only: first-launch, cold-existing and warm-reopen keep the same predecessor and gap they have today (so cold-existing still runs right after first-launch, page cache warm, per the non-goal that nothing about the existing cases' measurement changes), and new-project still sees a warm engine cache with nothing else in between. Extend `report::derive`'s milestone chain and `Counts` so `new-project`'s partition and engine-start count are reported like the other cases
- [ ] 5.3 Verify the tests pass and record a local smoke run showing 4 engine starts for `new-project` today (expected to drop to 2 once the template cache lands, per `store-creation-design-machine-cache-2026-09-29.md` section 11's P0 row)

## 6. Second readiness signal: `kuru-open-marker`

- [ ] 6.1 Write the harness and fixture tests first: a run ending on the `ready` marker, one ending on `waiting-ownership` then `ready`, one with only the legacy `Memory: ...` lines, and one with neither (a failed open) — and verify they fail on stubs
- [ ] 6.2 Parse `kuru-open-marker v1 <event> <monotonic_ns>` stderr lines as progress (never the recorded error line, never dropped from the kept `stderr`), feed `open-start`, `waiting-ownership` and `ready` into the milestone chain in place of the matching legacy line when present, and record in each JSONL record which signal (`marker` or `legacy`) ended the open; keep the harness's own stderr read-time as the interval end so old and new binaries stay comparable, keeping `monotonic_ns` as data only
- [ ] 6.3 Set `KURU_OPEN_MARKERS=1` in the explicit environment the harness passes to every launched command and document it
- [ ] 6.4 Extend `tests/fixtures/delivery.rs` to emit marker lines when `KURU_OPEN_MARKERS=1` is set (and legacy lines always, so both signals and their absence are exercised against a live process) and extend `tests/open_time.rs` to cover all four combinations from 6.1
- [ ] 6.5 Update `Record::opened` and `Report.failed_opens`'s doc comments ("Memory: ready." → either readiness signal) and verify unit and integration tests pass

## 7. Observer: skip the template cache's private working directories

- [ ] 7.1 Write a test asserting `templates/.build-*` and `templates/.stage-*` directories under the engine cache are recorded as present (their appearance/disappearance still timed from the parent listing, as Dolt's temporary directories already are) but never entered, while a store's own `*.staging-*` directory and an ordinary project stage remain fully visible — verify it fails on the current observer
- [ ] 7.2 Extend the observer's `unentered`/walk logic in `observe.rs` to skip those two patterns only when the parent segment is `templates` under the engine cache, and update its module doc comment
- [ ] 7.3 Verify the test passes and no existing observer or census test regresses

## 8. Documentation and full gate

- [ ] 8.1 Update `docs/development.md`'s open-time section: the fourth case and its expected engine starts, both readiness signals and `KURU_OPEN_MARKERS`, the template-cache directories the observer skips, and the schema version if it changes; verify `docs:check` passes
- [ ] 8.2 Re-run delivery tests, lint, lint:windows, typecheck, format:check, lint:tooling, docs:check and `cospec validate --strict`; record a local smoke run of all four cases and record results in verification.md
