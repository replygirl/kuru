# Tasks

## 1. Driver and tasks

- [x] 1.1 Add `test_support::usage_scan` with the fixture key (compiled constants), `create`, `seal` and the measurement driver, plus the `usage-scan-fixture` and `measure-usage-scan` subcommands, and verify with the unit tests (key, timeline parsing, bounds and floor, failure text) and the real-engine row-count test
- [x] 1.2 Add `--profile` to `measure:age-store` and the `measure:usage-scan:fixture` and `measure:usage-scan` tasks (only the latter sets `KURU_OPEN_TIMELINE=1`), and verify by running them locally

## 2. Workflow and gate

- [x] 2.1 Add the `usage-scan-scaling` job to `ci.yml` with exact-key restore, main-only save, in-job ageing on a miss and `--assert`, add it to `ci-gate`'s `needs`, and verify with the updated `release_workflow` tests and the workflow validator

## 3. Documentation

- [x] 3.1 Document the job, the provisional bounds, the fallback decision rule, the fixture key, local use and timeline leftovers in `docs/development.md`, and verify with `mise run docs:check`

## 4. Verification

- [x] 4.1 Run `format:check`, `lint`, `//packages/kuru-memory:lint:windows`, `typecheck`, `lint:tooling` and the kuru-memory and kuru-delivery tests covering the change, and record the results
- [x] 4.2 Run the driver locally on freshly aged 1k and 5k stores with `--assert` and record rows, ratio and timing
- [ ] 4.3 Record the first hosted run of `usage-scan-scaling` (verification 1.1 and 1.2)
