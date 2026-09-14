## 1. Fail-closed Windows workspace coverage [critical]

- [x] 1.1 @integration (agent) exercise the delivery helper with complete, missing, duplicate, altered-source, altered-feature and corrupt-profile inputs -> seven focused helper cases pass; the task-private Cargo prototype visits all six targets with three run/three omit, rejects its forced failed outcome after Cargo continues all six, and the pinned reporter rejects a corrupt raw profile
- [x] 1.2 @runtime (agent) run all four package shards and aggregate their raw profiles at the exact pull-request head on CI Windows 2025 -> run 34807526553 attempt 2 produced four checked receipts for source 4d3c2d7/tree 6d4252b; their 64-entry ledgers select each of the eight-package full-inventory executables exactly once across four shards, all 1,202 profile hashes and sizes verify, and report job 103870775454 passed at 36,054/39,204 = 91.965106% line coverage

## 2. Native workflow behavior

- [x] 2.1 @runtime (agent) run source installation and cold offline installed-runtime acceptance beside the coverage shards on CI Windows 2025 -> attempt-2 job 103867249310 passed locked-input preparation, offline source install, missing/corrupt bundle rejection, cold offline install/update chat, and OS-only Kuru/Dolt PE import checks in 12m13s
- [x] 2.2 @runtime (agent) run the reusable native workflow on CI Ubuntu and macOS -> attempt-2 Ubuntu and macOS native jobs passed their unchanged monolithic behavior and 90% coverage gates; ARM build passed, and the Intel build passed as isolated attempt-3 recovery job 103872313453 after the attempt-2 pinned-input DNS failure
- [x] 2.3 @integration (agent) validate workflow dependencies, package assignment, pinned tool resolution and failure propagation -> two workflow-validator cases and actionlint pass; helper negatives reject missing/duplicate shard, package, source, feature, ledger and profile evidence before a report can succeed

## 3. Observed performance

- [x] 3.1 @runtime (agent) compare exact-head CI Windows 2025 shard and aggregate timings with the recorded 33–35 minute monolithic coverage runs -> connector, application, memory/runtime and delivery/archive completed in 6m29s, 13m07s, 14m46s and 20m14s; the report took 7m07s and the complete start-to-native-gate path was 27m33s versus 47m15s for PR18 run 34769483594 job 103756331991, an observed 19m42s/41.7% reduction across separate hosted runs with unchanged test concurrency, deadlines and scope
