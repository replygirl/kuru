## 1. Build inputs

- [x] 1.1 Consolidate the authoritative memory asset manifest and add native delivery preparation with local/offline input support; verify corrupt, missing, mismatched and concurrent prepared input behavior.
- [x] 1.2 Add a local-only TARGET-aware memory build script and mise preparation dependencies; verify a clean mise build and explicit Cargo failure for missing/corrupt target input.

## 2. Complete runtime

- [x] 2.1 Replace runtime downloads with embedded-byte extraction while retaining existing cache, license, version and cancellation safeguards; run actual-engine and adversarial provisioning tests.
- [x] 2.2 Route source install and native build/package workflows through target preparation; exercise packaged offline chat/reopen and native installer/update roundtrips with empty caches.
- [x] 2.3 Update AGENTS.md and owning installation/development/memory guides; review every install path for a single Kuru dependency contract and remove stale runtime-download instructions.

## 3. Integration and completion

- [x] 3.1 Run full mise check and native hosted packaged-runtime checks on the four existing targets; record actual coverage and tested SHA without weakening checks.
- [x] 3.2 Record acceptance evidence, strictly validate and archive this change before its final branch commit; preserve required PR checks before merge.
