# Tasks

## 1. Measure and decide

- [x] 1.1 Record the repository's Actions cache usage and every entry's key and size before the change (`gh api repos/replygirl/kuru/actions/cache/usage` and `.../actions/caches --paginate`), and verify the numbers are written to the Unit 7 notes and `docs/development.md`
- [x] 1.2 Age fresh 1k and 5k stores with the merged `measure:age-store` task, run `CALL DOLT_GC()` on each store, and verify by recording `du` before and after and the decision the 1 GB rule gives (uncached: 1.57 GiB after GC)

## 2. Implement the decision (uncached)

- [x] 2.1 Remove the fixture key step, restore, restore report and save from the `usage-scan-scaling` job in `.github/workflows/ci.yml`, age in-job on every run, remove the `usage-scan-fixture key` subcommand (the seal keeps its compiled key), and verify with the updated `release_workflow` shape test, the `usage_scan` unit tests and `mise run lint:tooling`
- [x] 2.2 Update the "Usage scan scaling check" section of `docs/development.md` with the decision and its numbers, and verify with `mise run docs:check`

## 3. Verification

- [x] 3.1 Run `format:check`, `lint`, `lint:tooling`, `typecheck`, the `usage_scan` unit tests and `docs:check`, and record the results
- [ ] 3.2 Record this PR's hosted `usage-scan-scaling` run and its wall clock (verification 1.1)
