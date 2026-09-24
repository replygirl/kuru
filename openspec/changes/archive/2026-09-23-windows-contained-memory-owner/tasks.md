## 1. Native process boundary

- [x] 1.1 Update `packages/kuru-platform/src/windows/process.rs` so `IndependentService` retries only exact access-denied plus confirmed current-Job membership, recreating mutable launch state and preserving every other flag, attribute, handle and error boundary.
- [x] 1.2 Replace the native denied-breakaway rejection fixture with permitting-breakaway and contained-fallback regressions, including starter exit and outer-Job closure.

## 2. Project memory lifecycle

- [x] 2.1 Update `packages/kuru-memory/src/service.rs` lifetime wording and Windows native fixture to prove contained owner usability, complete owner/Dolt termination, and ordinary committed-state recovery.
- [x] 2.2 Run platform and memory checks available locally, record native Windows behavior separately, and obtain independent source review.

## 3. Delivery

- [x] 3.1 Complete the verification ledger with observed evidence, archive the change, and commit the reviewed correction for hosted native CI.
