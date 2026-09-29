# Verification

## 1. The harness measures a real release binary from outside and records every run [critical]

- [ ] 1.1 @integration (agent) run `measure:open-time` on macOS arm64 against a locally built release `kuru` (origin/main), 10 iterations -> 30 JSON records (10 per case), each with wall-clock to the `Memory: ready.` line and to exit, the calibration probe, load before and after, a process census, and observed engine starts; a summary table with every sample, median, minimum, maximum and spread. Local numbers are labelled a smoke test and possibly distorted by the recorded load
- [ ] 1.2 @integration (agent) read the same run's first-launch and warm-reopen records -> they show the stages the report needs from outside: engine extraction and activation, the `dolt version` probe, four engine lifetimes (three staging, one active), the stage rename and the service endpoint; the warm-reopen records show whether the client attached or spawned an owner
- [ ] 1.3 @regression (agent) `tests/open_time.rs` drives the harness end to end against the delivery fixture standing in for `kuru` (live subprocess, streamed stderr) -> records and summary are written; a failing open is recorded with its exit status and the harness still exits 0; an unretired process stops the series as an infrastructure failure
- [ ] 1.4 @regression (agent) unit tests for statistics, role classification, path normalisation, stage derivation and summary rendering, written before the implementation -> red on stubs, green after

## 2. The shipping build and the merge gate are unaffected

- [ ] 2.1 @regression (agent) `cargo tree -p kuru -e normal,build` for aarch64-apple-darwin, x86_64/aarch64 Linux and x86_64/aarch64 Windows before and after the dependency -> identical; Cargo.lock only gains entries
- [ ] 2.2 @regression (agent) `lint:tooling` (actionlint and the repository workflow check) -> passes with the new job and upload step; the job is absent from `ci-gate.needs`, has job-level `continue-on-error: true`, and no workflow gains `workflow_dispatch`

## 3. The CI job runs on its real runners [critical]

- [ ] 3.1 @runtime (human) the draft PR's CI run: `open-time` on GitHub-hosted ubuntu-latest, macos-latest, windows-latest and windows-11-arm downloads its OS's install-job binary, runs 10 iterations and uploads `ci-open-time-<os>` records; the job summary shows the table; `ci-gate` does not wait for it -> observed only after the draft PR is pushed by the lead

## 4. Repository checks

- [ ] 4.1 @regression (agent) `//packages/kuru-delivery:test`, `lint`, `lint:windows`, `typecheck`, `format:check`, `lint:tooling`, `docs:check`, `cospec validate --strict` on macOS arm64 -> recorded as observed
