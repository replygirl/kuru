## 1. Stock shell bootstrap [critical]

- [ ] 1.1 @regression (agent) run the isolated ToolHost stock PowerShell fixture on native Windows with fresh local state -> the prior source is evidenced stopping before its first `Join-Path`, while the corrected source sees the exact `$PSHOME` Management and Utility modules loaded at user entry and completes unqualified `Join-Path` and `Get-FileHash` under the unchanged shell deadline
- [ ] 1.2 @e2e (agent) retain the selected CDB capture for the first native corrected execution -> known-sleep capture succeeds, the authoritative shell completes without requiring a failure capture, cleanup remains confirmed, and the connector shard emits its checked receipt

## 2. Preserved boundaries

- [x] 2.1 @equivalence (agent) exercise focused connector checks -> a focused Rust reproduction first rejected implicit capture from the expanded `concat!` string, then compiled and ran with explicit `command = command`; two CDB tests, connector typecheck, clippy with warnings denied, formatting, and diff checks passed, and source review confirmed the exact two imports precede unchanged user source while unrelated autoloading, generic commands, output/error/exit projection, cleanup, and deadlines remain unchanged
- [ ] 2.2 @integration (agent) run the exact-head native matrix -> all four Windows shard receipts aggregate into the full workspace report at or above 90% line coverage, Windows install/offline checks pass, and Unix native jobs retain their existing behavior

## 3. Documentation

- [x] 3.1 @integration (agent) run repository documentation and content checks -> `mise run docs:check` passed after public and contributor guidance documented the ToolHost-only `$PSHOME` module bootstrap while preserving the general PSModulePath and configured-command policy
