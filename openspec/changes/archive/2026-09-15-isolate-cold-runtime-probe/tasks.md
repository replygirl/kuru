## 1. Checked cold probe copy

- [x] 1.1 Add a cold-only private probe copy that verifies the retained source and destination handles, complete pinned bytes, names and distinct identities before exact-version execution.
- [x] 1.2 Retain the candidate stage, probe area and installation lock through checked copy, owned probe completion and candidate activation with explicit cancellation-safe drop order, then close probe handles before staging cleanup and installation-lock release.

## 2. Regression coverage

- [x] 2.1 Add a native regression that retains the actual probe copy while the real candidate directory move succeeds and preserves the inside-candidate held-handle failure control.
- [x] 2.2 Add corrupt source/copy and canceled/failed probe controls proving no unverified payload activates and installation authority outlives all work.
- [x] 2.3 Run focused memory checks and the packaged Windows install/update cold-open acceptance, recording only observed evidence in the verification ledger.

The focused local checks and native Windows acceptance pass.
