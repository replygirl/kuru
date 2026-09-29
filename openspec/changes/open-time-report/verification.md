# Verification

## 1. The harness measures a real release binary from outside and records every run [critical]

- [x] 1.1 @integration (agent) run `measure:open-time` on macOS arm64 against a locally built release `kuru` (origin/main 31d8edef, sha256 6a63a3d0…), 10 iterations -> exit 0, 30 records, 0 failed opens; medians first-launch 6312 ms (6086–6530), cold-existing 635 ms (628–737), warm-reopen 14 ms (13–15); load before and after in every record, 1-minute load 3.2–17.2 during the series (another agent was building), so these are a smoke test of the harness, not the measurement
- [x] 1.2 @integration (agent) read the same run's first-launch and warm-reopen records -> every first launch shows extraction (median 1581 ms), `dolt version` (841 ms), exactly 4 engines (3 staging, 1 active) with lifetime, spawn-to-endpoint and served time each, the stage rename (89 ms) and the service endpoint; every warm reopen is `attached` with 0 engine starts. An earlier series (kept as local-run-v2-name-cache) miscounted engines seen between fork and exec; fixed in 2ceba056 and rerun
- [x] 1.3 @regression (agent) `tests/open_time.rs` drives the harness end to end against the delivery fixture standing in for `kuru` (live subprocess, streamed stderr) -> 4 passed: records and summary written with no scratch path; a failing open is recorded with exit code 1 and its error line and the series completes; a missing binary is an infrastructure error; a live process under the root (Unix) or a published endpoint makes the bounded retirement wait fail. These were written after the driver, so they have no red run
- [x] 1.4 @regression (agent) unit tests for statistics, role classification, path normalisation, stage derivation and summary rendering -> on `unimplemented!` stubs of the report module 5 failed (statistics, three derivations, summary) and 3 passed (classification and normalisation were implemented with their tests); after implementation 8 passed

## 2. The shipping build and the merge gate are unaffected

- [x] 2.1 @regression (agent) `cargo tree -p kuru -e normal,build` for aarch64-apple-darwin, x86_64/aarch64 Linux and x86_64/aarch64 Windows before and after the dependency -> identical (430/500/498/435/432 lines); Cargo.lock gains 94 lines and removes none
- [x] 2.2 @regression (agent) `lint:tooling` (actionlint and the repository workflow check) -> exit 0; `open-time` is absent from `ci-gate.needs`, has job-level `continue-on-error: true`; the upload step has step-level `continue-on-error: true`; no workflow gains `workflow_dispatch` and no workflow file is added

## 3. The CI job runs on its real runners [critical]

- [~] 3.1 @runtime (human) the draft PR's CI run: `open-time` on GitHub-hosted ubuntu-latest, macos-latest, windows-latest and windows-11-arm downloads its OS's install-job binary, runs 10 iterations and uploads `ci-open-time-<os>` records; the job summary shows the table; `ci-gate` does not wait for it -> defer: needs the draft PR pushed by the lead; this change may not push

## 4. Repository checks

- [x] 4.1 @regression (agent) `//packages/kuru-delivery:test`, `lint`, `lint:windows`, `typecheck`, `format:check`, `lint:tooling`, `docs:check`, `cospec:managed:check`, `cospec validate --strict` on macOS arm64 -> all exit 0 (delivery: 356 passed, 1 ignored, the existing opt-in previous-release update); mise.lock unchanged. The 90% workspace line gate was not measured locally
