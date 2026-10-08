# Verification

## 1. Confirmed results release operation ownership [critical]

- [x] 1.1 @regression (agent) synchronous result-wake observation of native hooks and shell holds -> October 8 macOS: the unfixed hook observed (1 lease, 1 worker), Unix shell returned before hold release, and all four unfixed MCP phases failed; fixed native regressions passed without timing assertions
- [x] 1.2 @integration (agent) real hook success and malformed response plus overlapping budget/cap test -> both responses wake with zero active leases and workers; original shared-budget/cap test and final 333-test connector suite passed
- [x] 1.3 @integration (agent) native Unix shell clean completion and prelaunch failure -> both public result futures release their hold and registry entry before return; synchronous wake also preserves the unrelated reservation
- [x] 1.4 @integration (agent) real MCP startup, send, request, close and clean failures with per-operation holds -> all four successful phases plus remote-error and malformed-record paths passed; original native fixture saw all four phase-specific hold assertions fail
- [x] 1.5 @regression (agent) native session selection refusal followed by immediate lock reacquisition inside the result wake -> original real-Dolt test failed with checked local ownership/draining; fixed native acquisition passed, along with all six driver tests. Used isolated prefetch/supervisor preparation and process-local 4096 descriptor limit; preserved the pre-existing invalid default cache
- [~] 1.6 @integration (agent) native Windows shell and portable hook/MCP regressions -> defer: this local host is macOS; Windows-target connector Clippy passed, and actual Windows x64/arm tests remain mandatory PR CI gates before merge

## 2. Cancellation and uncertain cleanup [critical]

- [x] 2.1 @integration (agent) native caller-loss and unconfirmed-cleanup fixtures -> final connector suite passed retained MCP cleanup after parent-runtime loss and Unix retained-owner/admission/drop fixtures; six native driver tests passed existing loss/recovery and held-work exclusion cases
- [x] 2.2 @equivalence (agent) review worker outcome publication sites -> reviewed connector auth/hooks/shell/MCP, memory backup/provision/creation/migration/stage/driver, platform browser/snapshot and runtime actor replies. Backup/snapshot workers join before return; provisioning and store workers transfer explicit retained ownership; auth leases and actor permits end in the completed inner future. No unrelated ownership handoff changed

## 3. Repository checks

- [x] 3.1 @integration (agent) affected package tests and static/documentation checks -> final connector suite 333/333, native driver selection 6/6, connector host and Windows-target Clippy, memory Clippy, Rust formatting and public docs build/content/link/anchor checks passed locally. Whole-workspace checks run through normal push hooks and CI; no local full coverage rerun
- [~] 3.2 @integration (agent) PR and resulting-main native CI -> defer: runs follow this local archive and are mandatory before delivery; actual CI facts are maintained in the private working document
