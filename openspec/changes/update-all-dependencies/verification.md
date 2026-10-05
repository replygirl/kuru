# Verification

## 1. Updated build inputs retain native acceptance [critical]

- [ ] 1.1 @runtime (agent) native CI provisions the verified engine and completes cold offline conversation, installation and previous-release updating on supported platforms -> successful final-head native acceptance receipts
- [ ] 1.2 @integration (agent) authoritative Linux build reproduces the refreshed Windows ARM64 recipe and each archive/payload inventory check succeeds -> observed output hashes match the committed manifest
- [ ] 1.3 @integration (agent) workspace behavioral coverage and terminal/memory/process fixtures execute with refreshed dependencies -> all partitions pass and combined line coverage remains at least 90%

## 2. Exact tooling and repository contracts

- [x] 2.1 @integration (agent) standalone cospec executes single-document clear, blocked, soft-blocked and missing-artifact flows without the old preload -> real cospec 0.8.3 contract: 2 tests passed; clear 0, hard-blocked 2, soft-blocked 3, missing-tasks 2; each JSON parsed as one document; local receipt tmp/dependency-update/cospec-contract.log
- [x] 2.2 @integration (agent) formatting, lint, typecheck, tooling, docs and Cospec gates run on final pins -> local static-checks-corrected.log exited 0 (host/Windows warnings-denied lint, typecheck, tooling, strict Cospec and managed checks); format-code.log and docs-check.log exited 0; public docs links and anchors passed
- [x] 2.3 @integration (agent) complete manifest/lock audit compares upstream stable releases and excluded mise selectors -> all 53 direct Cargo pins match primary non-yanked stable registry metadata; four owning lockfiles regenerated with existing mise 2026.9.18 and revision 1; five Communiqué archive provenance probes completed; min_version and all CI mise selectors unchanged
