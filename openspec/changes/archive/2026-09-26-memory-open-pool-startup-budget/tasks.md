## 1. Opening-phase pool budget

- [x] 1.1 Record the owned start's startup deadline on the server, select max(ordinary, remaining deadline) for `Server::pool` acquisition and identity verification during the opening phase, and end the phase in `MemoryStore::open_inner` before `Ready`; verify by the regression test in 1.4.
- [x] 1.2 Make authored identity rejections terminal for opening-phase attempts and verify with a real-server identity-mismatch test returning typed `Protocol`, not `PoolTimedOut`.
- [x] 1.3 Bound the Windows supervisor pipe accept by the remaining startup deadline; verify it compiles in native Windows CI (not available locally).
- [x] 1.4 Add a store-level regression test delaying the migrated staged reopen's main-pool authentication: inside the budget succeeds, beyond it fails with typed `PoolTimedOut`, and post-open pools keep the ordinary window; verify it fails before the fix and passes after.

## 2. Verification

- [x] 2.1 Run focused kuru-memory server/store open tests and the two originally failing kuru-runtime tests; record results.
- [x] 2.2 Run fmt and kuru-memory clippy `-D warnings`; record results.

## Observed evidence

Recorded in `verification.md` (macOS aarch64 host, 2026-09-26). Task 1.3 is implemented but its Windows compile/behavior is unverified locally; native Windows CI after push is the check. Not run: workspace coverage gate, the full kuru-runtime suite, and any Windows/Linux native job.
