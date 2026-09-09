## Context

The failing connection uses a write as its first statement after BEGIN DEFERRED, which SQLite treats as a write transaction. The error is SQLITE_BUSY (5), not BUSY_SNAPSHOT (517), and the Linux memory suite took 11.9 seconds while other gate tasks ran. The busy handler restores a five-second accumulated wait budget after initialization; it does not reserve the next writer turn.

## Decisions

Test successful contention in paired concurrent rounds, preserving two connections and eighty durable records. Join both outcomes before asserting errors, so an error cannot strand the other thread at a future barrier. Reopen before checking records. Separately hold a real write transaction through short, test-local busy deadlines and assert that append/state/clear operations fail without mutation, then succeed after release.

## Risks / Trade-offs

Do not raise the production timeout, retry arbitrary transactions, serialize all tests or change deferred transactions without evidence. The new pairing removes an unsupported fairness assumption while retaining actual concurrent writes. Existing cross-process and simultaneous-initialization tests remain intact. The user selected Dolt for versioned memory updates and dreaming history, to be implemented in a separate PR after the initial release and before production adoption. That transition is recorded in AGENTS.md; this correction preserves the current SQLite implementation and release scope.

## Integration contract

SQLite starts a write transaction when the first statement after BEGIN DEFERRED writes. sqlite3_busy_timeout bounds accumulated waiting, not writer fairness. Use the real bundled SQLite library and rusqlite error codes, with external BEGIN IMMEDIATE holding a genuine competing lock. References: https://www.sqlite.org/lang_transaction.html and https://www.sqlite.org/c3ref/busy_timeout.html.
