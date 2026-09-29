# Tasks

## 1. Report-only open-time measurement

- [x] 1.1 Add `sysinfo` (system only) as an optional tooling dependency and verify the shipping `kuru` dependency tree is unchanged on every target
- [x] 1.2 Write the harness unit and fixture tests first and verify they fail on stubs
- [x] 1.3 Implement the `open-time` subcommand and `measure:open-time` task and verify the tests pass
- [x] 1.4 Publish the install job's release binary and add the report-only `open-time` job to ci.yml; verify `lint:tooling` passes and `ci-gate` does not need the job
- [ ] 1.5 Run the harness locally against a release build of origin/main and record the numbers with load
- [ ] 1.6 Document the job in docs/development.md and verify `docs:check` passes
- [ ] 1.7 Run delivery test, lint, lint:windows, typecheck, format:check and cospec validate --strict and record the results
