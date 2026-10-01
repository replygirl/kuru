# Verification

## 1. The job runs on its real runner under the cache budget [critical]

- [~] 1.1 @runtime (agent) GitHub-hosted ubuntu-latest, this PR's CI run, job `usage-scan-scaling` -> defer: the PR's run happens after this archive; its result and wall clock are recorded in the PR and the Unit 7 notes. Reference, observed 2026-10-01 on main's uncached run 36919301194 (job 110561340867, head d17dfe40): success in 13 min 40 s, release build and key 171 s, ageing 7 min 41 s (1k 69,760 ms, 5k 380,384 ms), save 5 s, measure 9 s

## 2. The decision rests on measured sizes

- [x] 2.1 @integration (agent) age fresh 1k and 5k stores with the merged driver, run `CALL DOLT_GC()` on each, and measure `du` before and after -> the post-GC total is compared with the 1 GB budget and the branch follows the rule Observed 2026-10-01, macOS arm64, release build of d17dfe40, root under `$TMPDIR`. Ageing took 87,167 ms (1k) and 439,333 ms (5k). `CALL DOLT_GC()` ran through the pinned dolt 2.3.5 in each `memory/<scope>/data/kuru`, with `DOLT_ROOT_PATH`, `HOME` and `TMPDIR` under the store's `home/`; each GC took 1 to 2 s. `du -sk data-<n>/memory` went from 259,908 to 222,420 KiB (1k) and from 1,450,004 to 1,425,852 KiB (5k). The total is 1,648,272 KiB (1.57 GiB, 1.69 GB), over the 1 GB budget, so the job is uncached. `DOLT_GC('--full')` gave 209,212 and 1,366,200 KiB (1.50 GiB). A zstd -3 tar of both stores is 883,117,641 bytes.
- [x] 2.2 @integration (agent) query the repository's Actions cache usage and entry listing before the change -> the total, the count and any `usage-scan-fixture-v1-` entry are recorded Observed 2026-10-01. At about 20:09Z: 61,838,208,314 bytes in 475 active entries; main held 212 entries (42.18 GB); there was no fixture entry. At 20:29Z, after main saved: 63,785,560,225 bytes in 490 entries. `usage-scan-fixture-v1-405e3250…9205` held 495,129,806 bytes (pre-GC, compressed) and `v0-rust-usage-scan-scaling-…` 586,056,039 bytes.

## 3. Static workflow checks

- [x] 3.1 @integration (agent) run `mise run //packages/kuru-delivery:test -- --test release_workflow` and `mise run lint:tooling` -> the job's shape test and the workflow validator pass Observed 2026-10-01, macOS arm64: both exit 0, including `usage_scan_scaling_is_a_required_job_that_ages_its_fixture_uncached`, which forbids cache restore and save, `steps.fixture`, `cache-hit` and `-- key` in the job. `//packages/kuru-memory:test -- usage_scan` passes 16 tests. `format:check`, `lint`, `//packages/kuru-memory:lint:windows`, `typecheck` and `docs:check` exit 0.
