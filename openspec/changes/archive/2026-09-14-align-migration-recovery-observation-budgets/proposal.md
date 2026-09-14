## Why

The Windows memory shard twice reported a fixture's generic 10-second
observation deadline before a later child-channel closure; the log does not
establish why the underlying open had not reached the observed boundary. That
fixture limit is shorter than the existing 30-second startup and 30-second query
allowances, so the native fixture needs enough bounded time to report its actual
result without changing specified recovery or ownership behavior.

## What Changes

- `packages/kuru-memory/src/store/recovery_tests.rs` derives a private migration
  observation allowance from `startup_timeout + QUERY_TIMEOUT` for the
  open-to-AfterDdl and resume-to-completed-reopen boundaries.
- The parent child-readiness allowance additionally includes startup time for
  child initialization after spawn, the select prefers an available opening
  result over a closed pause semaphore, and cancellation errors name their
  observation boundary.
- The existing invalid-options early-error regression keeps its 10-second
  deadline, and all recovery, publication, process ownership, and cleanup
  assertions remain unchanged.

## Impact

Only the native migration recovery fixture changes. Default observation and
parent readiness allowances become 60 and 90 seconds respectively; these are
fixture-specific allowances derived from existing bounds, not new production
deadlines or worst-case promises for ordinary queries.
