# Verification

## 1. The gating job runs on its real runner and asserts the provisional bounds [critical]

- [ ] 1.1 @runtime (agent) GitHub-hosted ubuntu-latest, this PR's CI run, job `usage-scan-scaling` -> the job builds the release tooling, ages both stores in-job (cache miss: PR runs never save), runs 1 warm-up and 5 gated owner opens per size, prints the per-size scan rows, the ratio and the bounds labelled provisional, passes, and `ci-gate` lists it among its needs
- [ ] 1.2 @runtime (agent) the same run, cold in-job ageing -> the Ubuntu `elapsed_ms` for 1k and 5k is recorded, and the decision rule (switch to a bulk-seeded fixture if cold 5k ageing exceeds 20 min) gets a recorded verdict
- [~] 1.3 @runtime (agent) the first exact-key restore on ubuntu-latest records restore seconds and the hit against the 2-minute rule -> defer: only `main` saves the fixture, so the first hit happens on a run after this change merges; it is recorded in the Unit 7 notes then

## 2. The driver and key are deterministic and fail closed

- [ ] 2.1 @unit (agent) run the `test_support::usage_scan` unit tests -> the key is stable for equal inputs, changes when any single input changes and has the `usage-scan-fixture-v1-<64 hex>` shape; timeline parsing accepts a recorded timeline and refuses a wrong format or version, a missing or repeated event, decreasing offsets, dropped stamps and a null row count; the evaluation passes linear growth, fails quadratic growth, applies the 100 ms floor, fails the ceiling and a row-count mismatch, and the failure text names the bound and both numbers
- [ ] 2.2 @integration (agent) run the row-count test against real Dolt with spawned owners -> two tiny stores created and aged through the driver measure one gated sample each, and each sample's timeline row count equals `Counts::expected` for its plan
- [ ] 2.3 @integration (agent) run the driver once on macOS against freshly aged 1k and 5k stores (release build of HEAD) with `--assert` -> the assertions pass on main's linear scan; rows, ratio and timing are recorded

## 3. Static workflow checks

- [ ] 3.1 @integration (agent) run `mise run //packages/kuru-delivery:test` (release_workflow, repo_validation) and `mise run lint:tooling` -> the `ci-gate` needs string includes `usage-scan-scaling`, the new job's shape test passes (needs bundle-inputs, offline bundle inputs, 45-minute timeout, exact-key restore without `restore-keys`, main-only save, `--assert`), and the workflow validator accepts the file
