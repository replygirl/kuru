## 1. Fail-closed Windows workspace coverage [critical]

- [x] 1.1 @integration (agent) exercise the delivery helper with complete, missing, duplicate, altered-source, altered-feature and corrupt-profile inputs -> seven focused helper cases pass; the task-private Cargo prototype visits all six targets with three run/three omit, rejects its forced failed outcome after Cargo continues all six, and the pinned reporter rejects a corrupt raw profile
- [ ] 1.2 @runtime (agent) run all four package shards and aggregate their raw profiles at the exact pull-request head on CI Windows 2025 -> initial run 34774890790 failed closed before helper execution because cmd.exe could not parse a PowerShell-only mise task invocation, created no receipts and therefore did not run the aggregate; the explicit pwsh task correction still requires exact-head hosted proof that Cargo invokes every declared standard test target, the ledgers select every target exactly once across the package partition, and the full Windows workspace remains at or above 90% line coverage

## 2. Native workflow behavior

- [ ] 2.1 @runtime (agent) run source installation and cold offline installed-runtime acceptance beside the coverage shards on CI Windows 2025 -> the exact tested source passes the unchanged install, bundled-engine and PE checks
- [ ] 2.2 @runtime (agent) run the reusable native workflow on CI Ubuntu and macOS -> the existing monolithic suites and per-OS 90% workspace gates remain required and pass
- [x] 2.3 @integration (agent) validate workflow dependencies, package assignment, pinned tool resolution and failure propagation -> two workflow-validator cases and actionlint pass; helper negatives reject missing/duplicate shard, package, source, feature, ledger and profile evidence before a report can succeed

## 3. Observed performance

- [ ] 3.1 @runtime (agent) compare exact-head CI Windows 2025 shard and aggregate timings with the recorded 33–35 minute monolithic coverage runs -> record the actual critical path without changing test concurrency, deadlines or scope
