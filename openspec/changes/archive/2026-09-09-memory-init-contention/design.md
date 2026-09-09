## Context

Initialization commits an IMMEDIATE schema transaction, validates integrity and required columns, then enables WAL. That last pragma needs a stronger database lock. SQLite documents that a busy callback may be bypassed to prevent a lock-upgrade deadlock, so its configured busy timeout does not cover every transient SQLITE_BUSY response.

## Decisions

Keep the existing schema transaction and validation order. Retry only the WAL pragma on SQLITE_BUSY, with one fixed wall-clock budget and no live statement or transaction held between attempts. Other SQLite errors and unexpected journal modes remain errors. Restore the normal busy timeout before handing out the connection. Do not add process-local serialization, sidecar lock files, test retries or a generic database-operation retry wrapper.

## Risks / Trade-offs

Persistent contention must still fail within the documented bounded wait. A regression will hold a real competing SQLite transaction, demonstrate the old failure, release that lock, then require successful WAL initialization and preserved data. Independent child processes and the existing simultaneous-open test retain cross-process coverage. Corrupt, foreign and future databases must remain rejected without journal conversion.

## Integration contract

SQLite's busy-handler contract explicitly permits immediate SQLITE_BUSY during lock promotion; journal_mode cannot be changed inside a transaction. The retry uses the rusqlite error code and actual pragma result, not string matching. References: https://www.sqlite.org/c3ref/busy_handler.html and https://www.sqlite.org/pragma.html#pragma_journal_mode.
