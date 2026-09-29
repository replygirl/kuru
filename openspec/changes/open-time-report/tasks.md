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
