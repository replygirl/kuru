## 1. Native byte channels

- [x] 1.1 Add checked Unix listener/connect primitives with private parent and socket identity, and verify real successive-client and rejected-endpoint fixtures on macOS.
- [x] 1.2 Add Windows successive-client named-pipe listener while preserving the existing child rendezvous, and verify Windows-target test compilation plus a native timeout-then-connect fixture.

## 2. Contract and review readiness

- [x] 2.1 Record native and cross-target evidence in the verification ledger, explicitly leaving Windows native execution pending CI, and verify strict Cospec validation.
- [x] 2.2 Run package format, lint and native IPC tests; submit the bounded diff for independent review, then verify the reviewer findings are resolved before archive.
