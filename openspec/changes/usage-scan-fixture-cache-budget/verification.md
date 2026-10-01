# Verification

## 1. The job runs on its real runner under the cache budget [critical]

- [ ] 1.1 @runtime (agent) GitHub-hosted ubuntu-latest, this PR's CI run, job `usage-scan-scaling` -> the job passes with the chosen fixture handling, and its wall clock is recorded against the partition critical path (14 to 17 min)

## 2. The decision rests on measured sizes

- [ ] 2.1 @integration (agent) age fresh 1k and 5k stores with the merged driver, run `CALL DOLT_GC()` on each, and measure `du` before and after -> the post-GC total is compared with the 1 GB budget and the branch follows the rule
- [ ] 2.2 @integration (agent) query the repository's Actions cache usage and entry listing before the change -> the total, the count and any `usage-scan-fixture-v1-` entry are recorded

## 3. Static workflow checks

- [ ] 3.1 @integration (agent) run `mise run //packages/kuru-delivery:test -- --test release_workflow` and `mise run lint:tooling` -> the job's shape test and the workflow validator pass
