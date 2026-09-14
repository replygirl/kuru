## 1. Stock shell bootstrap [critical]

- [x] 1.1 @regression (agent) run the isolated ToolHost stock PowerShell fixture on native Windows with fresh local state -> run `34804953102`, job `103854899023`, passed the exact `$PSHOME` Management and Utility module assertions, unqualified `Join-Path` and `Get-FileHash`, and all six Windows CLI stock-shell cases; cleaned-source connector job `103862209817` passed again without debugger setup
- [x] 1.2 @e2e (agent) retain the selected CDB capture for the first native corrected execution -> the first corrected connector run passed its always-run known-sleep native and managed stack control with confirmed cleanup and emitted checked receipt `027af5aa...`; the authoritative shell completed without failure capture, and the diagnostic was removed only after retaining that evidence

## 2. Preserved boundaries

- [x] 2.1 @equivalence (agent) exercise focused connector checks -> a focused Rust reproduction first rejected implicit capture from the expanded `concat!` string, then compiled and ran with explicit `command = command`; two CDB tests, connector typecheck, clippy with warnings denied, formatting, and diff checks passed, and source review confirmed the exact two imports precede unchanged user source while unrelated autoloading, generic commands, output/error/exit projection, cleanup, and deadlines remain unchanged
- [x] 2.2 @integration (agent) run the exact-head native matrix -> run 34807526553 attempt 2 passed the four-receipt Windows aggregate at 91.965106%, debugger-free connector bootstrap, application, memory/runtime, delivery/archive, Windows install/offline and both Unix native jobs; exact-head Intel recovery job 103872313453 and downstream gate 103875536751 then passed in attempt 3 after the isolated attempt-2 DNS failure

## 3. Documentation

- [x] 3.1 @integration (agent) run repository documentation and content checks -> `mise run docs:check` passed after public and contributor guidance documented the ToolHost-only `$PSHOME` module bootstrap while preserving the general PSModulePath and configured-command policy
